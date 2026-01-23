//! Execution client implementation.

use sequencer_types::{Block, ExecutionConfig};

use crate::{Execution, ExecutionError, ExecutionResult, ExecutionStatus, ForkchoiceState, Result};

/// Client for interacting with reth via Engine API.
pub struct ExecutionClient {
    config: ExecutionConfig,
    forkchoice: ForkchoiceState,
}

impl ExecutionClient {
    /// Create a new execution client.
    ///
    /// # Errors
    ///
    /// Returns an error if the client cannot be initialized.
    pub fn new(config: ExecutionConfig) -> Result<Self> {
        if config.reth_url.is_empty() {
            return Err(ExecutionError::ConnectionFailed("reth_url cannot be empty".to_string()));
        }

        Ok(Self { config, forkchoice: ForkchoiceState::default() })
    }

    /// Get the reth URL.
    #[must_use]
    pub fn reth_url(&self) -> &str {
        &self.config.reth_url
    }

    /// Build execution payload from block.
    fn build_payload(&self, _block: &Block) -> Result<()> {
        // TODO: Build ExecutionPayloadV3 from block
        // - parent_hash from block.parent_hash()
        // - timestamp from block.timestamp()
        // - transactions RLP-encoded
        Ok(())
    }
}

#[async_trait::async_trait]
impl Execution for ExecutionClient {
    async fn execute_block(&self, block: &Block) -> Result<ExecutionResult> {
        let block_height = block.height();

        tracing::debug!(
            block_height,
            parent_hash = %block.parent_hash(),
            "Executing block via Engine API"
        );

        // TODO: Build payload
        self.build_payload(block)?;

        // TODO: Call engine_newPayloadV3
        // let status = client.request("engine_newPayloadV3", ...).await?;

        // Placeholder result
        let result = ExecutionResult {
            block_hash: block.hash(),
            block_number: block_height,
            gas_used: 0,
            status: ExecutionStatus::Valid,
        };

        tracing::info!(
            block_height,
            block_hash = %result.block_hash,
            "Block executed successfully"
        );

        Ok(result)
    }

    async fn update_forkchoice(&self, state: ForkchoiceState) -> Result<()> {
        tracing::debug!(
            head = %state.head,
            safe = %state.safe,
            finalized = %state.finalized,
            "Updating forkchoice"
        );

        // TODO: Call engine_forkchoiceUpdatedV3
        // let result = client.request("engine_forkchoiceUpdatedV3", ...).await?;

        Ok(())
    }

    async fn forkchoice(&self) -> Result<ForkchoiceState> {
        Ok(self.forkchoice)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> ExecutionConfig {
        ExecutionConfig {
            reth_url: "http://localhost:8551".to_string(),
            jwt_secret_path: "/tmp/jwt.hex".into(),
        }
    }

    #[test]
    fn test_new_client() {
        let config = test_config();
        let client = ExecutionClient::new(config);
        assert!(client.is_ok());
    }

    #[test]
    fn test_empty_url_fails() {
        let config =
            ExecutionConfig { reth_url: String::new(), jwt_secret_path: "/tmp/jwt.hex".into() };
        let client = ExecutionClient::new(config);
        assert!(client.is_err());
    }
}
