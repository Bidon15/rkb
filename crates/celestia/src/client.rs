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

use crate::{
    BlobSubmission, CelestiaError, DataAvailability, FinalityConfirmation, FinalityTracker, Result,
};

/// Default retry configuration.
const DEFAULT_MAX_RETRIES: u32 = 3;
const DEFAULT_RETRY_DELAY_MS: u64 = 1000;
const DEFAULT_RECONNECT_DELAY_MS: u64 = 5000;

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
        if config.rpc_endpoint.is_empty() {
            return Err(CelestiaError::InvalidConfig(
                "rpc_endpoint cannot be empty".to_string(),
            ));
        }

        if config.grpc_endpoint.is_empty() {
            return Err(CelestiaError::InvalidConfig(
                "grpc_endpoint cannot be empty".to_string(),
            ));
        }

        // Parse namespace from config (hex-encoded bytes)
        let namespace_bytes = hex::decode(&config.namespace).map_err(|e| {
            CelestiaError::InvalidConfig(format!("invalid namespace hex: {e}"))
        })?;

        let namespace = Namespace::new_v0(&namespace_bytes).map_err(|e| {
            CelestiaError::InvalidConfig(format!("invalid namespace: {e}"))
        })?;

        // Load Celestia signing key (hex-encoded secp256k1 private key)
        let private_key_hex = if config.celestia_key_path.exists() {
            let key = std::fs::read_to_string(&config.celestia_key_path).map_err(|e| {
                CelestiaError::InvalidConfig(format!("failed to read celestia key: {e}"))
            })?;
            key.trim().to_string()
        } else {
            return Err(CelestiaError::InvalidConfig(format!(
                "celestia key not found at {}",
                config.celestia_key_path.display()
            )));
        };

        // Create Lumina client for blob submission (gRPC)
        let lumina_client = ClientBuilder::new()
            .rpc_url(&config.rpc_endpoint)
            .grpc_url(&config.grpc_endpoint)
            .private_key_hex(&private_key_hex)
            .build()
            .await
            .map_err(|e| CelestiaError::ConnectionFailed(format!("lumina client: {e}")))?;

        let address_str = lumina_client
            .address()
            .map(|a| a.to_string())
            .unwrap_or_else(|_| "unknown".to_string());

        tracing::info!(
            rpc_endpoint = %config.rpc_endpoint,
            grpc_endpoint = %config.grpc_endpoint,
            address = %address_str,
            "Lumina client initialized for blob submission"
        );

        // Create RPC client for header subscription (finality tracking)
        // Note: This client doesn't need auth token for read-only operations
        let rpc_client = RpcClient::new(&config.rpc_endpoint, None, None, None)
            .await
            .map_err(|e| CelestiaError::ConnectionFailed(format!("rpc client: {e}")))?;

        let finality_tracker = Arc::new(RwLock::new(FinalityTracker::new()));
        let (finality_broadcast, _) = broadcast::channel(100);

        tracing::info!(
            rpc_endpoint = %config.rpc_endpoint,
            grpc_endpoint = %config.grpc_endpoint,
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
        let mut attempts = 0;
        let max_retries = DEFAULT_MAX_RETRIES;
        let commitment = Self::commitment_to_bytes(blob.commitment.into());

        loop {
            attempts += 1;

            // Configure transaction with gas price from config
            let tx_config = TxConfig::default().with_gas_price(self.config.gas_price);

            match self.lumina_client.blob().submit(&[blob.clone()], tx_config).await {
                Ok(tx_info) => {
                    tracing::debug!(
                        height = tx_info.height,
                        tx_hash = %hex::encode(&tx_info.hash),
                        "Blob submitted via gRPC"
                    );
                    return Ok((tx_info.height, commitment));
                }
                Err(e) => {
                    if attempts >= max_retries {
                        return Err(CelestiaError::SubmissionFailed(format!(
                            "failed after {attempts} attempts: {e}"
                        )));
                    }

                    tracing::warn!(
                        attempt = attempts,
                        max_retries,
                        error = %e,
                        "Blob submission failed, retrying..."
                    );

                    tokio::time::sleep(Duration::from_millis(
                        DEFAULT_RETRY_DELAY_MS * u64::from(attempts),
                    ))
                    .await;
                }
            }
        }
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

        loop {
            match header_sub.next().await {
                Some(Ok(header)) => {
                    let celestia_height = header.height();
                    let celestia_hash = header.hash();

                    // Convert Celestia hash to B256
                    let hash_bytes: [u8; 32] = celestia_hash
                        .as_bytes()
                        .try_into()
                        .unwrap_or([0u8; 32]);
                    let header_hash = B256::from(hash_bytes);

                    tracing::debug!(
                        celestia_height,
                        %header_hash,
                        "Received Celestia header"
                    );

                    // Check for finalized blocks
                    let confirmations = {
                        let mut tracker = self.finality_tracker.write().await;
                        tracker.on_celestia_finality(celestia_height, header_hash)
                    };

                    // Send finality confirmations
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
                Some(Err(e)) => {
                    tracing::error!(error = %e, "Header subscription error");
                    return Err(CelestiaError::SubscriptionFailed(e.to_string()));
                }
                None => {
                    tracing::warn!("Header subscription ended unexpectedly");
                    return Err(CelestiaError::SubscriptionFailed(
                        "subscription stream ended".to_string(),
                    ));
                }
            }
        }
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

        // Use RPC client to get blobs (BlobClient trait)
        use celestia_rpc::BlobClient;
        let blobs = self
            .rpc_client
            .blob_get_all(celestia_height, &[self.namespace])
            .await
            .map_err(|e| CelestiaError::Internal(format!("failed to fetch blobs: {e}")))?;

        let Some(blob_list) = blobs else {
            tracing::debug!(celestia_height, "No blobs found at height");
            return Ok(vec![]);
        };

        let mut blocks = Vec::with_capacity(blob_list.len());

        for blob in &blob_list {
            match Self::decode_block(&blob.data) {
                Ok(block) => {
                    tracing::debug!(
                        celestia_height,
                        block_height = block.height(),
                        "Retrieved block from Celestia"
                    );
                    blocks.push(block);
                }
                Err(e) => {
                    // Skip blobs that don't decode as valid blocks
                    // (could be from different applications using same namespace)
                    tracing::trace!(
                        celestia_height,
                        error = %e,
                        "Skipping blob that failed to decode"
                    );
                }
            }
        }

        Ok(blocks)
    }

    /// Get all blocks in a range of Celestia heights.
    ///
    /// Useful for syncing a node from Celestia DA.
    ///
    /// # Arguments
    ///
    /// * `start_height` - First Celestia height to fetch (inclusive)
    /// * `end_height` - Last Celestia height to fetch (inclusive)
    ///
    /// # Returns
    ///
    /// A vector of `(celestia_height, block)` tuples, sorted by Celestia height.
    ///
    /// # Errors
    ///
    /// Returns an error if any retrieval fails.
    pub async fn get_blocks_in_range(
        &self,
        start_height: u64,
        end_height: u64,
    ) -> Result<Vec<(u64, Block)>> {
        if start_height > end_height {
            return Ok(vec![]);
        }

        tracing::info!(
            start_height,
            end_height,
            "Fetching blocks from Celestia range"
        );

        let mut all_blocks = Vec::new();

        for celestia_height in start_height..=end_height {
            let blocks = self.get_blocks(celestia_height).await?;
            for block in blocks {
                all_blocks.push((celestia_height, block));
            }
        }

        tracing::info!(
            start_height,
            end_height,
            block_count = all_blocks.len(),
            "Retrieved blocks from Celestia range"
        );

        Ok(all_blocks)
    }

    /// Verify that a blob is included at the given Celestia height.
    ///
    /// This checks that a blob with the given commitment exists at the
    /// specified height in our namespace.
    ///
    /// # Errors
    ///
    /// Returns an error if verification fails.
    pub async fn verify_inclusion(
        &self,
        celestia_height: u64,
        commitment: &[u8],
    ) -> Result<bool> {
        tracing::debug!(
            celestia_height,
            commitment_len = commitment.len(),
            "Verifying blob inclusion"
        );

        // Get all blobs at this height in our namespace
        use celestia_rpc::BlobClient;
        let blobs = self
            .rpc_client
            .blob_get_all(celestia_height, &[self.namespace])
            .await
            .map_err(|e| CelestiaError::Internal(format!("failed to fetch blobs: {e}")))?;

        let Some(blob_list) = blobs else {
            tracing::debug!(celestia_height, "No blobs found at height");
            return Ok(false);
        };

        // Check if any blob matches the commitment
        for blob in &blob_list {
            let blob_commitment = Self::commitment_to_bytes(blob.commitment.into());
            if blob_commitment == commitment {
                tracing::debug!(celestia_height, "Blob inclusion verified");
                return Ok(true);
            }
        }

        tracing::debug!(
            celestia_height,
            "Blob not found with matching commitment"
        );
        Ok(false)
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

#[async_trait::async_trait]
impl DataAvailability for CelestiaClient {
    async fn submit_block(&self, block: &Block) -> Result<BlobSubmission> {
        let block_hash = block.hash();
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
            tokio_stream::wrappers::BroadcastStream::new(rx).filter_map(|r| r.ok()),
        )
    }

    fn track_finality(&self, _block_hash: B256, submission: BlobSubmission) {
        let tracker = self.finality_tracker.clone();

        tokio::spawn(async move {
            tracker.write().await.track(submission);
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

        let block = Block::new(header, vec![]);

        let encoded = CelestiaClient::encode_block(&block).unwrap();
        let decoded = CelestiaClient::decode_block(&encoded).unwrap();

        assert_eq!(block.height(), decoded.height());
        assert_eq!(block.hash(), decoded.hash());
    }
}
