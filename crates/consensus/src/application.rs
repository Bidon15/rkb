//! Application trait implementation for commonware-consensus Simplex.
//!
//! This module provides the `CertifiableAutomaton` implementation that bridges
//! our block production logic with the commonware-consensus Simplex protocol.

use std::collections::HashMap;
use std::sync::Arc;

use alloy_primitives::{Address, B256};
use commonware_consensus::{
    simplex::types::Context, types::Epoch, Automaton as Au, CertifiableAutomaton as CAu,
    Relay as Re,
};
use commonware_cryptography::{ed25519::PublicKey, Hasher, Sha256};
use futures::channel::{mpsc, oneshot};
use sequencer_types::{Block, BlockHeader, Transaction};
use tokio::sync::{broadcast, RwLock};

/// Digest type used by our application.
pub type AppDigest = <Sha256 as Hasher>::Digest;

/// Context type for Simplex.
pub type AppContext = Context<AppDigest, PublicKey>;

/// Messages sent to the application actor.
#[allow(missing_docs)]
pub enum Message {
    /// Request genesis digest for an epoch.
    Genesis {
        epoch: Epoch,
        response: oneshot::Sender<AppDigest>,
    },
    /// Request to propose a new block.
    Propose {
        context: AppContext,
        response: oneshot::Sender<AppDigest>,
    },
    /// Request to verify a proposed block.
    Verify {
        context: AppContext,
        payload: AppDigest,
        response: oneshot::Sender<bool>,
    },
    /// Broadcast a payload to the network.
    Broadcast {
        payload: AppDigest,
    },
}

/// Mailbox for communicating with the application actor.
///
/// This implements the `Automaton`, `CertifiableAutomaton`, and `Relay` traits
/// required by the Simplex consensus protocol.
#[derive(Clone)]
pub struct Mailbox {
    sender: mpsc::Sender<Message>,
}

impl Mailbox {
    /// Create a new mailbox with the given sender.
    pub const fn new(sender: mpsc::Sender<Message>) -> Self {
        Self { sender }
    }
}

impl Au for Mailbox {
    type Digest = AppDigest;
    type Context = AppContext;

    async fn genesis(&mut self, epoch: Epoch) -> Self::Digest {
        let (response, receiver) = oneshot::channel();
        let _ = self
            .sender
            .try_send(Message::Genesis { epoch, response });
        receiver.await.expect("Failed to receive genesis")
    }

    async fn propose(&mut self, context: Self::Context) -> oneshot::Receiver<Self::Digest> {
        let (response, receiver) = oneshot::channel();
        let _ = self
            .sender
            .try_send(Message::Propose { context, response });
        receiver
    }

    async fn verify(
        &mut self,
        context: Self::Context,
        payload: Self::Digest,
    ) -> oneshot::Receiver<bool> {
        let (response, receiver) = oneshot::channel();
        let _ = self.sender.try_send(Message::Verify {
            context,
            payload,
            response,
        });
        receiver
    }
}

impl CAu for Mailbox {
    // Use default certify implementation which always returns true
}

impl Re for Mailbox {
    type Digest = AppDigest;

    async fn broadcast(&mut self, payload: Self::Digest) {
        let _ = self.sender.try_send(Message::Broadcast { payload });
    }
}

/// Configuration for the application.
pub struct ApplicationConfig {
    /// Initial block height.
    pub initial_height: u64,

    /// Proposer address (our validator address).
    pub proposer: Address,

    /// Mailbox size for message buffering.
    pub mailbox_size: usize,
}

impl Default for ApplicationConfig {
    fn default() -> Self {
        Self {
            initial_height: 1,
            proposer: Address::ZERO,
            mailbox_size: 1024,
        }
    }
}

/// Application state for block production.
///
/// This manages the mempool, pending blocks, and coordinates with Simplex consensus.
pub struct Application {
    /// Hasher for computing digests.
    hasher: Sha256,

    /// Pending transactions waiting to be included in a block.
    mempool: Arc<RwLock<Vec<Transaction>>>,

    /// Pending blocks that have been proposed but not yet finalized.
    pending_blocks: HashMap<AppDigest, Block>,

    /// Verified block digests.
    verified: std::collections::HashSet<AppDigest>,

    /// Mailbox receiver for incoming messages.
    mailbox: mpsc::Receiver<Message>,

    /// Broadcast sender for finalized blocks.
    block_sender: broadcast::Sender<Block>,

    /// Current block height.
    height: u64,

    /// Last block hash.
    last_hash: B256,

    /// Proposer address.
    proposer: Address,
}

