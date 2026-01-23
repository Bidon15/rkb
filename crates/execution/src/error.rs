//! Execution error types.

use alloy_primitives::B256;

/// Errors that can occur in the execution module.
#[derive(Debug, thiserror::Error)]
pub enum ExecutionError {
    /// Failed to connect to reth.
    #[error("failed to connect to reth: {0}")]
    ConnectionFailed(String),

    /// Block execution failed.
    #[error("block execution failed: {0}")]
    ExecutionFailed(String),

    /// Invalid block.
    #[error("invalid block: {0}")]
    InvalidBlock(String),

    /// Forkchoice update failed.
    #[error("forkchoice update failed: {0}")]
    ForkchoiceFailed(String),

    /// Parent block not found.
    #[error("parent block not found: {parent_hash}")]
    ParentNotFound {
        /// Parent block hash.
        parent_hash: B256,
    },

    /// Invalid JWT secret.
    #[error("invalid JWT secret: {0}")]
    InvalidJwt(String),

    /// RPC error.
    #[error("RPC error: {0}")]
    Rpc(String),

    /// Internal error.
    #[error("internal error: {0}")]
    Internal(String),
}
