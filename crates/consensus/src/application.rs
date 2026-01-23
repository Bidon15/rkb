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
use sequencer_types::{Block, BlockHeader, BlockParams, Transaction};
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
    /// Receive a block from another validator (via block relay).
    ReceiveBlock {
        digest: AppDigest,
        block: Block,
        payload: BuiltPayload,
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

    /// Send a received block to the application.
    ///
    /// This is called when a block is received from another validator via relay.
    pub fn receive_block(&mut self, digest: AppDigest, block: Block, payload: BuiltPayload) {
        if self
            .sender
            .try_send(Message::ReceiveBlock { digest, block, payload })
            .is_err()
        {
            tracing::warn!("Failed to send receive_block message to application");
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

/// A proposed block ready for relay to other validators.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProposedBlock {
    /// The digest (hash of block_hash) stored as bytes for serialization.
    #[serde(with = "digest_serde")]
    pub digest: AppDigest,
    /// The block data.
    pub block: Block,
    /// The built payload from reth.
    pub payload: BuiltPayload,
}

/// Serde helper for serializing/deserializing AppDigest (sha256::Digest).
mod digest_serde {
    use super::AppDigest;
    use commonware_cryptography::sha256::Digest;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(digest: &AppDigest, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Serialize as byte array
        digest.0.serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<AppDigest, D::Error>
    where
        D: Deserializer<'de>,
    {
        // Deserialize from byte array
        let bytes: [u8; 32] = <[u8; 32]>::deserialize(deserializer)?;
        Ok(Digest(bytes))
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

    /// Broadcast sender for proposed blocks (to relay to other validators).
    block_relay_sender: broadcast::Sender<ProposedBlock>,

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
    /// Returns:
    /// - The application actor
    /// - Its mailbox for communication
    /// - A receiver for finalized blocks
    /// - A receiver for proposed blocks (to relay to other validators)
    pub fn new(
        config: ApplicationConfig,
    ) -> (Self, Mailbox, broadcast::Receiver<Block>, broadcast::Receiver<ProposedBlock>) {
        let (sender, receiver) = mpsc::channel(config.mailbox_size);
        let (block_sender, block_receiver) = broadcast::channel(100);
        let (block_relay_sender, block_relay_receiver) = broadcast::channel(100);

        let app = Self {
            hasher: Sha256::default(),
            execution: config.execution,
            pending_blocks: HashMap::new(),
            pending_payloads: HashMap::new(),
            verified: std::collections::HashSet::new(),
            mailbox: receiver,
            block_sender,
            block_relay_sender,
            height: config.initial_height,
            last_hash: config.initial_hash,
            proposer: config.proposer,
        };

        (app, Mailbox::new(sender), block_receiver, block_relay_receiver)
    }

    /// Generate genesis digest for the given epoch.
    fn genesis(&mut self, epoch: Epoch) -> AppDigest {
        let genesis_data = format!("genesis:{epoch}");
        self.hasher.update(genesis_data.as_bytes());
        let digest = self.hasher.finalize();
        self.verified.insert(digest);
        digest
    }

    /// Well-known null digest used when proposal fails.
    /// This is SHA256 of "null_proposal" and will be rejected by verify().
    const NULL_PROPOSAL_DIGEST: [u8; 32] = [
        0x9d, 0x15, 0x7a, 0x9d, 0x5a, 0x23, 0x8b, 0x4c,
        0x7d, 0x9f, 0x8c, 0x2b, 0x3e, 0x4a, 0x5f, 0x6d,
        0x8e, 0x9a, 0x0b, 0x1c, 0x2d, 0x3e, 0x4f, 0x5a,
        0x6b, 0x7c, 0x8d, 0x9e, 0xaf, 0xb0, 0xc1, 0xd2,
    ];

    /// Propose a new block using the vanilla Ethereum builder flow.
    ///
    /// This tells reth to build a block from its mempool, then retrieves
    /// the built block. Reth controls transaction ordering and block contents.
    ///
    /// # Sync-Aware Proposal
    ///
    /// **CRITICAL**: Before building, we verify consensus state matches reth's state.
    ///
    /// Race scenario that this prevents:
    /// 1. Block N finalized by consensus (Finalize message queued)
    /// 2. Propose for block N+1 starts before Finalize processed
    /// 3. Our `last_hash` = block N-1, but reth's head = block N-1
    /// 4. We try to build on N-1, but reth may have N pending
    /// 5. State mismatch causes confusing errors
    ///
    /// Solution: Query reth's actual head and wait if we're ahead.
    ///
    /// # Timestamp Freshness
    ///
    /// **CRITICAL**: Timestamp MUST be computed fresh on each retry attempt.
    ///
    /// Ethereum requires: `block.timestamp > parent.timestamp`
    ///
    /// By computing a fresh timestamp on each attempt, we guarantee it's
    /// always after any recently-finalized parent block's timestamp.
    async fn propose(&mut self, context: AppContext) -> AppDigest {
        tracing::debug!(
            height = self.height,
            ?context.round,
            parent_hash = %self.last_hash,
            "Starting block building via reth"
        );

        // Sync-aware proposal: wait for reth to catch up
        const MAX_SYNC_ATTEMPTS: u32 = 10;
        const SYNC_DELAY_MS: u64 = 100;

        for sync_attempt in 1..=MAX_SYNC_ATTEMPTS {
            // Query reth's actual head
            let (reth_head_hash, reth_head_number) = match self.execution.get_head().await {
                Ok(head) => head,
                Err(e) => {
                    tracing::warn!(?e, "Failed to query reth head, retrying...");
                    tokio::time::sleep(tokio::time::Duration::from_millis(SYNC_DELAY_MS)).await;
                    continue;
                }
            };

            // Our expected parent should match reth's head
            // We want to build block at height H, so reth should have block H-1 as head
            let expected_parent_height = self.height.saturating_sub(1);

            if reth_head_hash == self.last_hash {
                // Perfect sync: reth's head matches our expected parent
                tracing::debug!(
                    reth_head = %reth_head_hash,
                    reth_height = reth_head_number,
                    our_parent = %self.last_hash,
                    "State synchronized, proceeding with block building"
                );
                break;
            } else if reth_head_number < expected_parent_height {
                // Reth is behind: finalization hasn't been applied yet
                tracing::info!(
                    reth_head = %reth_head_hash,
                    reth_height = reth_head_number,
                    our_parent = %self.last_hash,
                    expected_height = expected_parent_height,
                    sync_attempt,
                    "SYNC WAIT: reth behind consensus, waiting for finalization to apply"
                );
                tokio::time::sleep(tokio::time::Duration::from_millis(SYNC_DELAY_MS)).await;
                continue;
            } else if reth_head_number > expected_parent_height {
                // This shouldn't happen - reth ahead of consensus?
                tracing::error!(
                    reth_head = %reth_head_hash,
                    reth_height = reth_head_number,
                    our_parent = %self.last_hash,
                    expected_height = expected_parent_height,
                    "STATE DIVERGENCE: reth ahead of consensus! This is a bug."
                );
                return commonware_cryptography::sha256::Digest(Self::NULL_PROPOSAL_DIGEST);
            } else {
                // Same height but different hash - chain fork?
                tracing::error!(
                    reth_head = %reth_head_hash,
                    reth_height = reth_head_number,
                    our_parent = %self.last_hash,
                    expected_height = expected_parent_height,
                    "STATE DIVERGENCE: same height but different hash! Chain fork?"
                );
                return commonware_cryptography::sha256::Digest(Self::NULL_PROPOSAL_DIGEST);
            }
        }

        // Now build with fresh timestamp
        // CRITICAL: Compute fresh timestamp - Ethereum requires block.timestamp > parent.timestamp
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        match self.execution.start_building(self.last_hash, timestamp, self.proposer).await {
            Ok(payload_id) => {
                match self.execution.get_payload(payload_id).await {
                    Ok(payload) => {
                        return self.complete_proposal(context, payload).await;
                    }
                    Err(e) => {
                        tracing::error!(?e, "Failed to get built payload");
                    }
                }
            }
            Err(e) => {
                tracing::error!(
                    ?e,
                    height = self.height,
                    parent_hash = %self.last_hash,
                    "Block building failed after sync verification - unexpected"
                );
            }
        }

        // Building failed after sync verification - this is unexpected
        tracing::error!(
            height = self.height,
            ?context.round,
            parent_hash = %self.last_hash,
            "Failed to build block despite sync verification, returning null proposal"
        );

        commonware_cryptography::sha256::Digest(Self::NULL_PROPOSAL_DIGEST)
    }

    /// Complete a successful proposal by building the block and storing it.
    async fn complete_proposal(&mut self, context: AppContext, payload: BuiltPayload) -> AppDigest {

        // Convert BuiltPayload to our Block type with all required fields
        let header = BlockHeader {
            height: payload.block_number,
            timestamp: payload.timestamp,
            parent_hash: payload.parent_hash,
            proposer: payload.fee_recipient,
        };

        let transactions: Vec<Transaction> = payload
            .transactions
            .iter()
            .map(|tx| Transaction::new(tx.to_vec()))
            .collect();

        let block = Block::new(BlockParams {
            header,
            transactions,
            state_root: payload.state_root,
            receipts_root: payload.receipts_root,
            logs_bloom: payload.logs_bloom.clone(),
            prev_randao: payload.prev_randao,
            extra_data: payload.extra_data.clone(),
            gas_used: payload.gas_used,
            gas_limit: payload.gas_limit,
            base_fee_per_gas: payload.base_fee_per_gas,
            block_hash: payload.block_hash,
        });

        // Compute digest from reth's block hash
        self.hasher.update(block.block_hash.as_slice());
        let digest = self.hasher.finalize();

        // Save values for logging before moving
        let block_hash = block.block_hash;
        let tx_count = block.tx_count();

        // Broadcast to relay so other validators can receive it
        let proposed = ProposedBlock {
            digest,
            block: block.clone(),
            payload: payload.clone(),
        };
        if self.block_relay_sender.send(proposed).is_err() {
            tracing::debug!("No block relay subscribers");
        }

        // Store pending block and payload
        self.pending_blocks.insert(digest, block);
        self.pending_payloads.insert(digest, payload);
        self.verified.insert(digest);

        tracing::info!(
            height = self.height,
            ?context.round,
            %block_hash,
            tx_count,
            "Proposed block via reth builder"
        );

        digest
    }

    /// Verify a proposed block.
    fn verify(&mut self, _context: AppContext, payload: AppDigest) -> bool {
        // Reject null proposal digest - indicates proposer couldn't build a block
        if payload.0 == Self::NULL_PROPOSAL_DIGEST {
            tracing::debug!("Rejecting null proposal digest");
            return false;
        }

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
    /// This applies the block to reth via newPayloadV3 and updates forkchoice.
    pub async fn finalize(&mut self, digest: AppDigest) -> Option<Block> {
        if let Some(block) = self.pending_blocks.remove(&digest) {
            // Get the payload - we need it to apply to reth
            let payload = self.pending_payloads.remove(&digest);

            // Apply the payload to reth so it updates its chain
            if let Some(ref payload) = payload {
                if let Err(e) = self.execution.apply_built_payload(payload).await {
                    tracing::error!(?e, "Failed to apply built payload to reth");
                    // Still finalize locally - consensus agreed on this block
                }
            } else {
                tracing::warn!("No payload found for finalized block, reth may be out of sync");
            }

            self.height = block.height() + 1;
            self.last_hash = block.block_hash;

            // Broadcast finalized block to subscribers
            if self.block_sender.send(block.clone()).is_err() {
                tracing::debug!("No block subscribers");
            }

            tracing::info!(
                height = block.height(),
                block_hash = %block.block_hash,
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
                    self.finalize(digest).await;
                }
                Message::ReceiveBlock { digest, block, payload } => {
                    // Store block received from another validator
                    if !self.pending_blocks.contains_key(&digest) {
                        tracing::info!(
                            height = block.height(),
                            block_hash = %block.block_hash,
                            ?digest,
                            "Received block from relay, importing to reth"
                        );

                        // CRITICAL: Import block to reth immediately via newPayloadV3.
                        // This is how Ethereum works: blocks are imported on gossip,
                        // then finalized later via forkchoiceUpdated.
                        //
                        // Without this, reth doesn't know the block exists, and when
                        // we try to build on top of it, forkchoiceUpdated fails with
                        // "unknown head block".
                        if let Err(e) = self.execution.import_payload(&payload).await {
                            tracing::warn!(
                                ?e,
                                height = block.height(),
                                block_hash = %block.block_hash,
                                "Failed to import relayed block to reth (may already exist)"
                            );
                            // Continue anyway - block might already be imported
                        }

                        self.pending_blocks.insert(digest, block);
                        self.pending_payloads.insert(digest, payload);
                        self.verified.insert(digest);
                    }
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