impl Application {
    /// Create a new application with the given configuration.
    ///
    /// Returns the application actor and its mailbox for communication.
    pub fn new(
        config: ApplicationConfig,
        mempool: Arc<RwLock<Vec<Transaction>>>,
    ) -> (Self, Mailbox, broadcast::Receiver<Block>) {
        let (sender, receiver) = mpsc::channel(config.mailbox_size);
        let (block_sender, block_receiver) = broadcast::channel(100);

        let app = Self {
            hasher: Sha256::default(),
            mempool,
            pending_blocks: HashMap::new(),
            verified: std::collections::HashSet::new(),
            mailbox: receiver,
            block_sender,
            height: config.initial_height,
            last_hash: B256::ZERO,
            proposer: config.proposer,
        };

        (app, Mailbox::new(sender), block_receiver)
    }

    /// Get a reference to the mempool.
    pub fn mempool(&self) -> &Arc<RwLock<Vec<Transaction>>> {
        &self.mempool
    }

    /// Generate genesis digest for the given epoch.
    fn genesis(&mut self, epoch: Epoch) -> AppDigest {
        let genesis_data = format!("genesis:{epoch}");
        self.hasher.update(genesis_data.as_bytes());
        let digest = self.hasher.finalize();
        self.verified.insert(digest);
        digest
    }

    /// Propose a new block.
    async fn propose(&mut self, context: AppContext) -> AppDigest {
        // Get transactions from mempool
        let transactions: Vec<Transaction> = self.mempool.write().await.drain(..).collect();

        // Build block header
        let header = BlockHeader {
            height: self.height,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            parent_hash: self.last_hash,
            proposer: self.proposer,
        };

        let block = Block::new(header, transactions);
        let block_hash = block.hash();

        // Compute digest from block hash
        self.hasher.update(block_hash.as_slice());
        let digest = self.hasher.finalize();

        // Store pending block
        self.pending_blocks.insert(digest, block);
        self.verified.insert(digest);

        tracing::debug!(
            height = self.height,
            ?context.round,
            %block_hash,
            "Proposed block"
        );

        digest
    }

    /// Verify a proposed block.
    fn verify(&mut self, _context: AppContext, payload: AppDigest) -> bool {
        // Accept blocks we've proposed or already verified
        if self.pending_blocks.contains_key(&payload) || self.verified.contains(&payload) {
            self.verified.insert(payload);
            return true;
        }

        // For now, optimistically accept unknown blocks
        // In production, we'd fetch the block from the relay
        self.verified.insert(payload);
        true
    }

    /// Handle a broadcast request.
    fn broadcast(&mut self, payload: AppDigest) {
        tracing::debug!(?payload, "Broadcasting block digest");
        // The actual block content is propagated via the block relay channel
    }

    /// Finalize a block and update state.
    pub fn finalize(&mut self, digest: AppDigest) -> Option<Block> {
        if let Some(block) = self.pending_blocks.remove(&digest) {
            let block_hash = block.hash();
            self.height = block.height() + 1;
            self.last_hash = block_hash;

            // Broadcast finalized block to subscribers
            if self.block_sender.send(block.clone()).is_err() {
                tracing::debug!("No block subscribers");
            }

            tracing::info!(
                height = block.height(),
                %block_hash,
                tx_count = block.tx_count(),
                "Block finalized"
            );

            Some(block)
        } else {
            tracing::warn!(?digest, "Attempted to finalize unknown block");
            None
        }
    }

    /// Run the application actor loop.
    pub async fn run(mut self) {
        use futures::StreamExt;

        tracing::info!("Application actor started");

        while let Some(message) = self.mailbox.next().await {
            match message {
                Message::Genesis { epoch, response } => {
                    let digest = self.genesis(epoch);
                    let _ = response.send(digest);
                }
                Message::Propose { context, response } => {
                    let digest = self.propose(context).await;
                    let _ = response.send(digest);
                }
                Message::Verify {
                    context,
                    payload,
                    response,
                } => {
                    let valid = self.verify(context, payload);
                    let _ = response.send(valid);
                }
                Message::Broadcast { payload } => {
                    self.broadcast(payload);
                }
            }
        }

        tracing::info!("Application actor stopped");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_application_genesis() {
        let mempool = Arc::new(RwLock::new(Vec::new()));
        let config = ApplicationConfig::default();
        let (mut app, _mailbox, _rx) = Application::new(config, mempool);

        let digest1 = app.genesis(Epoch::zero());
        let digest2 = app.genesis(Epoch::zero());

        // Same epoch should produce same genesis
        assert_eq!(digest1, digest2);
    }
}
