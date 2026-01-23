//! Execution module using reth Engine API.
//!
//! This crate drives block execution on a vanilla reth node via the Engine API.

#![warn(missing_docs)]

mod client;
mod error;
mod forkchoice;
mod jwt;
mod payload;

pub use client::ExecutionClient;
pub use error::ExecutionError;
pub use forkchoice::ForkchoiceState;
pub use payload::{BuiltPayload, ExecutionResult, ExecutionStatus, PayloadAttributes, PayloadId};

use alloy_primitives::{Address, B256};
use sequencer_types::Block;

/// Result type for execution operations.
pub type Result<T> = std::result::Result<T, ExecutionError>;

/// Trait for execution layer operations.
#[async_trait::async_trait]
pub trait Execution: Send + Sync {
    /// Execute a block and return the result.
    ///
    /// # Errors
    ///
    /// Returns an error if execution fails.
    async fn execute_block(&self, block: &Block) -> Result<ExecutionResult>;

    /// Update forkchoice (soft/firm confirmations).
    ///
    /// # Errors
    ///
    /// Returns an error if the update fails.
    async fn update_forkchoice(&self, state: ForkchoiceState) -> Result<()>;

    /// Get current forkchoice state.
    ///
    /// # Errors
    ///
    /// Returns an error if the state cannot be retrieved.
    async fn forkchoice(&self) -> Result<ForkchoiceState>;
}

/// Trait for block building operations (vanilla Ethereum flow).
///
/// This trait provides the builder pattern where reth builds blocks
/// from its mempool, rather than the sequencer specifying transactions.
#[async_trait::async_trait]
pub trait BlockBuilder: Send + Sync {
    /// Get reth's current head block.
    ///
    /// Returns the (hash, number, timestamp) of reth's chain head. Used for:
    /// - Sync-aware proposals to verify consensus state matches execution state
    /// - Ensuring new block timestamps are strictly greater than parent
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    async fn get_head(&self) -> Result<(B256, u64, u64)>;

    /// Import a payload to reth without updating forkchoice.
    ///
    /// This calls `newPayloadV3` to make reth aware of a block, but does NOT
    /// make it the chain head. This is how Ethereum works:
    /// 1. Blocks are gossiped and imported (newPayload)
    /// 2. Finality is determined (attestations/consensus)
    /// 3. Head is updated (forkchoiceUpdated)
    ///
    /// By importing blocks early (on relay receive), reth knows about them
    /// before we try to build on top of them.
    ///
    /// # Errors
    ///
    /// Returns an error if the import fails.
    async fn import_payload(&self, payload: &BuiltPayload) -> Result<()>;

    /// Start building a new block.
    ///
    /// Calls `forkchoiceUpdated` with payload attributes to signal reth
    /// to start building a block from its mempool.
    ///
    /// # Arguments
    ///
    /// * `parent_hash` - Hash of the parent block
    /// * `timestamp` - Timestamp for the new block
    /// * `fee_recipient` - Address to receive transaction fees
    ///
    /// # Returns
    ///
    /// A payload ID that can be used to retrieve the built block.
    ///
    /// # Errors
    ///
    /// Returns an error if the build request fails.
    async fn start_building(
        &self,
        parent_hash: B256,
        timestamp: u64,
        fee_recipient: Address,
    ) -> Result<PayloadId>;

    /// Get a built block.
    ///
    /// Calls `getPayload` to retrieve a block that reth has built.
    ///
    /// # Arguments
    ///
    /// * `payload_id` - The ID returned by `start_building`
    ///
    /// # Returns
    ///
    /// The built payload with the correct block hash.
    ///
    /// # Errors
    ///
    /// Returns an error if the payload cannot be retrieved.
    async fn get_payload(&self, payload_id: PayloadId) -> Result<BuiltPayload>;
}
