//! Celestia client implementation.
//!
//! Uses celestia-client (Lumina) for direct gRPC blob submission,
//! and celestia-rpc for header subscription (finality tracking).

use std::sync::Arc;
use std::time::Duration;

use alloy_primitives::B256;
use celestia_client::tx::TxConfig;
use celestia_client::{Client as LuminaClient, ClientBuilder};
use celestia_rpc::{Client as RpcClient, HeaderClient};
use celestia_types::nmt::Namespace;
use celestia_types::{AppVersion, Blob};
use futures::Stream;
use sequencer_types::{Block, CelestiaConfig};
use tendermint::merkle;
use tokio::sync::{broadcast, mpsc, RwLock};

use crate::retry::{with_retry, DEFAULT_MAX_RETRIES, DEFAULT_RECONNECT_DELAY_MS, DEFAULT_RETRY_DELAY_MS};
use crate::{
    BlobSubmission, CelestiaError, DataAvailability, FinalityConfirmation, FinalityTracker, Result,
};

/// Celestia DA client.
///
/// Uses two clients:
/// - `lumina_client`: For blob submission via gRPC (PayForBlobs tx)
/// - `rpc_client`: For header subscription (finality tracking)
pub struct CelestiaClient {
    config: CelestiaConfig,
    /// Lumina client for blob submission (gRPC).
    lumina_client: LuminaClient,
    /// RPC client for header subscription.
    rpc_client: RpcClient,
    namespace: Namespace,
    finality_tracker: Arc<RwLock<FinalityTracker>>,
    /// Sender for finality confirmations (used by run_finality_tracker)
    finality_tx: mpsc::Sender<FinalityConfirmation>,
    /// Broadcast sender for multiple subscribers
    finality_broadcast: broadcast::Sender<FinalityConfirmation>,
}

impl CelestiaClient {
    /// Create a new Celestia client.
    ///
    /// # Errors
    ///
    /// Returns an error if the client cannot be initialized.
    pub async fn new(
        config: CelestiaConfig,
        finality_tx: mpsc::Sender<FinalityConfirmation>,
    ) -> Result<Self> {
        Self::validate_config(&config)?;

        let namespace = Self::parse_namespace(&config.namespace)?;
        let private_key_hex = Self::load_private_key(&config.celestia_key_path)?;
        let grpc_url = Self::build_grpc_url(&config.core_grpc_addr, config.core_grpc_tls_enabled);

        let lumina_client = Self::build_lumina_client(&config, &grpc_url, &private_key_hex).await?;
        let rpc_client = Self::build_rpc_client(&config).await?;

        let finality_tracker = Arc::new(RwLock::new(FinalityTracker::new()));
        let (finality_broadcast, _) = broadcast::channel(100);

        tracing::info!(
            bridge_addr = %config.bridge_addr,
            core_grpc_addr = %grpc_url,
            namespace = %config.namespace,
            "Connected to Celestia node"
        );

        Ok(Self {
            config,
            lumina_client,
            rpc_client,
            namespace,
            finality_tracker,
            finality_tx,
            finality_broadcast,
        })
    }

    /// Validate configuration fields.
    fn validate_config(config: &CelestiaConfig) -> Result<()> {
        if config.bridge_addr.is_empty() {
            return Err(CelestiaError::InvalidConfig("bridge_addr cannot be empty".into()));
        }
        if config.core_grpc_addr.is_empty() {
            return Err(CelestiaError::InvalidConfig("core_grpc_addr cannot be empty".into()));
        }
        Ok(())
    }

    /// Parse namespace from hex-encoded config string.
    fn parse_namespace(namespace_hex: &str) -> Result<Namespace> {
        let bytes = hex::decode(namespace_hex)
            .map_err(|e| CelestiaError::InvalidConfig(format!("invalid namespace hex: {e}")))?;
        Namespace::new_v0(&bytes)
            .map_err(|e| CelestiaError::InvalidConfig(format!("invalid namespace: {e}")))
    }

    /// Load private key from file path.
    fn load_private_key(path: &std::path::Path) -> Result<String> {
        if !path.exists() {
            return Err(CelestiaError::InvalidConfig(format!(
                "celestia key not found at {}",
                path.display()
            )));
        }
        let key = std::fs::read_to_string(path)
            .map_err(|e| CelestiaError::InvalidConfig(format!("failed to read celestia key: {e}")))?;
        Ok(key.trim().to_string())
    }

