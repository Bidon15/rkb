//! Execution client implementation.

use alloy_primitives::{Address, Bytes, B256, U256};

// =============================================================================
// Constants
// =============================================================================

/// HMAC-SHA256 block size in bytes.
const HMAC_BLOCK_SIZE: usize = 64;

use alloy_rpc_types_engine::{
    ExecutionPayloadEnvelopeV3, ExecutionPayloadV1, ExecutionPayloadV2, ExecutionPayloadV3,
    ForkchoiceState as EngineForkchoiceState, ForkchoiceUpdated,
    PayloadAttributes as EnginePayloadAttributes, PayloadId as EnginePayloadId, PayloadStatus,
    PayloadStatusEnum,
};
use jsonrpsee::core::client::ClientT;
use jsonrpsee::http_client::{HeaderMap, HeaderValue, HttpClient, HttpClientBuilder};
use jsonrpsee::rpc_params;
use sequencer_types::{Block, ExecutionConfig};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::{
    BlockBuilder, BuiltPayload, Execution, ExecutionError, ExecutionResult, ExecutionStatus,
    ForkchoiceState, PayloadId, Result,
};

/// Client for interacting with reth via Engine API.
pub struct ExecutionClient {
    /// HTTP client for Engine API calls.
    client: HttpClient,

    /// Current forkchoice state.
    forkchoice: Arc<RwLock<ForkchoiceState>>,
}

impl ExecutionClient {
    /// Create a new execution client.
    ///
    /// # Errors
    ///
    /// Returns an error if the client cannot be initialized.
    pub fn new(config: ExecutionConfig) -> Result<Self> {
        if config.reth_url.is_empty() {
            return Err(ExecutionError::ConnectionFailed(
                "reth_url cannot be empty".to_string(),
            ));
        }

        // Read JWT secret
        let jwt_secret = std::fs::read_to_string(&config.jwt_secret_path)
            .map_err(|e| ExecutionError::ConnectionFailed(format!("failed to read JWT secret: {e}")))?;
        let jwt_secret = jwt_secret.trim();

        // Build JWT token (HS256)
        let jwt_token = build_jwt_token(jwt_secret)
            .map_err(|e| ExecutionError::ConnectionFailed(format!("failed to build JWT: {e}")))?;

        // Build HTTP client with Authorization header
        let mut headers = HeaderMap::new();
        headers.insert(
            "Authorization",
            HeaderValue::from_str(&format!("Bearer {jwt_token}"))
                .map_err(|e| ExecutionError::ConnectionFailed(format!("invalid header value: {e}")))?,
        );

        let client = HttpClientBuilder::default()
            .set_headers(headers)
            .build(&config.reth_url)
            .map_err(|e| ExecutionError::ConnectionFailed(format!("failed to build HTTP client: {e}")))?;

        Ok(Self {
            client,
            forkchoice: Arc::new(RwLock::new(ForkchoiceState::default())),
        })
    }

    /// Get the genesis block hash from reth.
    ///
    /// This queries `eth_getBlockByNumber` for block 0 to get the actual
    /// genesis hash. Must be called at startup to initialize forkchoice.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub async fn get_genesis_hash(&self) -> Result<B256> {
        #[derive(serde::Deserialize)]
        struct BlockResponse {
            hash: B256,
        }

        let response: BlockResponse = self
            .client
            .request("eth_getBlockByNumber", rpc_params!["0x0", false])
            .await
            .map_err(|e| ExecutionError::ConnectionFailed(format!("failed to get genesis: {e}")))?;

