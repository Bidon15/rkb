//! Core types for the PoA sequencer.
//!
//! This crate provides shared type definitions used across all sequencer components.

#![warn(missing_docs)]

mod block;
mod config;
mod transaction;

pub use block::{Block, BlockHash, BlockHeader, BlockParams, Signature};
pub use config::{CelestiaConfig, ChainConfig, ConsensusConfig, ExecutionConfig};
pub use transaction::{Transaction, TransactionHash};

/// Re-export commonly used types from alloy.
pub mod primitives {
    pub use alloy_primitives::{Address, Bytes, B256, U256};
}
