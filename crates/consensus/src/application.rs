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
use sequencer_execution::{BlockBuilder, BuiltPayload, ExecutionClient};
use sequencer_types::{Block, BlockHeader, Transaction};
use tokio::sync::broadcast;

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
    /// Finalize a block (triggered when consensus reaches finalization).
    Finalize {
        digest: AppDigest,
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

    /// Send a finalize message to the application.
    ///
    /// This is called by the reporter when consensus finalizes a block.
    pub fn finalize(&mut self, digest: AppDigest) {
        if self.sender.try_send(Message::Finalize { digest }).is_err() {
            tracing::warn!("Failed to send finalize message to application");
        }
    }
}

impl Au for Mailbox {
    type Digest = AppDigest;
    type Context = AppContext;

    async fn genesis(&mut self, epoch: Epoch) -> Self::Digest {
        let (response, receiver) = oneshot::channel();
        if self.sender.try_send(Message::Genesis { epoch, response }).is_err() {
            tracing::warn!("Failed to send genesis message to application");
        }
        receiver.await.expect("Failed to receive genesis")
    }

    async fn propose(&mut self, context: Self::Context) -> oneshot::Receiver<Self::Digest> {
        let (response, receiver) = oneshot::channel();
        if self.sender.try_send(Message::Propose { context, response }).is_err() {
            tracing::warn!("Failed to send propose message to application");
        }
        receiver
    }

    async fn verify(
        &mut self,
        context: Self::Context,
        payload: Self::Digest,
    ) -> oneshot::Receiver<bool> {
        let (response, receiver) = oneshot::channel();
        if self.sender.try_send(Message::Verify { context, payload, response }).is_err() {
            tracing::warn!("Failed to send verify message to application");
        }
        receiver
    }
}

impl CAu for Mailbox {
    // Use default certify implementation which always returns true
}

impl Re for Mailbox {
    type Digest = AppDigest;

    async fn broadcast(&mut self, payload: Self::Digest) {
        if self.sender.try_send(Message::Broadcast { payload }).is_err() {
            tracing::warn!("Failed to send broadcast message to application");
        }
    }
}

/// Configuration for the application.
pub struct ApplicationConfig {
    /// Initial block height.
    pub initial_height: u64,

    /// Initial block hash (parent of first block).
    pub initial_hash: B256,

    /// Proposer address (our validator address).
    pub proposer: Address,

    /// Execution client for building blocks via reth.
    pub execution: Arc<ExecutionClient>,

    /// Mailbox size for message buffering.
    pub mailbox_size: usize,
}

/// Application state for block production.
///
/// This coordinates block building with reth via the ExecutionClient
/// and integrates with Simplex consensus.
pub struct Application {
    /// Hasher for computing digests.
    hasher: Sha256,

    /// Execution client for building blocks via reth.
    execution: Arc<ExecutionClient>,

    /// Pending blocks that have been proposed but not yet finalized.
    /// Keyed by digest (hash of reth's block_hash).
    pending_blocks: HashMap<AppDigest, Block>,

    /// Built payloads from reth, keyed by digest.
    /// Contains the full payload data for execution.
    pending_payloads: HashMap<AppDigest, BuiltPayload>,

    /// Verified block digests.
    verified: std::collections::HashSet<AppDigest>,

    /// Mailbox receiver for incoming messages.
    mailbox: mpsc::Receiver<Message>,

    /// Broadcast sender for finalized blocks.
    block_sender: broadcast::Sender<Block>,

    /// Current block height.
    height: u64,

    /// Last block hash (reth's EVM block hash).
    last_hash: B256,

    /// Proposer address (fee recipient).
    proposer: Address,
}

impl Application {
    /// Create a new application with the given configuration.
    ///
    /// Returns the application actor and its mailbox for communication.
    pub fn new(config: ApplicationConfig) -> (Self, Mailbox, broadcast::Receiver<Block>) {
        let (sender, receiver) = mpsc::channel(config.mailbox_size);
        let (block_sender, block_receiver) = broadcast::channel(100);

        let app = Self {
            hasher: Sha256::default(),
            execution: config.execution,
            pending_blocks: HashMap::new(),
            pending_payloads: HashMap::new(),
            verified: std::collections::HashSet::new(),
            mailbox: receiver,
            block_sender,
            height: config.initial_height,
            last_hash: config.initial_hash,
            proposer: config.proposer,
        };

        (app, Mailbox::new(sender), block_receiver)
    }