        tracing::info!(genesis_hash = %response.hash, "Retrieved genesis block hash from reth");
        Ok(response.hash)
    }

    /// Get reth's current head block hash and number.
    ///
    /// This queries `eth_getBlockByNumber("latest")` to get reth's actual
    /// chain head. Used for sync-aware proposals to verify our state matches reth.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub async fn get_head(&self) -> Result<(B256, u64)> {
        #[derive(serde::Deserialize)]
        struct BlockResponse {
            hash: B256,
            #[serde(deserialize_with = "deserialize_u64_hex")]
            number: u64,
        }

        /// Deserialize a hex string (with 0x prefix) to u64
        fn deserialize_u64_hex<'de, D>(deserializer: D) -> std::result::Result<u64, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            let s: String = serde::Deserialize::deserialize(deserializer)?;
            u64::from_str_radix(s.trim_start_matches("0x"), 16)
                .map_err(serde::de::Error::custom)
        }

        let response: BlockResponse = self
            .client
            .request("eth_getBlockByNumber", rpc_params!["latest", false])
            .await
            .map_err(|e| ExecutionError::ConnectionFailed(format!("failed to get head: {e}")))?;

        Ok((response.hash, response.number))
    }

    /// Initialize the forkchoice state with the genesis block hash.
    ///
    /// This calls `forkchoiceUpdated` on reth to establish the initial forkchoice,
    /// which is required before block building can start. Retries if reth is syncing.
    ///
    /// # Errors
    ///
    /// Returns an error if the forkchoice update fails after retries.
    pub async fn init_forkchoice(&self, genesis_hash: B256) -> Result<()> {
        let state = ForkchoiceState {
            head: genesis_hash,
            safe: genesis_hash,
            finalized: genesis_hash,
        };

        tracing::info!(%genesis_hash, "Initializing forkchoice with genesis...");

        // Retry loop - reth might be starting up
        let max_retries = 30;
        let retry_delay = std::time::Duration::from_secs(1);

        for attempt in 1..=max_retries {
            match self.update_forkchoice(state).await {
                Ok(()) => {
                    tracing::info!(%genesis_hash, attempt, "Forkchoice initialized with genesis on reth");
                    return Ok(());
                }
                Err(e) => {
                    if attempt < max_retries {
                        tracing::warn!(
                            %genesis_hash,
                            attempt,
                            max_retries,
                            error = %e,
                            "Forkchoice init failed, reth may be starting up. Retrying..."
                        );
                        tokio::time::sleep(retry_delay).await;
                    } else {
                        return Err(e);
                    }
                }
            }
        }

        unreachable!()
    }

    /// Apply a built payload to reth and update forkchoice to make it the new HEAD.
    ///
    /// This is called after consensus finalizes a block to actually apply it to reth's chain.
    /// Without this, reth doesn't know about the block and will return SYNCING when trying
    /// to build the next block.
    ///
    /// # Errors
    ///
    /// Returns an error if the payload is invalid or forkchoice update fails.
    pub async fn apply_built_payload(&self, payload: &BuiltPayload) -> Result<()> {
        tracing::debug!(
            block_number = payload.block_number,
            %payload.block_hash,
            "Applying built payload to reth"
        );

        // Reconstruct ExecutionPayloadV3 from BuiltPayload using exact values from reth
        let v1 = ExecutionPayloadV1 {
            parent_hash: payload.parent_hash,
            fee_recipient: payload.fee_recipient,
            state_root: payload.state_root,
            receipts_root: payload.receipts_root,
            logs_bloom: alloy_primitives::Bloom::from_slice(&payload.logs_bloom),
            prev_randao: payload.prev_randao,
            block_number: payload.block_number,
            gas_limit: payload.gas_limit,
            gas_used: payload.gas_used,
            timestamp: payload.timestamp,
            extra_data: payload.extra_data.clone(),
            base_fee_per_gas: U256::from(payload.base_fee_per_gas),
            block_hash: payload.block_hash,
            transactions: payload.transactions.clone(),
        };

        let v2 = ExecutionPayloadV2 {
            payload_inner: v1,
            withdrawals: vec![], // No withdrawals in PoA
        };

        let execution_payload = ExecutionPayloadV3 {
            payload_inner: v2,
            blob_gas_used: 0,
            excess_blob_gas: 0,
        };

        // Call engine_newPayloadV3 to validate and import the block
        let response: PayloadStatus = self
            .client
            .request(
                "engine_newPayloadV3",
                rpc_params![execution_payload, Vec::<B256>::new(), B256::ZERO],
            )
            .await
            .map_err(|e| ExecutionError::ExecutionFailed(format!("newPayload RPC failed: {e}")))?;

        tracing::debug!(
            ?response.status,
            latest_valid_hash = ?response.latest_valid_hash,
            "engine_newPayloadV3 response for built payload"
        );

        match response.status {
            PayloadStatusEnum::Valid | PayloadStatusEnum::Accepted => {
                // Block imported successfully, now update forkchoice
            }
            PayloadStatusEnum::Syncing => {
                // Reth is processing - this is ok, forkchoice update will complete it
                tracing::debug!("newPayload returned SYNCING, proceeding with forkchoice update");
            }
            PayloadStatusEnum::Invalid { ref validation_error } => {
                return Err(ExecutionError::InvalidBlock(validation_error.clone()));
            }
        }

        // Update forkchoice to make this block the new HEAD
        let new_state = ForkchoiceState {
            head: payload.block_hash,
            safe: payload.block_hash,
            finalized: self.forkchoice.read().await.finalized,
        };

        self.update_forkchoice(new_state).await?;

        tracing::info!(
            block_number = payload.block_number,
            %payload.block_hash,
            "Built payload applied to reth and forkchoice updated"
        );

        Ok(())
    }

    /// Build execution payload from block.
    ///
    /// All blocks now have required reth-computed fields from the vanilla Ethereum
    /// build flow, so this is a direct conversion.
    fn build_payload(block: &Block) -> ExecutionPayloadV3 {
        // Collect transaction bytes
        let transactions: Vec<Bytes> = block
            .transactions
            .iter()
            .map(|tx| Bytes::from(tx.data().to_vec()))
            .collect();

        // All fields are now required - no more conditional defaults
        let v1 = ExecutionPayloadV1 {
            parent_hash: block.parent_hash(),
            fee_recipient: block.header.proposer,
            state_root: block.state_root,
            receipts_root: block.receipts_root,
            logs_bloom: alloy_primitives::Bloom::from_slice(&block.logs_bloom),
            prev_randao: block.prev_randao,
            block_number: block.height(),
            gas_limit: block.gas_limit,
            gas_used: block.gas_used,
            timestamp: block.timestamp(),
            extra_data: block.extra_data.clone(),
            base_fee_per_gas: U256::from(block.base_fee_per_gas),
            block_hash: block.block_hash,
            transactions,
        };

        let v2 = ExecutionPayloadV2 {
            payload_inner: v1,
            withdrawals: vec![], // No withdrawals in PoA
        };

        ExecutionPayloadV3 {
            payload_inner: v2,
            blob_gas_used: 0,
            excess_blob_gas: 0,
        }
    }
}

