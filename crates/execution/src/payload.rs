//! Execution payload types.

use alloy_primitives::{Address, Bytes, B256};

/// Payload ID returned by forkchoiceUpdated when building a block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PayloadId(pub [u8; 8]);

impl PayloadId {
    /// Create from bytes.
    #[must_use]
    pub fn from_bytes(bytes: [u8; 8]) -> Self {
        Self(bytes)
    }
}

/// Attributes for building a new block.
#[derive(Debug, Clone)]
pub struct PayloadAttributes {
    /// Block timestamp.
    pub timestamp: u64,

    /// Previous RANDAO value (use parent block hash for PoA).
    pub prev_randao: B256,

    /// Fee recipient (block proposer).
    pub suggested_fee_recipient: Address,

    /// Withdrawals (empty for PoA).
    pub withdrawals: Vec<()>,

    /// Parent beacon block root (zero for PoA).
    pub parent_beacon_block_root: B256,
}

/// A block built by reth via the builder flow.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BuiltPayload {
    /// The block hash (computed by reth).
    pub block_hash: B256,

    /// Block number.
    pub block_number: u64,

    /// Parent hash.
    pub parent_hash: B256,

    /// Fee recipient.
    pub fee_recipient: Address,

    /// State root (computed by reth).
    pub state_root: B256,

    /// Receipts root (computed by reth).
    pub receipts_root: B256,

    /// Logs bloom.
    pub logs_bloom: Bytes,

    /// Previous RANDAO value (required for block hash computation).
    pub prev_randao: B256,

    /// Extra data (required for block hash computation).
    pub extra_data: Bytes,

    /// Gas limit.
    pub gas_limit: u64,

    /// Gas used.
    pub gas_used: u64,

    /// Block timestamp.
    pub timestamp: u64,

    /// Transactions (RLP encoded).
    pub transactions: Vec<Bytes>,

    /// Base fee per gas.
    pub base_fee_per_gas: u64,
}

/// Result of executing a block.
#[derive(Debug, Clone, PartialEq, Eq)]
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
