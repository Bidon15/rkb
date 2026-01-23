//! Execution payload types.

use alloy_primitives::B256;

/// Result of executing a block.
#[derive(Debug, Clone)]
pub struct ExecutionResult {
    /// Block hash (computed by reth).
    pub block_hash: B256,

    /// Block number.
    pub block_number: u64,

    /// Gas used by all transactions.
    pub gas_used: u64,

    /// Execution status.
    pub status: ExecutionStatus,
}

/// Status of block execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionStatus {
    /// Block was executed successfully.
    Valid,

    /// Block was invalid.
    Invalid {
        /// Reason for invalidity.
        reason: String,
    },

    /// Node is syncing.
    Syncing,

    /// Status is unknown/accepted.
    Accepted,
}

impl ExecutionResult {
    /// Check if execution was successful.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        matches!(self.status, ExecutionStatus::Valid)
    }
}