#[async_trait::async_trait]
impl Execution for ExecutionClient {
    async fn execute_block(&self, block: &Block) -> Result<ExecutionResult> {
        let block_height = block.height();
        let block_hash = block.block_hash;

        tracing::debug!(
            block_height,
            %block_hash,
            parent_hash = %block.parent_hash(),
            tx_count = block.tx_count(),
            "Executing block via Engine API"
        );

        let payload = Self::build_payload(block);

        // Call engine_newPayloadV3
        // Parameters: payload, versioned_hashes (empty for non-blob txs), parent_beacon_block_root
        let response: PayloadStatus = self
            .client
            .request(
                "engine_newPayloadV3",
                rpc_params![payload, Vec::<B256>::new(), B256::ZERO],
            )
            .await
            .map_err(|e| ExecutionError::ExecutionFailed(format!("newPayload RPC failed: {e}")))?;

        tracing::debug!(?response.status, "engine_newPayloadV3 response");

        let status = match response.status {
            PayloadStatusEnum::Valid => ExecutionStatus::Valid,
            PayloadStatusEnum::Invalid { ref validation_error } => {
                return Err(ExecutionError::InvalidBlock(
                    validation_error.clone(),
                ));
            }
            PayloadStatusEnum::Syncing => ExecutionStatus::Syncing,
            PayloadStatusEnum::Accepted => ExecutionStatus::Accepted,
        };

        // Use the block hash returned by reth if available, otherwise use ours
        let final_block_hash = response.latest_valid_hash.unwrap_or(block_hash);

        tracing::info!(
            block_height,
            %final_block_hash,
            ?status,
            "Block executed"
        );

        Ok(ExecutionResult {
            block_hash: final_block_hash,
            block_number: block_height,
            gas_used: 0, // Would need to fetch from reth
            status,
        })
    }