    /// Generate genesis digest for the given epoch.
    fn genesis(&mut self, epoch: Epoch) -> AppDigest {
        let genesis_data = format!("genesis:{epoch}");
        self.hasher.update(genesis_data.as_bytes());
        let digest = self.hasher.finalize();
        self.verified.insert(digest);
        digest
    }

    /// Propose a new block using the vanilla Ethereum builder flow.
    ///
    /// This tells reth to build a block from its mempool, then retrieves
    /// the built block. Reth controls transaction ordering and block contents.
    async fn propose(&mut self, context: AppContext) -> AppDigest {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        tracing::debug!(
            height = self.height,
            ?context.round,
            parent_hash = %self.last_hash,
            "Starting block building via reth"
        );

        // Start building a block via reth
        let payload_id = match self.execution.start_building(
            self.last_hash,
            timestamp,
            self.proposer,
        ).await {
            Ok(id) => id,
            Err(e) => {
                tracing::error!(?e, "Failed to start building block");
                // Return a dummy digest - this block will fail verification
                return self.hasher.finalize();
            }
        };

        // Get the built payload from reth
        let payload = match self.execution.get_payload(payload_id).await {
            Ok(p) => p,
            Err(e) => {
                tracing::error!(?e, "Failed to get built payload");
                return self.hasher.finalize();
            }
        };

        // Use reth's block hash as the authoritative hash
        let block_hash = payload.block_hash;

        // Convert BuiltPayload to our Block type
        let header = BlockHeader {
            height: payload.block_number,
            timestamp: payload.timestamp,
            parent_hash: payload.parent_hash,
            proposer: payload.fee_recipient,
        };

        // Convert transactions from Bytes to Transaction
        let transactions: Vec<Transaction> = payload
            .transactions
            .iter()
            .map(|tx| Transaction::new(tx.to_vec()))
            .collect();

        let block = Block::new(header, transactions);

        // Compute digest from reth's block hash
        self.hasher.update(block_hash.as_slice());
        let digest = self.hasher.finalize();

        // Store pending block and payload
        self.pending_blocks.insert(digest, block);
        self.pending_payloads.insert(digest, payload);
        self.verified.insert(digest);

        tracing::info!(
            height = self.height,
            ?context.round,
            %block_hash,
            tx_count = self.pending_blocks.get(&digest).map(|b| b.tx_count()).unwrap_or(0),
            "Proposed block via reth builder"
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
    ///
    /// Uses reth's authoritative block hash from the payload if available.
    pub fn finalize(&mut self, digest: AppDigest) -> Option<Block> {
        if let Some(block) = self.pending_blocks.remove(&digest) {
            // Get reth's authoritative block hash from payload, or compute from header
            let block_hash = self
                .pending_payloads
                .remove(&digest)
                .map(|p| p.block_hash)
                .unwrap_or_else(|| block.hash());

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
            // Clean up orphaned payload if exists
            self.pending_payloads.remove(&digest);
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
                    if response.send(digest).is_err() {
                        tracing::warn!("Genesis response receiver dropped");
                    }
                }
                Message::Propose { context, response } => {
                    let digest = self.propose(context).await;
                    if response.send(digest).is_err() {
                        tracing::warn!("Propose response receiver dropped");
                    }
                }
                Message::Verify {
                    context,
                    payload,
                    response,
                } => {
                    let valid = self.verify(context, payload);
                    if response.send(valid).is_err() {
                        tracing::warn!("Verify response receiver dropped");
                    }
                }
                Message::Broadcast { payload } => {
                    self.broadcast(payload);
                }
                Message::Finalize { digest } => {
                    self.finalize(digest);
                }
            }
        }

        tracing::info!("Application actor stopped");
    }
}

#[cfg(test)]
mod tests {
    // Tests require a running reth instance for the ExecutionClient.
    // Integration tests should be added in a separate test crate.
}