    /// Build the Lumina gRPC client.
    async fn build_lumina_client(
        config: &CelestiaConfig,
        grpc_url: &str,
        private_key_hex: &str,
    ) -> Result<LuminaClient> {
        let mut builder = ClientBuilder::new()
            .rpc_url(&config.bridge_addr)
            .grpc_url(grpc_url)
            .private_key_hex(private_key_hex);

        if !config.bridge_auth_token.is_empty() {
            builder = builder.rpc_auth_token(&config.bridge_auth_token);
        }
        if !config.core_grpc_auth_token.is_empty() {
            builder = builder.grpc_metadata("authorization", &config.core_grpc_auth_token);
        }

        let client = builder.build().await
            .map_err(|e| CelestiaError::ConnectionFailed(format!("lumina client: {e}")))?;

        let address_str = client.address().map_or_else(|_| "unknown".into(), |a| a.to_string());
        tracing::info!(bridge_addr = %config.bridge_addr, core_grpc_addr = %grpc_url, address = %address_str, "Lumina client initialized");

        Ok(client)
    }

    /// Build the RPC client for header subscription.
    async fn build_rpc_client(config: &CelestiaConfig) -> Result<RpcClient> {
        let auth = if config.bridge_auth_token.is_empty() { None } else { Some(config.bridge_auth_token.as_str()) };
        RpcClient::new(&config.bridge_addr, auth, None, None).await
            .map_err(|e| CelestiaError::ConnectionFailed(format!("rpc client: {e}")))
    }

    /// Build gRPC URL with proper scheme based on TLS setting.
    fn build_grpc_url(addr: &str, tls_enabled: bool) -> String {
        // If address already has a scheme, use it as-is
        if addr.starts_with("http://") || addr.starts_with("https://") {
            return addr.to_string();
        }

        // Otherwise, add the appropriate scheme
        if tls_enabled {
            format!("https://{addr}")
        } else {
            format!("http://{addr}")
        }
    }

    /// Encode a block as blob data.
    fn encode_block(block: &Block) -> Result<Vec<u8>> {
        bincode::serialize(block).map_err(|e| CelestiaError::EncodeFailed(e.to_string()))
    }

    /// Decode blob data back to a block.
    fn decode_block(data: &[u8]) -> Result<Block> {
        bincode::deserialize(data).map_err(|e| CelestiaError::DecodeFailed(e.to_string()))
    }

    /// Convert commitment hash to bytes.
    fn commitment_to_bytes(hash: merkle::Hash) -> Vec<u8> {
        hash.to_vec()
    }

    /// Submit a blob with retry logic using Lumina client (gRPC).
    async fn submit_with_retry(&self, blob: Blob) -> Result<(u64, Vec<u8>)> {
        let commitment = Self::commitment_to_bytes(blob.commitment.into());
        let gas_price = self.config.gas_price;
        let client = &self.lumina_client;

        let height = with_retry(
            "Blob submission",
            DEFAULT_MAX_RETRIES,
            DEFAULT_RETRY_DELAY_MS,
            || async {
                let tx_config = TxConfig::default().with_gas_price(gas_price);
                let tx_info = client
                    .blob()
                    .submit(std::slice::from_ref(&blob), tx_config)
                    .await?;

                tracing::debug!(
                    height = tx_info.height,
                    tx_hash = %hex::encode(tx_info.hash),
                    "Blob submitted via gRPC"
                );

                Ok::<_, celestia_client::Error>(tx_info.height)
            },
        )
        .await?;

        Ok((height, commitment))
    }

    /// Run the finality tracking loop with automatic reconnection.
    ///
    /// This subscribes to Celestia headers and emits finality confirmations
    /// when blocks become finalized. If the subscription drops, it will
    /// automatically reconnect.
    ///
    /// # Errors
    ///
    /// Returns an error if the subscription fails permanently.
    pub async fn run_finality_tracker(&self) -> Result<()> {
        tracing::info!("Starting Celestia finality tracker");

        loop {
            if let Err(e) = self.run_header_subscription().await {
                tracing::error!(error = %e, "Header subscription failed");
            }

            tracing::info!(
                delay_ms = DEFAULT_RECONNECT_DELAY_MS,
                "Reconnecting to header subscription..."
            );
            tokio::time::sleep(Duration::from_millis(DEFAULT_RECONNECT_DELAY_MS)).await;
        }
    }