    async fn update_forkchoice(&self, state: ForkchoiceState) -> Result<()> {
        tracing::debug!(
            head = %state.head,
            safe = %state.safe,
            finalized = %state.finalized,
            "Updating forkchoice"
        );

        let engine_state = EngineForkchoiceState {
            head_block_hash: state.head,
            safe_block_hash: state.safe,
            finalized_block_hash: state.finalized,
        };

        // Call engine_forkchoiceUpdatedV3
        // Parameters: forkchoice_state, payload_attributes (None = no new block)
        let response: ForkchoiceUpdated = self
            .client
            .request(
                "engine_forkchoiceUpdatedV3",
                rpc_params![engine_state, Option::<()>::None],
            )
            .await
            .map_err(|e| ExecutionError::ForkchoiceFailed(format!("forkchoiceUpdated RPC failed: {e}")))?;

        tracing::debug!(?response.payload_status.status, "engine_forkchoiceUpdatedV3 response");

        match response.payload_status.status {
            PayloadStatusEnum::Valid | PayloadStatusEnum::Syncing | PayloadStatusEnum::Accepted => {
                // Update our cached forkchoice
                *self.forkchoice.write().await = state;
                Ok(())
            }
            PayloadStatusEnum::Invalid { ref validation_error } => {
                Err(ExecutionError::ForkchoiceFailed(validation_error.clone()))
            }
        }
    }

    async fn forkchoice(&self) -> Result<ForkchoiceState> {
        Ok(*self.forkchoice.read().await)
    }
}

#[async_trait::async_trait]
impl BlockBuilder for ExecutionClient {
    async fn get_head(&self) -> Result<(B256, u64)> {
        // Delegate to inherent method
        Self::get_head(self).await
    }

    async fn import_payload(&self, payload: &BuiltPayload) -> Result<()> {
        tracing::debug!(
            block_number = payload.block_number,
            %payload.block_hash,
            "Importing payload to reth (newPayloadV3 only, no forkchoice update)"
        );

        // Reconstruct ExecutionPayloadV3 from BuiltPayload
        let v1 = ExecutionPayloadV1 {
            parent_hash: payload.parent_hash,
            fee_recipient: payload.fee_recipient,
            state_root: payload.state_root,
            receipts_root: payload.receipts_root,
            logs_bloom: alloy_primitives::Bloom::from_slice(&payload.logs_bloom),
            prev_randao: payload.prev_randao,
            block_number: payload.block_number,
            gas_limit: payload.gas_limit,
            gas_used: payload.gas_used,
            timestamp: payload.timestamp,
            extra_data: payload.extra_data.clone(),
            base_fee_per_gas: U256::from(payload.base_fee_per_gas),
            block_hash: payload.block_hash,
            transactions: payload.transactions.clone(),
        };

        let v2 = ExecutionPayloadV2 {
            payload_inner: v1,
            withdrawals: vec![],
        };

        let execution_payload = ExecutionPayloadV3 {
            payload_inner: v2,
            blob_gas_used: 0,
            excess_blob_gas: 0,
        };

        // Call engine_newPayloadV3 to import (NOT finalize) the block
        let response: PayloadStatus = self
            .client
            .request(
                "engine_newPayloadV3",
                rpc_params![execution_payload, Vec::<B256>::new(), B256::ZERO],
            )
            .await
            .map_err(|e| ExecutionError::ExecutionFailed(format!("newPayload RPC failed: {e}")))?;

        tracing::debug!(
            ?response.status,
            latest_valid_hash = ?response.latest_valid_hash,
            block_number = payload.block_number,
            "engine_newPayloadV3 response (import only)"
        );

        match response.status {
            PayloadStatusEnum::Valid | PayloadStatusEnum::Accepted | PayloadStatusEnum::Syncing => {
                // Block imported successfully (or reth is processing it)
                tracing::info!(
                    block_number = payload.block_number,
                    %payload.block_hash,
                    status = ?response.status,
                    "Payload imported to reth"
                );
                Ok(())
            }
            PayloadStatusEnum::Invalid { ref validation_error } => {
                Err(ExecutionError::InvalidBlock(validation_error.clone()))
            }
        }
    }

