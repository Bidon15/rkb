//! Blocker implementation for peer management.
//!
//! This module provides peer blocking functionality for the consensus network.

use commonware_cryptography::ed25519;
use commonware_p2p::Blocker;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::RwLock;

/// A simple blocker that tracks blocked peers.
///
/// In production, this would integrate with the P2P network layer to
/// actually disconnect and prevent connections from blocked peers.
#[derive(Clone, Default)]
pub struct ConsensusBlocker {
    /// Set of blocked peer public keys.
    blocked: Arc<RwLock<HashSet<Vec<u8>>>>,
}

impl ConsensusBlocker {
    /// Create a new blocker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if a peer is blocked.
    pub async fn is_blocked(&self, peer: &ed25519::PublicKey) -> bool {
        use commonware_codec::Write;
        let mut bytes = Vec::new();
        peer.write(&mut bytes);
        self.blocked.read().await.contains(&bytes)
    }
}

impl Blocker for ConsensusBlocker {
    type PublicKey = ed25519::PublicKey;

    async fn block(&mut self, peer: Self::PublicKey) {
        use commonware_codec::Write;
        let mut bytes = Vec::new();
        peer.write(&mut bytes);

        tracing::warn!(?peer, "Blocking peer due to misbehavior");
        self.blocked.write().await.insert(bytes);
    }
}
