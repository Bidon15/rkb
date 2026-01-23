//! Execution client implementation.

use alloy_primitives::{Bytes, B256, U256};

// =============================================================================
// Constants
// =============================================================================

/// Default gas limit for block execution.
const DEFAULT_GAS_LIMIT: u64 = 30_000_000;

/// Default base fee per gas (1 gwei).
const DEFAULT_BASE_FEE_GWEI: u64 = 1_000_000_000;

/// HMAC-SHA256 block size in bytes.
const HMAC_BLOCK_SIZE: usize = 64;

use alloy_rpc_types_engine::{
    ExecutionPayloadV1, ExecutionPayloadV2, ExecutionPayloadV3,
    ForkchoiceState as EngineForkchoiceState, ForkchoiceUpdated, PayloadStatus, PayloadStatusEnum,
};
use jsonrpsee::core::client::ClientT;
use jsonrpsee::http_client::{HeaderMap, HeaderValue, HttpClient, HttpClientBuilder};
use jsonrpsee::rpc_params;
use sequencer_types::{Block, ExecutionConfig};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::{Execution, ExecutionError, ExecutionResult, ExecutionStatus, ForkchoiceState, Result};

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

    /// Build execution payload from block.
    fn build_payload(block: &Block) -> ExecutionPayloadV3 {
        // Collect transaction bytes
        let transactions: Vec<Bytes> = block
            .transactions
            .iter()
            .map(|tx| Bytes::from(tx.data().to_vec()))
            .collect();

        let v1 = ExecutionPayloadV1 {
            parent_hash: block.parent_hash(),
            fee_recipient: block.header.proposer,
            state_root: B256::ZERO,     // Computed by reth
            receipts_root: B256::ZERO,  // Computed by reth
            logs_bloom: Default::default(),
            prev_randao: B256::ZERO,    // PoA doesn't use randomness
            block_number: block.height(),
            gas_limit: DEFAULT_GAS_LIMIT,
            gas_used: 0, // Computed by reth
            timestamp: block.timestamp(),
            extra_data: Bytes::new(),
            base_fee_per_gas: U256::from(DEFAULT_BASE_FEE_GWEI),
            block_hash: block.hash(),
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
        let block_hash = block.hash();

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