    async fn start_building(
        &self,
        parent_hash: B256,
        timestamp: u64,
        fee_recipient: Address,
    ) -> Result<PayloadId> {
        tracing::debug!(
            %parent_hash,
            timestamp,
            %fee_recipient,
            "Starting block building"
        );

        // Get current forkchoice for finalized hash
        let current = *self.forkchoice.read().await;

        // Use parent_hash for both head and safe to avoid state inconsistency.
        // In our PoA model with no reorgs, the parent we're building on is safe.
        // Finalized comes from the cached state (updated on DA confirmation).
        let engine_state = EngineForkchoiceState {
            head_block_hash: parent_hash,
            safe_block_hash: parent_hash,
            finalized_block_hash: current.finalized,
        };

        // Build payload attributes for V3
        let payload_attributes = EnginePayloadAttributes {
            timestamp,
            prev_randao: parent_hash, // Use parent hash as prev_randao for PoA
            suggested_fee_recipient: fee_recipient,
            withdrawals: Some(vec![]), // Empty withdrawals for PoA
            parent_beacon_block_root: Some(B256::ZERO), // Required for V3
            target_blobs_per_block: None,
            max_blobs_per_block: None,
        };

        // Call forkchoiceUpdated with payload attributes to start building
        let response: ForkchoiceUpdated = self
            .client
            .request(
                "engine_forkchoiceUpdatedV3",
                rpc_params![engine_state, Some(payload_attributes)],
            )
            .await
            .map_err(|e| {
                ExecutionError::ForkchoiceFailed(format!("forkchoiceUpdated RPC failed: {e}"))
            })?;

        tracing::info!(
            status = ?response.payload_status.status,
            payload_id = ?response.payload_id,
            latest_valid_hash = ?response.payload_status.latest_valid_hash,
            head = %parent_hash,
            safe = %parent_hash,
            finalized = %current.finalized,
            "engine_forkchoiceUpdatedV3 response (with payload attributes)"
        );

        match response.payload_status.status {
            PayloadStatusEnum::Valid => {
                // Only VALID status returns a payload ID
                let payload_id = response.payload_id.ok_or_else(|| {
                    ExecutionError::ForkchoiceFailed(
                        "forkchoiceUpdated returned no payload ID".to_string(),
                    )
                })?;

                tracing::info!(
                    %parent_hash,
                    ?payload_id,
                    "Block building started"
                );

                // Convert from alloy PayloadId to our PayloadId
                Ok(PayloadId::from_bytes(payload_id.0.into()))
            }
            PayloadStatusEnum::Syncing => {
                // Reth is syncing - can't build blocks yet
                Err(ExecutionError::ForkchoiceFailed(
                    "reth is syncing, cannot build blocks yet".to_string(),
                ))
            }
            PayloadStatusEnum::Accepted => {
                // Accepted but not validated - unusual for block building
                Err(ExecutionError::ForkchoiceFailed(
                    "forkchoice accepted but not validated, cannot build".to_string(),
                ))
            }
            PayloadStatusEnum::Invalid { ref validation_error } => {
                Err(ExecutionError::ForkchoiceFailed(format!(
                    "invalid forkchoice: {}",
                    validation_error
                )))
            }
        }
    }

    async fn get_payload(&self, payload_id: PayloadId) -> Result<BuiltPayload> {
        tracing::debug!(?payload_id, "Getting built payload");

        // Convert our PayloadId to alloy's
        let engine_payload_id = EnginePayloadId::new(payload_id.0);

        // Call getPayloadV3
        let response: ExecutionPayloadEnvelopeV3 = self
            .client
            .request("engine_getPayloadV3", rpc_params![engine_payload_id])
            .await
            .map_err(|e| ExecutionError::ExecutionFailed(format!("getPayload RPC failed: {e}")))?;

        let payload = response.execution_payload;
        let v2 = payload.payload_inner;
        let v1 = v2.payload_inner;

        tracing::info!(
            block_number = v1.block_number,
            %v1.block_hash,
            gas_used = v1.gas_used,
            tx_count = v1.transactions.len(),
            "Got built payload from reth"
        );

        Ok(BuiltPayload {
            block_hash: v1.block_hash,
            block_number: v1.block_number,
            parent_hash: v1.parent_hash,
            fee_recipient: v1.fee_recipient,
            state_root: v1.state_root,
            receipts_root: v1.receipts_root,
            logs_bloom: v1.logs_bloom.0.to_vec().into(),
            prev_randao: v1.prev_randao,
            extra_data: v1.extra_data.clone(),
            gas_limit: v1.gas_limit,
            gas_used: v1.gas_used,
            timestamp: v1.timestamp,
            transactions: v1.transactions,
            base_fee_per_gas: v1.base_fee_per_gas.to::<u64>(),
        })
    }
}

