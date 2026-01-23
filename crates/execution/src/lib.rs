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
pub use payload::{ExecutionResult, ExecutionStatus};

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
