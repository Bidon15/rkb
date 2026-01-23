//! Celestia DA module.
//!
//! This crate handles blob submission to Celestia and finality tracking
//! using the Lumina light client.

#![warn(missing_docs)]

mod client;
mod error;
mod finality;
mod index;

pub use client::CelestiaClient;
pub use index::{BlockIndex, IndexEntry};
pub use error::CelestiaError;
pub use finality::{FinalityConfirmation, FinalityTracker};
use futures::Stream;

use alloy_primitives::B256;
use sequencer_types::Block;

/// Result type for Celestia operations.
pub type Result<T> = std::result::Result<T, CelestiaError>;

/// Information about a submitted blob.
#[derive(Debug, Clone)]
pub struct BlobSubmission {
    /// Our block hash.
    pub block_hash: B256,

    /// Our block height.
    pub block_height: u64,

    /// Celestia block height where blob was included.
    pub celestia_height: u64,

    /// Blob commitment for verification.
    pub commitment: Vec<u8>,
}

/// Trait for Celestia DA operations.
#[async_trait::async_trait]
pub trait DataAvailability: Send + Sync {
    /// Submit a block as a blob to Celestia.
    ///
    /// # Errors
    ///
    /// Returns an error if submission fails.
    async fn submit_block(&self, block: &Block) -> Result<BlobSubmission>;

    /// Get finality confirmation stream.
    fn finality_stream(&self) -> Box<dyn Stream<Item = FinalityConfirmation> + Send + Unpin>;

    /// Track a submission for finality.
    fn track_finality(&self, block_hash: B256, submission: BlobSubmission);
}
