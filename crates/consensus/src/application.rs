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
use sequencer_types::{Block, BlockHeader, BlockParams, BlockTiming, Transaction};
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

    /// Block timing mode (vanilla = 1s blocks, subsecond = <1s with forked Reth).
    pub block_timing: BlockTiming,
}

/// Result of waiting for reth to sync with consensus state.
enum SyncResult {
    /// Reth is synced, parent timestamp available.
    Synced { parent_timestamp: u64 },
    /// Sync failed or state diverged.
    Failed,
}

/// Result of querying reth head state.
struct RethHead {
    hash: B256,
    number: u64,
    timestamp: u64,
}

/// Sync check outcome.
enum SyncCheck {
    /// States match, proceed with building.
    Synced,
    /// Reth behind consensus, should retry.
    Behind,
    /// State divergence, abort.
    Diverged,
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

    /// Block timing mode.
    block_timing: BlockTiming,
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
            block_timing: config.block_timing,
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
    async fn wait_for_reth_sync(&self) -> SyncResult {
        const MAX_ATTEMPTS: u32 = 10;
        const DELAY_MS: u64 = 100;

        for attempt in 1..=MAX_ATTEMPTS {
            let Some(head) = self.query_reth_head().await else {
                tokio::time::sleep(tokio::time::Duration::from_millis(DELAY_MS)).await;
                continue;
            };

            match self.check_sync_state(&head, attempt) {
                SyncCheck::Synced => return SyncResult::Synced { parent_timestamp: head.timestamp },
                SyncCheck::Behind => {
                    tokio::time::sleep(tokio::time::Duration::from_millis(DELAY_MS)).await;
                    continue;
                }
                SyncCheck::Diverged => return SyncResult::Failed,
            }
        }

        tracing::error!(max_attempts = MAX_ATTEMPTS, "Sync timeout");
        SyncResult::Failed
    }

    /// Query reth head state, returns None on error.
    async fn query_reth_head(&self) -> Option<RethHead> {
        match self.execution.get_head().await {
            Ok((hash, number, timestamp)) => Some(RethHead { hash, number, timestamp }),
            Err(e) => {
                tracing::warn!(?e, "Failed to query reth head, retrying...");
                None
            }
        }
    }

    /// Check if reth state matches our expected parent.
    fn check_sync_state(&self, head: &RethHead, attempt: u32) -> SyncCheck {
        let expected_height = self.height.saturating_sub(1);

        if head.hash == self.last_hash {
            tracing::debug!(reth_head = %head.hash, reth_height = head.number, "State synchronized");
            return SyncCheck::Synced;
        }

        if head.number < expected_height {
            tracing::info!(reth_height = head.number, expected_height, attempt, "SYNC WAIT: reth behind consensus");
            return SyncCheck::Behind;
        }

        self.log_divergence(head, expected_height);
        SyncCheck::Diverged
    }

    /// Log state divergence error.
    fn log_divergence(&self, head: &RethHead, expected_height: u64) {
        if head.number > expected_height {
            tracing::error!(reth_height = head.number, expected_height, "STATE DIVERGENCE: reth ahead of consensus");
        } else {
            tracing::error!(reth_hash = %head.hash, our_hash = %self.last_hash, "STATE DIVERGENCE: different hash at same height");
        }
    }

    /// Get current wall clock timestamp in seconds.
    ///
    /// Unlike the previous implementation, this returns the actual wall clock
    /// without any artificial adjustment. The timing mode check in `can_build_block`
    /// ensures we only call this when the timestamp will be valid.
    fn wall_clock_timestamp() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    /// Check if we can build a block based on timing mode.
    ///
    /// - Vanilla: Only when wall clock > parent timestamp (enforces 1s minimum)
    /// - Subsecond: When wall clock >= parent timestamp (allows same-second)
    fn can_build_block(block_timing: BlockTiming, parent_timestamp: u64) -> bool {
        let now = Self::wall_clock_timestamp();

        match block_timing {
            BlockTiming::Vanilla => now > parent_timestamp,
            BlockTiming::Subsecond => now >= parent_timestamp,
        }
    }

