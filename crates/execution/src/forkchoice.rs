//! Forkchoice state management.

use alloy_primitives::B256;

/// Current forkchoice state.
///
/// Maps to Ethereum's forkchoice rule:
/// - `head`: Latest block (soft confirmation)
/// - `safe`: Safe block (soft confirmation)
/// - `finalized`: Finalized block (firm confirmation from Celestia)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ForkchoiceState {
    /// Head block hash (latest soft-confirmed block).
    pub head: B256,

    /// Safe block hash (same as head for PoA).
    pub safe: B256,

    /// Finalized block hash (Celestia-finalized block).
    pub finalized: B256,
}

impl ForkchoiceState {
    /// Create a new forkchoice state at genesis.
    #[must_use]
    pub fn genesis(genesis_hash: B256) -> Self {
        Self { head: genesis_hash, safe: genesis_hash, finalized: genesis_hash }
    }

    /// Update for a new soft confirmation.
    ///
    /// Moves head and safe forward, keeps finalized unchanged.
    #[must_use]
    pub fn with_soft(self, block_hash: B256) -> Self {
        Self { head: block_hash, safe: block_hash, finalized: self.finalized }
    }

    /// Update for a new firm confirmation.
    ///
    /// Only moves finalized forward.
    #[must_use]
    pub fn with_firm(self, block_hash: B256) -> Self {
        Self { head: self.head, safe: self.safe, finalized: block_hash }
    }
}

impl Default for ForkchoiceState {
    fn default() -> Self {
        Self::genesis(B256::ZERO)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_forkchoice_updates() {
        let genesis = B256::repeat_byte(0x00);
        let block1 = B256::repeat_byte(0x01);
        let block2 = B256::repeat_byte(0x02);

        let state = ForkchoiceState::genesis(genesis);
        assert_eq!(state.head, genesis);
        assert_eq!(state.finalized, genesis);

        // Soft update
        let state = state.with_soft(block1);
        assert_eq!(state.head, block1);
        assert_eq!(state.safe, block1);
        assert_eq!(state.finalized, genesis);

        // Another soft update
        let state = state.with_soft(block2);
        assert_eq!(state.head, block2);
        assert_eq!(state.finalized, genesis);

        // Firm update
        let state = state.with_firm(block1);
        assert_eq!(state.head, block2);
        assert_eq!(state.finalized, block1);
    }
}
