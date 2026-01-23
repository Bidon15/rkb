//! Execution module using reth Engine API.
//!
//! This crate drives block execution on a vanilla reth node via the Engine API.

#![warn(missing_docs)]

mod client;
mod error;
mod forkchoice;
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