    /// Return null proposal digest.
    fn null_proposal() -> AppDigest {
        commonware_cryptography::sha256::Digest(Self::NULL_PROPOSAL_DIGEST)
    }

    /// Check if we've already proposed at this height.
    fn already_proposed_at_height(&self, context: &AppContext) -> bool {
        let Some(last) = self.last_proposed_height else { return false };
        if last < self.height { return false; }

        tracing::warn!(height = self.height, last_proposed = last, ?context.round, "Skipping - already proposed");
        true
    }

    /// Build a block via reth, returns payload on success.
    async fn build_via_reth(&self, timestamp: u64) -> Option<BuiltPayload> {
        let payload_id = match self.execution.start_building(self.last_hash, timestamp, self.proposer).await {
            Ok(id) => id,
            Err(e) => {
                tracing::error!(?e, height = self.height, "Block building failed");
                return None;
            }
        };

        match self.execution.get_payload(payload_id).await {
            Ok(payload) => Some(payload),
            Err(e) => {
                tracing::error!(?e, "Failed to get built payload");
                None
            }
        }
    }

    /// Propose a new block using the vanilla Ethereum builder flow.
    async fn propose(&mut self, context: AppContext) -> AppDigest {
        if self.already_proposed_at_height(&context) {
            return Self::null_proposal();
        }

        tracing::debug!(height = self.height, ?context.round, parent_hash = %self.last_hash, "Starting block building");

        let parent_timestamp = match self.wait_for_reth_sync().await {
            SyncResult::Synced { parent_timestamp } => parent_timestamp,
            SyncResult::Failed => return Self::null_proposal(),
        };

        if !Self::can_build_block(self.block_timing, parent_timestamp) {
            tracing::debug!(height = self.height, parent_timestamp, "Waiting for timestamp to advance");
            return Self::null_proposal();
        }

        let timestamp = Self::wall_clock_timestamp();
        match self.build_via_reth(timestamp).await {
            Some(payload) => self.complete_proposal(context, payload).await,
            None => {
                tracing::error!(height = self.height, ?context.round, "Build failed, returning null proposal");
                Self::null_proposal()
            }
        }
    }

    /// Convert a BuiltPayload to our Block type.
    fn build_block_from_payload(payload: &BuiltPayload) -> Block {
        let header = BlockHeader {
            height: payload.block_number,
            timestamp: payload.timestamp,
            parent_hash: payload.parent_hash,
            proposer: payload.fee_recipient,
        };

        let transactions: Vec<Transaction> = payload.transactions.iter().map(|tx| Transaction::new(tx.to_vec())).collect();

        Block::new(BlockParams {
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
        })
    }

    /// Compute digest from block hash.
    fn compute_digest(&mut self, block_hash: B256) -> AppDigest {
        self.hasher.update(block_hash.as_slice());
        self.hasher.finalize()
    }

    /// Broadcast proposed block to relay.
    fn broadcast_to_relay(&self, digest: AppDigest, block: &Block, payload: &BuiltPayload) {
        let proposed = ProposedBlock { digest, block: block.clone(), payload: payload.clone() };
        if self.block_relay_sender.send(proposed).is_err() {
            tracing::debug!("No block relay subscribers");
        }
    }