    /// Run a single header subscription session.
    async fn run_header_subscription(&self) -> Result<()> {
        use futures::StreamExt;

        let mut header_sub = self.rpc_client.header_subscribe();

        while let Some(result) = header_sub.next().await {
            match result {
                Ok(header) => self.process_header(&header).await?,
                Err(e) => {
                    tracing::error!(error = %e, "Header subscription error");
                    return Err(CelestiaError::SubscriptionFailed(e.to_string()));
                }
            }
        }

        tracing::warn!("Header subscription ended unexpectedly");
        Err(CelestiaError::SubscriptionFailed(
            "subscription stream ended".to_string(),
        ))
    }

    /// Process a single Celestia header.
    async fn process_header(&self, header: &celestia_types::ExtendedHeader) -> Result<()> {
        let celestia_height = header.height();
        let header_hash = Self::header_to_b256(header);

        tracing::debug!(
            celestia_height,
            %header_hash,
            "Received Celestia header"
        );

        let confirmations = {
            let mut tracker = self.finality_tracker.write().await;
            tracker.on_celestia_finality(celestia_height, header_hash)
        };

        self.broadcast_confirmations(confirmations).await;
        Ok(())
    }

    /// Broadcast finality confirmations to all subscribers.
    async fn broadcast_confirmations(&self, confirmations: Vec<FinalityConfirmation>) {
        for confirmation in confirmations {
            tracing::info!(
                block_height = confirmation.block_height,
                celestia_height = confirmation.celestia_height,
                latency_ms = confirmation.latency_ms(),
                "Block finalized on Celestia"
            );

            // Send to main channel
            if self.finality_tx.send(confirmation.clone()).await.is_err() {
                tracing::warn!("Finality receiver dropped");
            }

            // Broadcast to all subscribers (ignore errors - no receivers is OK)
            let _ = self.finality_broadcast.send(confirmation);
        }
    }

    /// Convert a Celestia header hash to B256.
    fn header_to_b256(header: &celestia_types::ExtendedHeader) -> B256 {
        let hash_bytes: [u8; 32] = header
            .hash()
            .as_bytes()
            .try_into()
            .expect("Celestia header hash is always 32 bytes");
        B256::from(hash_bytes)
    }

    /// Get a block from Celestia by height.
    ///
    /// This retrieves all blobs in our namespace at the given Celestia height
    /// and returns the first successfully decoded block.
    ///
    /// # Errors
    ///
    /// Returns an error if the retrieval fails.
    pub async fn get_block(&self, celestia_height: u64) -> Result<Option<Block>> {
        let blocks = self.get_blocks(celestia_height).await?;
        Ok(blocks.into_iter().next())
    }

    /// Get all blocks from Celestia at a given height.
    ///
    /// This retrieves all blobs in our namespace at the given Celestia height
    /// and attempts to decode each as a block. Invalid blobs are skipped.
    ///
    /// # Errors
    ///
    /// Returns an error if the retrieval fails.
    pub async fn get_blocks(&self, celestia_height: u64) -> Result<Vec<Block>> {
        tracing::debug!(celestia_height, "Fetching blocks from Celestia");

        let blob_list = self.fetch_blobs(celestia_height).await?;
        let Some(blobs) = blob_list else {
            tracing::debug!(celestia_height, "No blobs found at height");
            return Ok(vec![]);
        };

        let mut blocks = Vec::new();
        for blob in &blobs {
            let Some(block) = Self::try_decode_blob(blob, celestia_height) else { continue };
            blocks.push(block);
        }
        Ok(blocks)
    }

    /// Fetch blobs from Celestia at a given height.
    async fn fetch_blobs(&self, celestia_height: u64) -> Result<Option<Vec<Blob>>> {
        use celestia_rpc::BlobClient;
        self.rpc_client
            .blob_get_all(celestia_height, &[self.namespace])
            .await
            .map_err(|e| CelestiaError::Internal(format!("failed to fetch blobs: {e}")))
    }

