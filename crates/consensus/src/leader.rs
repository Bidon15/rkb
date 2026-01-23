//! Leader election utilities.
//!
//! This module provides round-robin leader election for the PoA consensus.

/// Determine the leader index for a given height using round-robin.
///
/// The leader rotates through validators in order based on block height.
///
/// # Arguments
///
/// * `height` - The block height
/// * `validator_count` - Number of validators in the set
///
/// # Returns
///
/// The index of the validator who should be leader for this height.
#[inline]
#[must_use]
pub fn leader_index(height: u64, validator_count: usize) -> usize {
    (height as usize) % validator_count
}

/// Check if a validator is the leader for a given height.
///
/// # Arguments
///
/// * `our_index` - Our index in the validator set
/// * `height` - The block height
/// * `validator_count` - Number of validators in the set
///
/// # Returns
///
/// `true` if we are the leader for this height, `false` otherwise.
#[inline]
#[must_use]
pub fn is_leader(our_index: usize, height: u64, validator_count: usize) -> bool {
    leader_index(height, validator_count) == our_index
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_leader_rotation() {
        assert_eq!(leader_index(0, 3), 0);
        assert_eq!(leader_index(1, 3), 1);
        assert_eq!(leader_index(2, 3), 2);
        assert_eq!(leader_index(3, 3), 0); // wraps
        assert_eq!(leader_index(4, 3), 1);
        assert_eq!(leader_index(5, 3), 2);
        assert_eq!(leader_index(6, 3), 0); // wraps again
    }

    #[test]
    fn test_leader_rotation_single_validator() {
        // Single validator is always leader
        assert_eq!(leader_index(0, 1), 0);
        assert_eq!(leader_index(1, 1), 0);
        assert_eq!(leader_index(100, 1), 0);
    }

    #[test]
    fn test_is_leader() {
        // Validator 0 is leader at heights 0, 3, 6...
        assert!(is_leader(0, 0, 3));
        assert!(!is_leader(0, 1, 3));
        assert!(!is_leader(0, 2, 3));
        assert!(is_leader(0, 3, 3));

        // Validator 1 is leader at heights 1, 4, 7...
        assert!(!is_leader(1, 0, 3));
        assert!(is_leader(1, 1, 3));
        assert!(!is_leader(1, 2, 3));
        assert!(!is_leader(1, 3, 3));
        assert!(is_leader(1, 4, 3));
    }

    #[test]
    fn test_is_leader_two_validators() {
        // Two validators alternate
        assert!(is_leader(0, 0, 2));
        assert!(is_leader(1, 1, 2));
        assert!(is_leader(0, 2, 2));
        assert!(is_leader(1, 3, 2));
    }
}