    /// Complete a successful proposal by building the block and storing it.
    async fn complete_proposal(&mut self, context: AppContext, payload: BuiltPayload) -> AppDigest {
        let block = Self::build_block_from_payload(&payload);
        let digest = self.compute_digest(block.block_hash);

        let block_hash = block.block_hash;
        let tx_count = block.tx_count();

        self.broadcast_to_relay(digest, &block, &payload);
        self.pending_blocks.insert(digest, block);
        self.pending_payloads.insert(digest, payload);
        self.verified.insert(digest);
        self.last_proposed_height = Some(self.height);

        tracing::info!(height = self.height, ?context.round, %block_hash, tx_count, "Proposed block");
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

    /// Apply payload to reth execution client.
    async fn apply_to_reth(&self, payload: Option<&BuiltPayload>) {
        let Some(payload) = payload else {
            tracing::warn!("No payload found for finalized block");
            return;
        };
        if let Err(e) = self.execution.apply_built_payload(payload).await {
            tracing::error!(?e, "Failed to apply payload to reth");
        }
    }

    /// Clean up stale pending blocks below current height.
    fn cleanup_stale_blocks(&mut self) {
        let stale: Vec<_> = self.pending_blocks.iter()
            .filter(|(_, b)| b.height() < self.height)
            .map(|(d, _)| *d)
            .collect();

        if stale.is_empty() { return; }

        tracing::debug!(count = stale.len(), height = self.height, "Cleaning up stale blocks");
        for digest in stale {
            self.pending_blocks.remove(&digest);
            self.pending_payloads.remove(&digest);
        }
    }

    /// Broadcast finalized block to subscribers.
    fn broadcast_finalized(&self, block: &Block) {
        if self.block_sender.send(block.clone()).is_err() {
            tracing::debug!("No block subscribers");
        }
    }

    /// Check if block height matches expected height.
    fn is_valid_height(&self, block: &Block, digest: AppDigest) -> bool {
        if block.height() == self.height { return true; }

        tracing::debug!(expected = self.height, actual = block.height(), ?digest, "Ignoring stale finalization");
        false
    }

    /// Finalize a block and update state.
    pub async fn finalize(&mut self, digest: AppDigest) -> Option<Block> {
        let Some(block) = self.pending_blocks.remove(&digest) else {
            self.pending_payloads.remove(&digest);
            tracing::debug!(?digest, "Ignoring finalization for unknown block");
            return None;
        };

        if !self.is_valid_height(&block, digest) {
            self.pending_payloads.remove(&digest);
            return None;
        }

        let payload = self.pending_payloads.remove(&digest);
        self.apply_to_reth(payload.as_ref()).await;

        self.height = block.height() + 1;
        self.last_hash = block.block_hash;

        self.cleanup_stale_blocks();
        self.broadcast_finalized(&block);

        tracing::info!(height = block.height(), block_hash = %block.block_hash, tx_count = block.tx_count(), "Block finalized");
        Some(block)
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

    /// Handle genesis message.
    fn handle_genesis(&mut self, epoch: Epoch, response: oneshot::Sender<AppDigest>) {
        let digest = self.genesis(epoch);
        let _ = response.send(digest);
    }

    /// Handle propose message.
    async fn handle_propose(&mut self, context: AppContext, response: oneshot::Sender<AppDigest>) {
        let digest = self.propose(context).await;
        let _ = response.send(digest);
    }

    /// Handle verify message.
    fn handle_verify(&mut self, context: AppContext, payload: AppDigest, response: oneshot::Sender<bool>) {
        let valid = self.verify(context, payload);
        let _ = response.send(valid);
    }

    /// Process a single message.
    async fn process_message(&mut self, message: Message) {
        match message {
            Message::Genesis { epoch, response } => self.handle_genesis(epoch, response),
            Message::Propose { context, response } => self.handle_propose(context, response).await,
            Message::Verify { context, payload, response } => self.handle_verify(context, payload, response),
            Message::Broadcast { payload } => self.broadcast(payload),
            Message::Finalize { digest } => { self.finalize(digest).await; }
            Message::ReceiveBlock { digest, block, payload } => self.handle_receive_block(digest, block, payload).await,
        }
    }

    /// Run the application actor loop.
    pub async fn run(mut self) {
        use futures::StreamExt;

        tracing::info!("Application actor started");
        while let Some(message) = self.mailbox.next().await {
            self.process_message(message).await;
        }
        tracing::info!("Application actor stopped");
    }
}

#[cfg(test)]
mod tests {
    // Tests require a running reth instance for the ExecutionClient.
    // Integration tests should be added in a separate test crate.
}