    /// Try to decode a blob as a block, logging failures.
    fn try_decode_blob(blob: &Blob, celestia_height: u64) -> Option<Block> {
        match Self::decode_block(&blob.data) {
            Ok(block) => {
                tracing::debug!(celestia_height, block_height = block.height(), "Retrieved block");
                Some(block)
            }
            Err(e) => {
                tracing::trace!(celestia_height, error = %e, "Skipping blob that failed to decode");
                None
            }
        }
    }

    /// Get all blocks in a range of Celestia heights.
    ///
    /// # Errors
    ///
    /// Returns an error if any retrieval fails.
    pub async fn get_blocks_in_range(&self, start: u64, end: u64) -> Result<Vec<(u64, Block)>> {
        if start > end {
            return Ok(vec![]);
        }

        tracing::info!(start_height = start, end_height = end, "Fetching blocks from Celestia range");

        let mut all_blocks = Vec::new();
        for height in start..=end {
            let blocks = self.get_blocks(height).await?;
            all_blocks.extend(blocks.into_iter().map(|b| (height, b)));
        }

        tracing::info!(start_height = start, end_height = end, block_count = all_blocks.len(), "Retrieved blocks");
        Ok(all_blocks)
    }

    /// Verify that a blob is included at the given Celestia height.
    ///
    /// # Errors
    ///
    /// Returns an error if verification fails.
    pub async fn verify_inclusion(&self, celestia_height: u64, commitment: &[u8]) -> Result<bool> {
        tracing::debug!(celestia_height, commitment_len = commitment.len(), "Verifying blob inclusion");

        let Some(blobs) = self.fetch_blobs(celestia_height).await? else {
            tracing::debug!(celestia_height, "No blobs found at height");
            return Ok(false);
        };

        let found = blobs.iter().any(|b| Self::commitment_to_bytes(b.commitment.into()) == commitment);
        tracing::debug!(celestia_height, found, "Blob inclusion check complete");
        Ok(found)
    }

    /// Get the Celestia address for this client.
    ///
    /// # Errors
    ///
    /// Returns an error if the address cannot be determined.
    pub fn address(&self) -> Result<String> {
        self.lumina_client
            .address()
            .map(|a| a.to_string())
            .map_err(|e| CelestiaError::Internal(format!("failed to get address: {e}")))
    }

    /// Get the namespace for this chain.
    #[must_use]
    pub fn namespace(&self) -> &str {
        &self.config.namespace
    }

    /// Get the current finality tracker state.
    pub async fn pending_finality_count(&self) -> usize {
        self.finality_tracker.read().await.pending_count()
    }

    /// Get the last known finalized Celestia height.
    pub async fn finalized_height(&self) -> u64 {
        self.finality_tracker.read().await.finalized_height()
    }
}

/// Block info for submission tracking.
struct BlockInfo {
    hash: B256,
    height: u64,
}

impl CelestiaClient {
    /// Submit multiple blocks as a batch to Celestia.
    ///
    /// # Errors
    ///
    /// Returns an error if submission fails.
    pub async fn submit_blocks(&self, blocks: &[Block]) -> Result<Vec<BlobSubmission>> {
        if blocks.is_empty() {
            return Ok(vec![]);
        }

        tracing::info!(
            block_count = blocks.len(),
            first_height = blocks.first().map(Block::height),
            last_height = blocks.last().map(Block::height),
            "Submitting block batch to Celestia"
        );

        let (blobs, block_infos) = self.create_blobs_for_blocks(blocks)?;
        let celestia_height = self.submit_blobs_with_retry(&blobs).await?;
        Ok(Self::build_submissions(blobs, block_infos, celestia_height))
    }

    /// Create blobs for a batch of blocks.
    fn create_blobs_for_blocks(&self, blocks: &[Block]) -> Result<(Vec<Blob>, Vec<BlockInfo>)> {
        let mut blobs = Vec::with_capacity(blocks.len());
        let mut infos = Vec::with_capacity(blocks.len());

        for block in blocks {
            let blob = self.create_blob_for_block(block)?;
            infos.push(BlockInfo { hash: block.block_hash, height: block.height() });
            blobs.push(blob);
        }
        Ok((blobs, infos))
    }

