//! Finality tracking for Celestia.

use std::collections::HashMap;
use std::time::Instant;

use alloy_primitives::B256;

use crate::BlobSubmission;

/// Confirmation that a block has been finalized on Celestia.
#[derive(Debug, Clone)]
pub struct FinalityConfirmation {
    /// Our block hash that is now final.
    pub block_hash: B256,

    /// Our block height.
    pub block_height: u64,

    /// Celestia height that finalized this block.
    pub celestia_height: u64,

    /// Celestia block hash (for proofs).
    pub celestia_header_hash: B256,

    /// When the block was submitted.
    pub submitted_at: Instant,

    /// When finality was confirmed.
    pub confirmed_at: Instant,
}

impl FinalityConfirmation {
    /// Time from submission to finality confirmation in milliseconds.
    #[must_use]
    pub fn latency_ms(&self) -> u128 {
        self.confirmed_at.duration_since(self.submitted_at).as_millis()
    }
}

/// Pending finality entry.
#[derive(Debug, Clone)]
pub struct PendingFinality {
    /// Our block hash.
    pub block_hash: B256,

    /// Our block height.
    pub block_height: u64,

    /// Celestia height where blob was submitted.
    pub celestia_height: u64,

    /// Blob commitment.
    pub commitment: Vec<u8>,

    /// When submitted.
    pub submitted_at: Instant,
}

/// Tracks blocks awaiting Celestia finality.
pub struct FinalityTracker {
    /// Blocks awaiting finality, keyed by block hash.
    pending: HashMap<B256, PendingFinality>,

    /// Last known finalized Celestia height.
    celestia_finalized_height: u64,
}

impl FinalityTracker {
    /// Create a new finality tracker.
    #[must_use]
    pub fn new() -> Self {
        Self { pending: HashMap::new(), celestia_finalized_height: 0 }
    }

    /// Track a new submission for finality.
    pub fn track(&mut self, submission: BlobSubmission) {
        let pending = PendingFinality {
            block_hash: submission.block_hash,
            block_height: submission.block_height,
            celestia_height: submission.celestia_height,
            commitment: submission.commitment,
            submitted_at: Instant::now(),
        };

        self.pending.insert(submission.block_hash, pending);
    }

    /// Process a new finalized Celestia height.
    ///
    /// Returns confirmations for any blocks that are now finalized.
    pub fn on_celestia_finality(
        &mut self,
        celestia_height: u64,
        celestia_header_hash: B256,
    ) -> Vec<FinalityConfirmation> {
        self.celestia_finalized_height = celestia_height;
        let now = Instant::now();

        // Find all blocks whose Celestia height is now finalized
        let finalized: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, p)| p.celestia_height <= celestia_height)
            .map(|(hash, p)| FinalityConfirmation {
                block_hash: *hash,
                block_height: p.block_height,
                celestia_height: p.celestia_height,
                celestia_header_hash,
                submitted_at: p.submitted_at,
                confirmed_at: now,
            })
            .collect();

        // Remove from pending
        for confirmation in &finalized {
            self.pending.remove(&confirmation.block_hash);
        }

        // Sort by block height (ascending)
        let mut sorted = finalized;
        sorted.sort_by_key(|c| c.block_height);

        sorted
    }

    /// Number of blocks awaiting finality.
    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Last known finalized Celestia height.
    #[must_use]
    pub fn finalized_height(&self) -> u64 {
        self.celestia_finalized_height
    }
}

impl Default for FinalityTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_track_and_finalize() {
        let mut tracker = FinalityTracker::new();

        let submission = BlobSubmission {
            block_hash: B256::ZERO,
            block_height: 1,
            celestia_height: 100,
            commitment: vec![1, 2, 3],
        };

        tracker.track(submission);
        assert_eq!(tracker.pending_count(), 1);

        // Finalize at height 99 - shouldn't finalize our block
        let confirmations = tracker.on_celestia_finality(99, B256::ZERO);
        assert!(confirmations.is_empty());
        assert_eq!(tracker.pending_count(), 1);

        // Finalize at height 100 - should finalize our block
        let confirmations = tracker.on_celestia_finality(100, B256::ZERO);
        assert_eq!(confirmations.len(), 1);
        assert_eq!(tracker.pending_count(), 0);
    }
}