/// Build a JWT token for Engine API authentication.
fn build_jwt_token(secret_hex: &str) -> std::result::Result<String, String> {
    use alloy_primitives::hex;

    // Decode hex secret
    let secret_bytes = hex::decode(secret_hex.trim_start_matches("0x"))
        .map_err(|e| format!("invalid hex: {e}"))?;

    if secret_bytes.len() != 32 {
        return Err(format!("JWT secret must be 32 bytes, got {}", secret_bytes.len()));
    }

    // Build JWT header and payload
    let header = r#"{"alg":"HS256","typ":"JWT"}"#;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let payload = format!(r#"{{"iat":{now}}}"#);

    // Base64url encode
    let header_b64 = base64url_encode(header.as_bytes());
    let payload_b64 = base64url_encode(payload.as_bytes());

    let message = format!("{header_b64}.{payload_b64}");

    // HMAC-SHA256 signature
    let signature = hmac_sha256(&secret_bytes, message.as_bytes());
    let signature_b64 = base64url_encode(&signature);

    Ok(format!("{message}.{signature_b64}"))
}

/// Base64url encode without padding.
fn base64url_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

    let mut result = String::new();
    let mut i = 0;

    while i < data.len() {
        let b0 = data[i] as usize;
        let b1 = data.get(i + 1).copied().unwrap_or(0) as usize;
        let b2 = data.get(i + 2).copied().unwrap_or(0) as usize;

        result.push(ALPHABET[b0 >> 2] as char);
        result.push(ALPHABET[((b0 & 0x03) << 4) | (b1 >> 4)] as char);

        if i + 1 < data.len() {
            result.push(ALPHABET[((b1 & 0x0f) << 2) | (b2 >> 6)] as char);
        }
        if i + 2 < data.len() {
            result.push(ALPHABET[b2 & 0x3f] as char);
        }

        i += 3;
    }

    result
}

/// HMAC-SHA256 implementation.
fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    // Prepare key
    let mut key_block = [0u8; HMAC_BLOCK_SIZE];
    if key.len() > HMAC_BLOCK_SIZE {
        let hash = Sha256::digest(key);
        key_block[..32].copy_from_slice(&hash);
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }

    // Inner padding
    let mut inner = [0x36u8; HMAC_BLOCK_SIZE];
    for (i, b) in key_block.iter().enumerate() {
        inner[i] ^= b;
    }

    // Outer padding
    let mut outer = [0x5cu8; HMAC_BLOCK_SIZE];
    for (i, b) in key_block.iter().enumerate() {
        outer[i] ^= b;
    }

    // Inner hash
    let mut hasher = Sha256::new();
    hasher.update(inner);
    hasher.update(message);
    let inner_hash = hasher.finalize();

    // Outer hash
    let mut hasher = Sha256::new();
    hasher.update(outer);
    hasher.update(inner_hash);
    let result = hasher.finalize();

    let mut output = [0u8; 32];
    output.copy_from_slice(&result);
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_url_fails() {
        // Note: Can't test new() without a valid JWT file
        let config =
            ExecutionConfig { reth_url: String::new(), jwt_secret_path: "/tmp/jwt.hex".into() };
        let client = ExecutionClient::new(config);
        assert!(client.is_err());
    }

    #[test]
    fn test_jwt_token_generation() {
        let secret = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let token = build_jwt_token(secret);
        assert!(token.is_ok());
        let token = token.unwrap();
        // JWT should have 3 parts separated by dots
        assert_eq!(token.split('.').count(), 3);
    }

    #[test]
    fn test_base64url_encode() {
        assert_eq!(base64url_encode(b"hello"), "aGVsbG8");
        assert_eq!(base64url_encode(b""), "");
    }
}