    /// Create a single blob for a block.
    fn create_blob_for_block(&self, block: &Block) -> Result<Blob> {
        let address = self.lumina_client.address()
            .map_err(|e| CelestiaError::Internal(format!("failed to get address: {e}")))?;
        let data = Self::encode_block(block)?;
        Blob::new(self.namespace, data, Some(address), AppVersion::latest())
            .map_err(|e| CelestiaError::EncodeFailed(format!("failed to create blob: {e}")))
    }

    /// Submit blobs with retry logic.
    async fn submit_blobs_with_retry(&self, blobs: &[Blob]) -> Result<u64> {
        let gas_price = self.config.gas_price;
        let client = &self.lumina_client;
        let block_count = blobs.len();

        with_retry("Batch submission", DEFAULT_MAX_RETRIES, DEFAULT_RETRY_DELAY_MS, || async {
            let tx_config = TxConfig::default().with_gas_price(gas_price);
            let tx_info = client.blob().submit(blobs, tx_config).await?;
            tracing::info!(height = tx_info.height, tx_hash = %hex::encode(tx_info.hash), block_count, "Batch submitted");
            Ok::<_, celestia_client::Error>(tx_info.height)
        }).await
    }

    /// Build submission results from blobs and block infos.
    fn build_submissions(blobs: Vec<Blob>, infos: Vec<BlockInfo>, celestia_height: u64) -> Vec<BlobSubmission> {
        infos.into_iter().zip(blobs).map(|(info, blob)| BlobSubmission {
            block_hash: info.hash,
            block_height: info.height,
            celestia_height,
            commitment: Self::commitment_to_bytes(blob.commitment.into()),
        }).collect()
    }
}

#[async_trait::async_trait]
impl DataAvailability for CelestiaClient {
    async fn submit_block(&self, block: &Block) -> Result<BlobSubmission> {
        let block_hash = block.block_hash;
        let block_height = block.height();

        tracing::debug!(
            block_height,
            %block_hash,
            "Submitting block to Celestia via gRPC"
        );

        let blob_data = Self::encode_block(block)?;

        // Create blob with namespace and our address as fee payer
        let address = self.lumina_client.address().map_err(|e| {
            CelestiaError::Internal(format!("failed to get address: {e}"))
        })?;

        let blob = Blob::new(self.namespace, blob_data, Some(address), AppVersion::latest())
            .map_err(|e| CelestiaError::EncodeFailed(format!("failed to create blob: {e}")))?;

        // Submit blob with retry logic
        let (celestia_height, commitment) = self.submit_with_retry(blob).await?;

        let submission = BlobSubmission {
            block_hash,
            block_height,
            celestia_height,
            commitment,
        };

        tracing::info!(
            block_height,
            celestia_height,
            commitment_len = submission.commitment.len(),
            "Block submitted to Celestia via gRPC"
        );

        Ok(submission)
    }

    fn finality_stream(&self) -> Box<dyn Stream<Item = FinalityConfirmation> + Send + Unpin> {
        use tokio_stream::StreamExt as _;

        // Subscribe to the broadcast channel
        let rx = self.finality_broadcast.subscribe();
        Box::new(
            tokio_stream::wrappers::BroadcastStream::new(rx).filter_map(std::result::Result::ok),
        )
    }

    fn track_finality(&self, _block_hash: B256, submission: BlobSubmission) {
        let tracker = self.finality_tracker.clone();

        tokio::spawn(async move {
            tracker.write().await.track(&submission);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_block_encoding_roundtrip() {
        use sequencer_types::BlockHeader;

        let header = BlockHeader {
            height: 42,
            timestamp: 1234567890,
            parent_hash: B256::ZERO,
            proposer: alloy_primitives::Address::ZERO,
        };

        let block = Block::test_block(header, vec![]);

        let encoded = CelestiaClient::encode_block(&block).unwrap();
        let decoded = CelestiaClient::decode_block(&encoded).unwrap();

        assert_eq!(block.height(), decoded.height());
        assert_eq!(block.block_hash, decoded.block_hash);
    }
}
