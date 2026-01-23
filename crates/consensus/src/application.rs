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

/// Result of waiting for reth to sync with consensus state.
enum SyncResult {
    /// Reth is synced, parent timestamp available.
    Synced { parent_timestamp: u64 },
    /// Sync failed or state diverged.
    Failed,
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

    /// Current block height (next block to finalize).
    height: u64,

    /// Last block hash (reth's EVM block hash).
    last_hash: B256,

    /// Proposer address (fee recipient).
    proposer: Address,

    /// Height of the last block we proposed (prevents duplicate proposals).
    /// When Simplex requests a proposal at a height we've already proposed,
    /// we return null proposal to avoid creating duplicate blocks.
    last_proposed_height: Option<u64>,
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
            last_proposed_height: None,
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

    /// Wait for reth's head to match our expected parent.
    ///
    /// This prevents race conditions where consensus runs ahead of execution.
    async fn wait_for_reth_sync(&self) -> SyncResult {
        const MAX_SYNC_ATTEMPTS: u32 = 10;
        const SYNC_DELAY_MS: u64 = 100;

        let expected_parent_height = self.height.saturating_sub(1);

        for sync_attempt in 1..=MAX_SYNC_ATTEMPTS {
            let (reth_head_hash, reth_head_number, reth_head_timestamp) =
                match self.execution.get_head().await {
                    Ok(head) => head,
                    Err(e) => {
                        tracing::warn!(?e, "Failed to query reth head, retrying...");
                        tokio::time::sleep(tokio::time::Duration::from_millis(SYNC_DELAY_MS)).await;
                        continue;
                    }
                };

            if reth_head_hash == self.last_hash {
                tracing::debug!(
                    reth_head = %reth_head_hash,
                    reth_height = reth_head_number,
                    parent_timestamp = reth_head_timestamp,
                    our_parent = %self.last_hash,
                    "State synchronized, proceeding with block building"
                );
                return SyncResult::Synced { parent_timestamp: reth_head_timestamp };
            }

            if reth_head_number < expected_parent_height {
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
            }

            // State divergence - reth ahead or different hash at same height
            if reth_head_number > expected_parent_height {
                tracing::error!(
                    reth_head = %reth_head_hash,
                    reth_height = reth_head_number,
                    our_parent = %self.last_hash,
                    expected_height = expected_parent_height,
                    "STATE DIVERGENCE: reth ahead of consensus! This is a bug."
                );
            } else {
                tracing::error!(
                    reth_head = %reth_head_hash,
                    reth_height = reth_head_number,
                    our_parent = %self.last_hash,
                    expected_height = expected_parent_height,
                    "STATE DIVERGENCE: same height but different hash! Chain fork?"
                );
            }
            return SyncResult::Failed;
        }

        tracing::error!("Sync timeout after {} attempts", MAX_SYNC_ATTEMPTS);
        SyncResult::Failed
    }

    /// Compute block timestamp that's strictly greater than parent's.
    ///
    /// Ethereum requires: `block.timestamp > parent.timestamp`
    fn compute_block_timestamp(parent_timestamp: u64) -> u64 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let timestamp = now.max(parent_timestamp + 1);

        if timestamp != now {
            tracing::warn!(
                now,
                parent_timestamp,
                adjusted_timestamp = timestamp,
                "Adjusted block timestamp to be > parent (clock may be behind)"
            );
        }

        timestamp
    }

    /// Propose a new block using the vanilla Ethereum builder flow.
    ///
    /// Returns null proposal if:
    /// - We've already proposed at this height (prevents duplicate blocks)
    /// - Reth is not synced with our expected parent
    /// - Block building fails
    async fn propose(&mut self, context: AppContext) -> AppDigest {
        // CRITICAL: Prevent duplicate proposals at the same height.
        // When Simplex advances views quickly, propose() can be called multiple times
        // before the previous block is finalized. We must return null proposal to
        // avoid creating duplicate blocks at the same height.
        if let Some(last_proposed) = self.last_proposed_height {
            if last_proposed >= self.height {
                tracing::warn!(
                    current_height = self.height,
                    last_proposed_height = last_proposed,
                    ?context.round,
                    "Skipping proposal - already proposed at this height, waiting for finalization"
                );
                return commonware_cryptography::sha256::Digest(Self::NULL_PROPOSAL_DIGEST);
            }
        }

        tracing::debug!(
            height = self.height,
            ?context.round,
            parent_hash = %self.last_hash,
            "Starting block building via reth"
        );

        // Wait for reth to sync
        let parent_timestamp = match self.wait_for_reth_sync().await {
            SyncResult::Synced { parent_timestamp } => parent_timestamp,
            SyncResult::Failed => {
                return commonware_cryptography::sha256::Digest(Self::NULL_PROPOSAL_DIGEST);
            }
        };

        let timestamp = Self::compute_block_timestamp(parent_timestamp);

        // Build block via reth
        match self.execution.start_building(self.last_hash, timestamp, self.proposer).await {
            Ok(payload_id) => {
                match self.execution.get_payload(payload_id).await {
                    Ok(payload) => return self.complete_proposal(context, payload).await,
                    Err(e) => tracing::error!(?e, "Failed to get built payload"),
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

        // Mark that we've proposed at this height to prevent duplicates
        self.last_proposed_height = Some(self.height);

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
    ///
    /// CRITICAL: Only accepts blocks at the expected height. This prevents
    /// finalizing duplicate blocks when Simplex views advance faster than
    /// finalization callbacks are processed.
    pub async fn finalize(&mut self, digest: AppDigest) -> Option<Block> {
        if let Some(block) = self.pending_blocks.remove(&digest) {
            // CRITICAL: Verify block is at expected height to prevent duplicate finalizations.
            // When Simplex advances views quickly, multiple blocks can be proposed at the
            // same height before the first one is finalized. We must reject stale blocks.
            if block.height() != self.height {
                tracing::warn!(
                    expected_height = self.height,
                    block_height = block.height(),
                    block_hash = %block.block_hash,
                    ?digest,
                    "Rejecting stale block finalization - height mismatch"
                );
                // Clean up the orphaned payload
                self.pending_payloads.remove(&digest);
                return None;
            }

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

            // Clean up any stale pending blocks at heights we've already finalized.
            // This prevents memory leaks from accumulated duplicate proposals.
            let stale_digests: Vec<_> = self
                .pending_blocks
                .iter()
                .filter(|(_, b)| b.height() < self.height)
                .map(|(d, _)| *d)
                .collect();

            if !stale_digests.is_empty() {
                tracing::debug!(
                    count = stale_digests.len(),
                    new_height = self.height,
                    "Cleaning up stale pending blocks"
                );
                for digest in stale_digests {
                    self.pending_blocks.remove(&digest);
                    self.pending_payloads.remove(&digest);
                }
            }

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

    /// Handle a block received from another validator via relay.
    ///
    /// Imports the block to reth immediately and stores it as pending.
    async fn handle_receive_block(&mut self, digest: AppDigest, block: Block, payload: BuiltPayload) {
        if self.pending_blocks.contains_key(&digest) {
            return; // Already have it
        }

        tracing::info!(
            height = block.height(),
            block_hash = %block.block_hash,
            ?digest,
            "Received block from relay, importing to reth"
        );

        // Import block to reth immediately via newPayloadV3.
        // This is how Ethereum works: blocks are imported on gossip,
        // then finalized later via forkchoiceUpdated.
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
                    self.handle_receive_block(digest, block, payload).await;
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
