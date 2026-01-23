//! Transaction types.

use alloy_primitives::{keccak256, B256};
use serde::{Deserialize, Serialize};

/// Hash of a transaction.
pub type TransactionHash = B256;

/// A transaction to be included in a block.
///
/// Transactions are opaque bytes - the execution layer (reth) interprets them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transaction {
    /// Raw transaction bytes (RLP-encoded Ethereum transaction).
    data: Vec<u8>,
}

impl Transaction {
    /// Create a new transaction from raw bytes.
    #[must_use]
    pub fn new(data: Vec<u8>) -> Self {
        Self { data }
    }

    /// Get the raw transaction bytes.
    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Consume and return the raw bytes.
    #[must_use]
    pub fn into_data(self) -> Vec<u8> {
        self.data
    }

    /// Length of the transaction data.
    #[must_use]
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Check if transaction data is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Compute the transaction hash (keccak256).
    ///
    /// Note: This performs a cryptographic hash computation on each call.
    /// Cache the result if calling multiple times.
    #[must_use]
    pub fn hash(&self) -> TransactionHash {
        keccak256(&self.data)
    }
}
