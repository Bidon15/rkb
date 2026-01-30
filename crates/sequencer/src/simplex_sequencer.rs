//! Simplex BFT sequencer.
//!
//! Uses commonware-consensus Simplex protocol for multi-validator BFT consensus.
//! Blocks are batched and submitted to Celestia at configurable intervals.

use std::collections::HashSet;
use std::net::{SocketAddr, ToSocketAddrs};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use alloy_primitives::Address;
use commonware_cryptography::{ed25519, Signer};
use eyre::{Context, Result};
use tokio::sync::{mpsc, RwLock};
use alloy_primitives::B256;
use tracing::{error, info, warn};

use sequencer_celestia::{BlockIndex, CelestiaClient, DataAvailability, FinalityConfirmation, IndexEntry};
use sequencer_consensus::{on_finalized, start_simplex_runtime, OnFinalized, SimplexRuntimeConfig};
use sequencer_execution::{Execution, ExecutionClient, ForkchoiceState};
use sequencer_types::{Block, ConsensusConfig};

use crate::config::Config;

/// Simplex BFT sequencer node.
///
/// Uses multi-validator BFT consensus with leader-only Celestia submission.
pub struct SimplexSequencer {
    config: Config,
}

/// Components needed to run the sequencer.
struct SequencerComponents {
    celestia: Arc<CelestiaClient>,
    execution: Arc<ExecutionClient>,
    forkchoice: Arc<RwLock<ForkchoiceState>>,
    executed_blocks: Arc<RwLock<HashSet<B256>>>,
    finality_rx: mpsc::Receiver<FinalityConfirmation>,
}

/// Context for syncing operations (reduces parameter passing).
struct SyncContext<'a> {
    execution: &'a ExecutionClient,
    forkchoice: &'a RwLock<ForkchoiceState>,
    executed_blocks: &'a RwLock<HashSet<B256>>,
    index: &'a mut BlockIndex,
}

/// Context for block finalization (reduces Arc clone boilerplate).
#[derive(Clone)]
struct FinalizationContext {
    execution: Arc<ExecutionClient>,
    forkchoice: Arc<RwLock<ForkchoiceState>>,
    celestia: Arc<CelestiaClient>,
    executed_blocks: Arc<RwLock<HashSet<B256>>>,
    batch_tx: Option<mpsc::Sender<Block>>,
    batch_enabled: bool,
}

/// Mutable state for batch submission loop.
struct BatchState {
    pending_blocks: Vec<Block>,
    pending_size: usize,
}

impl BatchState {
    fn new() -> Self {
        Self {
            pending_blocks: Vec::with_capacity(64),
            pending_size: 0,
        }
    }

    fn is_empty(&self) -> bool {
        self.pending_blocks.is_empty()
    }
}

/// Action to take after processing a batch event.
enum BatchAction {
    /// Continue waiting for more blocks.
    Continue,
    /// Submit the current batch.
    Submit,
    /// Submit remaining blocks and exit.
    Shutdown,
}

impl SimplexSequencer {
    /// Create a new Simplex sequencer.
    pub fn new(config: Config) -> Self {
        Self { config }
    }

    /// Initialize all sequencer components.
    async fn init_components(&self) -> Result<SequencerComponents> {
        let (celestia, finality_rx) = self.init_celestia().await?;
        let execution = self.init_execution()?;

        // Get genesis hash from reth and initialize forkchoice
        let genesis_hash = execution
            .get_genesis_hash()
            .await
            .wrap_err("failed to get genesis hash from reth")?;
        execution
            .init_forkchoice(genesis_hash)
            .await
            .wrap_err("failed to initialize forkchoice on reth")?;

        let forkchoice = Arc::new(RwLock::new(ForkchoiceState {
            head: genesis_hash,
            safe: genesis_hash,
            finalized: genesis_hash,
        }));

        let executed_blocks = Arc::new(RwLock::new(HashSet::new()));

        Ok(SequencerComponents {
            celestia,
            execution,
            forkchoice,
            executed_blocks,
            finality_rx,
        })
    }

    /// Run the Simplex BFT sequencer.
    ///
    /// This starts:
    /// 1. P2P networking for validator communication
    /// 2. Simplex BFT consensus engine
    /// 3. Execution layer (reth) integration
    /// 4. Celestia finality tracker
    /// 5. Batch submission to Celestia at configured intervals
    ///
    /// If `genesis_da_height` is configured, the sequencer will first sync
    /// historical blocks from Celestia DA before starting consensus.
    ///
    /// # Errors
    ///
    /// Returns an error if any component fails to start.
    pub async fn run(self) -> Result<()> {
        info!(chain_id = self.config.chain_id, "Simplex sequencer starting");

        let components = self.init_components().await?;
        let genesis_hash = components.forkchoice.read().await.head;
        let runtime_config = build_runtime_config(&self.config.consensus, genesis_hash)?;

        // Sync from Celestia DA if configured
        let genesis_da_height = self.config.celestia.genesis_da_height;
        if genesis_da_height > 0 {
            let mut block_index = BlockIndex::open(&self.config.celestia.index_path)
                .await
                .wrap_err("failed to open block index")?;

            info!(genesis_da_height, "Syncing historical blocks from Celestia DA");
            let synced_height = Self::sync_from_celestia(
                &components.celestia,
                &components.execution,
                &components.forkchoice,
                &components.executed_blocks,
                &mut block_index,
                genesis_da_height,
            )
            .await
            .wrap_err("failed to sync from Celestia")?;

            info!(synced_height, "Celestia sync complete, starting consensus");
        }

        info!(
            validators = runtime_config.validators.len(),
            listen_addr = %runtime_config.listen_addr,
            "Starting Simplex BFT consensus"
        );

        // Spawn batch submission task and create finalization callback
        let batch_tx = self.spawn_batch_task(&components.celestia);
        let callback = Self::create_finalization_callback(
            Arc::clone(&components.execution),
            Arc::clone(&components.forkchoice),
            Arc::clone(&components.celestia),
            Arc::clone(&components.executed_blocks),
            batch_tx,
            self.config.celestia.batch_interval_ms > 0,
        );

        // Spawn consensus runtime in separate thread (it blocks)
        let execution_for_consensus = Arc::clone(&components.execution);
        let runtime_handle = std::thread::spawn(move || {
            if let Err(e) = start_simplex_runtime(runtime_config, execution_for_consensus, callback) {
                error!(error = %e, "Simplex runtime failed");
            }
        });

        // Run main event loop
        self.run_event_loop(
            components.finality_rx,
            components.execution,
            components.forkchoice,
            components.executed_blocks,
        )
        .await;

        drop(runtime_handle);
        info!("Simplex sequencer shutting down");

        Ok(())
    }

    /// Initialize the Celestia client and finality tracker.
    async fn init_celestia(
        &self,
    ) -> Result<(Arc<CelestiaClient>, mpsc::Receiver<FinalityConfirmation>)> {
        let (finality_tx, finality_rx) = mpsc::channel::<FinalityConfirmation>(100);

        let celestia = Arc::new(
            CelestiaClient::new(self.config.celestia.clone(), finality_tx)
                .await
                .wrap_err("failed to create Celestia client")?,
        );

        // Start finality tracker in background
        let celestia_finality = Arc::clone(&celestia);
        tokio::spawn(async move {
            if let Err(e) = celestia_finality.run_finality_tracker().await {
                error!(error = %e, "Celestia finality tracker failed");
            }
        });
        info!("Celestia finality tracker started");

        Ok((celestia, finality_rx))
    }

    /// Initialize the execution client.
    fn init_execution(&self) -> Result<Arc<ExecutionClient>> {
        let execution = Arc::new(
            ExecutionClient::new(self.config.execution.clone())
                .wrap_err("failed to create execution client")?,
        );
        info!(reth_url = %self.config.execution.reth_url, "Execution client initialized");
        Ok(execution)
    }

    /// Fetch blocks from Celestia and sort by height.
    async fn fetch_blocks_from_celestia(
        celestia: &CelestiaClient,
        start_height: u64,
        end_height: u64,
    ) -> Result<Vec<(u64, Block)>> {
        let blocks_with_heights = celestia
            .get_blocks_in_range(start_height, end_height)
            .await
            .wrap_err("failed to fetch blocks from Celestia")?;

        if blocks_with_heights.is_empty() {
            return Ok(Vec::new());
        }

        // Sort by block height to ensure proper execution order
        let mut sorted_blocks: Vec<_> = blocks_with_heights.into_iter().collect();
        sorted_blocks.sort_by_key(|(_, block)| block.height());

        info!(
            block_count = sorted_blocks.len(),
            first_height = sorted_blocks.first().map(|(_, b)| b.height()),
            last_height = sorted_blocks.last().map(|(_, b)| b.height()),
            "Fetched blocks from Celestia"
        );

        Ok(sorted_blocks)
    }

    /// Sync a single block: validate, execute, update state.
    ///
    /// Returns the block hash to use as next parent.
    async fn sync_single_block(
        ctx: &mut SyncContext<'_>,
        block: Block,
        celestia_height: u64,
        expected_parent: B256,
    ) -> Result<B256> {
        let block_height = block.height();
        let block_hash = block.block_hash;

        check_parent_continuity(&block, expected_parent);

        info!(
            block_height,
            celestia_height,
            tx_count = block.tx_count(),
            "Syncing block from Celestia"
        );

        execute_sync_block(ctx.execution, ctx.forkchoice, &block, block_hash, block_height).await?;

        // Track as successfully executed
        ctx.executed_blocks.write().await.insert(block_hash);

        // Add to index (commitment not available during sync)
        let entry = IndexEntry {
            block_height,
            block_hash,
            celestia_height,
            commitment: vec![],
        };
        ctx.index
            .insert(entry)
            .await
            .wrap_err("failed to update block index")?;

        info!(block_height, celestia_height, "Block synced from Celestia");

        Ok(block_hash)
    }

    /// Sync historical blocks from Celestia DA (orchestrator).
    ///
    /// Fetches all blocks from `genesis_da_height` to the current Celestia height,
    /// executes them on reth, and updates the block index.
    async fn sync_from_celestia(
        celestia: &CelestiaClient,
        execution: &ExecutionClient,
        forkchoice: &RwLock<ForkchoiceState>,
        executed_blocks: &RwLock<HashSet<B256>>,
        index: &mut BlockIndex,
        genesis_da_height: u64,
    ) -> Result<u64> {
        let current_celestia_height = celestia.finalized_height().await;

        if let Some(height) = check_sync_preconditions(index, genesis_da_height, current_celestia_height) {
            return Ok(height);
        }

        info!(
            genesis_da_height,
            current_celestia_height,
            "Syncing blocks from Celestia DA"
        );

        let blocks = Self::fetch_blocks_from_celestia(celestia, genesis_da_height, current_celestia_height).await?;

        if blocks.is_empty() {
            info!("No blocks found in Celestia range");
            return Ok(index.latest_height().unwrap_or(0));
        }

        let mut ctx = SyncContext {
            execution,
            forkchoice,
            executed_blocks,
            index,
        };

        let last_synced_height = Self::sync_block_batch(&mut ctx, blocks).await?;
        info!(last_synced_height, "Celestia sync complete");

        Ok(last_synced_height)
    }

    /// Sync a batch of blocks from Celestia.
    async fn sync_block_batch(
        ctx: &mut SyncContext<'_>,
        blocks: Vec<(u64, Block)>,
    ) -> Result<u64> {
        let mut parent_hash = ctx.forkchoice.read().await.head;
        let mut last_synced_height = ctx.index.latest_height().unwrap_or(0);

        for (celestia_height, block) in blocks {
            let block_height = block.height();
            let block_hash = block.block_hash;

            // Skip already-indexed blocks
            if ctx.index.get_by_height(block_height).is_some() {
                info!(block_height, "Block already indexed, skipping");
                parent_hash = block_hash;
                continue;
            }

            parent_hash = Self::sync_single_block(ctx, block, celestia_height, parent_hash).await?;
            last_synced_height = block_height;
        }

        Ok(last_synced_height)
    }

    /// Spawn the batch submission task.
    ///
    /// Returns a channel sender for queueing blocks, or None if batching is disabled.
    fn spawn_batch_task(&self, celestia: &Arc<CelestiaClient>) -> Option<mpsc::Sender<Block>> {
        let batch_interval_ms = self.config.celestia.batch_interval_ms;
        let max_batch_size_bytes = self.config.celestia.max_batch_size_bytes;

        if batch_interval_ms == 0 {
            info!("Batch submission disabled (batch_interval_ms = 0), submitting every block immediately");
            return None;
        }

        let (batch_tx, batch_rx) = mpsc::channel::<Block>(1000);
        let celestia_batch = Arc::clone(celestia);

        tokio::spawn(run_batch_submission_loop(
            batch_rx,
            celestia_batch,
            batch_interval_ms,
            max_batch_size_bytes,
        ));

        info!(
            batch_interval_ms,
            max_batch_size_bytes,
            "Celestia batch submission task started"
        );

        Some(batch_tx)
    }

    /// Create the finalization callback for processing committed blocks.
    fn create_finalization_callback(
        execution: Arc<ExecutionClient>,
        forkchoice: Arc<RwLock<ForkchoiceState>>,
        celestia: Arc<CelestiaClient>,
        executed_blocks: Arc<RwLock<HashSet<B256>>>,
        batch_tx: Option<mpsc::Sender<Block>>,
        batch_enabled: bool,
    ) -> impl OnFinalized {
        let ctx = FinalizationContext {
            execution,
            forkchoice,
            celestia,
            executed_blocks,
            batch_tx,
            batch_enabled,
        };

        on_finalized(move |block, is_leader| {
            let ctx = ctx.clone();
            async move {
                process_finalized_block(&ctx, block, is_leader).await;
                Ok(())
            }
        })
    }

    /// Run the main event loop handling finality confirmations and shutdown.
    async fn run_event_loop(
        &self,
        mut finality_rx: mpsc::Receiver<FinalityConfirmation>,
        execution: Arc<ExecutionClient>,
        forkchoice: Arc<RwLock<ForkchoiceState>>,
        executed_blocks: Arc<RwLock<HashSet<B256>>>,
    ) {
        loop {
            tokio::select! {
                Some(confirmation) = finality_rx.recv() => {
                    handle_finality_confirmation(
                        &confirmation,
                        &execution,
                        &forkchoice,
                        &executed_blocks,
                    ).await;
                }

                _ = tokio::signal::ctrl_c() => {
                    warn!("Shutdown signal received");
                    break;
                }
            }
        }
    }
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Check sync preconditions. Returns Some(height) to skip sync, None to proceed.
fn check_sync_preconditions(
    index: &BlockIndex,
    genesis_da_height: u64,
    current_celestia_height: u64,
) -> Option<u64> {
    if current_celestia_height == 0 {
        info!("No Celestia finality data yet, skipping sync");
        return Some(index.latest_height().unwrap_or(0));
    }

    if genesis_da_height > current_celestia_height {
        info!(
            genesis_da_height,
            current_celestia_height,
            "Already synced past current Celestia height"
        );
        return Some(index.latest_height().unwrap_or(0));
    }

    None
}

/// Check parent hash continuity and log warnings.
fn check_parent_continuity(block: &Block, expected_parent: B256) {
    let block_height = block.height();

    // Skip check for genesis block
    if block_height <= 1 {
        return;
    }

    if block.parent_hash() != expected_parent {
        warn!(
            block_height,
            expected_parent = %expected_parent,
            actual_parent = %block.parent_hash(),
            "Parent hash mismatch, possible chain discontinuity"
        );
        // Continue anyway - might be out-of-order blocks in same Celestia height
    }
}

/// Execute a block during sync and update forkchoice.
async fn execute_sync_block(
    execution: &ExecutionClient,
    forkchoice: &RwLock<ForkchoiceState>,
    block: &Block,
    block_hash: B256,
    block_height: u64,
) -> Result<()> {
    let result = execution
        .execute_block(block)
        .await
        .wrap_err_with(|| format!("failed to execute block {block_height}"))?;

    if !result.is_valid() {
        eyre::bail!("block {} invalid during sync: {:?}", block_height, result.status);
    }

    let new_forkchoice = {
        let current = forkchoice.read().await;
        current.with_soft(block_hash)
    };

    execution
        .update_forkchoice(new_forkchoice)
        .await
        .wrap_err_with(|| format!("failed to update forkchoice for block {block_height}"))?;

    *forkchoice.write().await = new_forkchoice;
    Ok(())
}

/// Handle a received block, updating batch state.
fn handle_received_block(
    block: Option<Block>,
    state: &mut BatchState,
    max_batch_size_bytes: usize,
) -> BatchAction {
    let Some(block) = block else {
        return BatchAction::Shutdown;
    };

    let block_size = estimate_block_size(&block);
    state.pending_blocks.push(block);
    state.pending_size += block_size;

    if state.pending_size >= max_batch_size_bytes {
        info!(
            pending_size = state.pending_size,
            max_batch_size_bytes,
            block_count = state.pending_blocks.len(),
            "Batch size limit reached, submitting early"
        );
        BatchAction::Submit
    } else {
        BatchAction::Continue
    }
}

/// Run the batch submission loop.
async fn run_batch_submission_loop(
    mut batch_rx: mpsc::Receiver<Block>,
    celestia: Arc<CelestiaClient>,
    batch_interval_ms: u64,
    max_batch_size_bytes: usize,
) {
    let mut ticker = tokio::time::interval(Duration::from_millis(batch_interval_ms));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let mut state = BatchState::new();

    loop {
        let action = wait_for_batch_event(&mut batch_rx, &mut ticker, &mut state, max_batch_size_bytes).await;

        match action {
            BatchAction::Continue => {}
            BatchAction::Submit => {
                submit_batch(&celestia, &mut state).await;
                ticker.reset();
            }
            BatchAction::Shutdown => {
                submit_batch(&celestia, &mut state).await;
                break;
            }
        }
    }
}

/// Wait for the next batch event and determine action.
async fn wait_for_batch_event(
    batch_rx: &mut mpsc::Receiver<Block>,
    ticker: &mut tokio::time::Interval,
    state: &mut BatchState,
    max_batch_size_bytes: usize,
) -> BatchAction {
    tokio::select! {
        _ = ticker.tick() => {
            if state.is_empty() { BatchAction::Continue } else { BatchAction::Submit }
        }
        block = batch_rx.recv() => {
            handle_received_block(block, state, max_batch_size_bytes)
        }
    }
}

/// Process a finalized block: execute, track, and optionally submit to Celestia.
async fn process_finalized_block(ctx: &FinalizationContext, block: Block, is_leader: bool) {
    let block_hash = block.block_hash;
    let block_height = block.height();

    if let Err(e) = execute_and_update_forkchoice(
        &ctx.execution,
        &ctx.forkchoice,
        &block,
        block_hash,
        block_height,
    )
    .await
    {
        error!(height = block_height, error = %e, "Block execution failed");
        return;
    }

    // Track as successfully executed (enables finality confirmation)
    ctx.executed_blocks.write().await.insert(block_hash);

    if is_leader {
        submit_to_celestia(&ctx.celestia, ctx.batch_tx.as_ref(), ctx.batch_enabled, block, block_hash, block_height).await;
    } else {
        info!(height = block_height, "Block finalized (not leader, skipping Celestia submission)");
    }
}

/// Execute a block and update the forkchoice state.
async fn execute_and_update_forkchoice(
    execution: &ExecutionClient,
    forkchoice: &RwLock<ForkchoiceState>,
    block: &Block,
    block_hash: alloy_primitives::B256,
    block_height: u64,
) -> Result<()> {
    info!(
        height = block_height,
        tx_count = block.tx_count(),
        "Executing block on reth"
    );

    let result = execution
        .execute_block(block)
        .await
        .wrap_err("execution failed")?;

    if !result.is_valid() {
        eyre::bail!("block invalid: {:?}", result.status);
    }

    info!(
        height = block_height,
        %block_hash,
        "Block executed successfully"
    );

    // Update forkchoice - this block is now head and safe
    let new_forkchoice = {
        let current = forkchoice.read().await;
        current.with_soft(block_hash)
    };

    execution
        .update_forkchoice(new_forkchoice)
        .await
        .wrap_err("forkchoice update failed")?;

    *forkchoice.write().await = new_forkchoice;

    Ok(())
}

/// Submit a block to Celestia (either batched or immediate).
async fn submit_to_celestia(
    celestia: &CelestiaClient,
    batch_tx: Option<&mpsc::Sender<Block>>,
    batch_enabled: bool,
    block: Block,
    block_hash: alloy_primitives::B256,
    block_height: u64,
) {
    if batch_enabled {
        queue_for_batch(batch_tx, block, block_height).await;
    } else {
        submit_immediate(celestia, block, block_hash, block_height).await;
    }
}

/// Queue a block for batch submission.
async fn queue_for_batch(batch_tx: Option<&mpsc::Sender<Block>>, block: Block, block_height: u64) {
    let Some(tx) = batch_tx else {
        return;
    };

    if let Err(e) = tx.send(block).await {
        error!(height = block_height, error = %e, "Failed to queue block for batch submission");
    } else {
        info!(height = block_height, "Block queued for batch submission");
    }
}

/// Submit a single block immediately to Celestia.
async fn submit_immediate(
    celestia: &CelestiaClient,
    block: Block,
    block_hash: alloy_primitives::B256,
    block_height: u64,
) {
    match celestia.submit_block(&block).await {
        Ok(submission) => {
            info!(
                block_height,
                celestia_height = submission.celestia_height,
                "Block submitted to Celestia"
            );
            celestia.track_finality(block_hash, submission);
        }
        Err(e) => {
            error!(block_height, error = %e, "Failed to submit block to Celestia");
        }
    }
}

/// Handle a finality confirmation from Celestia.
///
/// This updates the forkchoice finalized pointer to the confirmed block.
/// The block MUST have been successfully executed (present in `executed_blocks`)
/// to ensure reth knows about it and it's on the canonical chain.
async fn handle_finality_confirmation(
    confirmation: &FinalityConfirmation,
    execution: &ExecutionClient,
    forkchoice: &RwLock<ForkchoiceState>,
    executed_blocks: &RwLock<HashSet<B256>>,
) {
    log_finality_event(confirmation);

    if !verify_block_executed(confirmation, executed_blocks).await {
        return;
    }

    update_finalized_forkchoice(confirmation, execution, forkchoice).await;
}

/// Log the finality confirmation event.
fn log_finality_event(confirmation: &FinalityConfirmation) {
    info!(
        block_height = confirmation.block_height,
        celestia_height = confirmation.celestia_height,
        latency_ms = confirmation.latency_ms(),
        "Block finalized on Celestia"
    );
}

/// Verify the block was successfully executed before updating finality.
///
/// Returns true if safe to proceed with finality update.
async fn verify_block_executed(
    confirmation: &FinalityConfirmation,
    executed_blocks: &RwLock<HashSet<B256>>,
) -> bool {
    // CRITICAL: Only update finalized pointer for blocks we've successfully executed.
    // If a block wasn't executed (e.g., execution failed, we're out of sync),
    // updating forkchoice would fail with "Invalid forkchoice state" (-38002)
    // because reth either doesn't know the block or it's not on the canonical chain.
    if executed_blocks.read().await.contains(&confirmation.block_hash) {
        return true;
    }

    warn!(
        block_height = confirmation.block_height,
        block_hash = %confirmation.block_hash,
        "Skipping finality confirmation - block not in executed set. \
         This may indicate the block failed to execute or we're behind on sync."
    );
    false
}

/// Update the forkchoice finalized pointer.
async fn update_finalized_forkchoice(
    confirmation: &FinalityConfirmation,
    execution: &ExecutionClient,
    forkchoice: &RwLock<ForkchoiceState>,
) {
    let new_forkchoice = {
        let current = forkchoice.read().await;
        current.with_firm(confirmation.block_hash)
    };

    if let Err(e) = execution.update_forkchoice(new_forkchoice).await {
        error!(
            block_height = confirmation.block_height,
            block_hash = %confirmation.block_hash,
            error = %e,
            "Failed to update finalized forkchoice - block may not be on canonical chain"
        );
        return;
    }

    *forkchoice.write().await = new_forkchoice;
    info!(
        finalized_height = confirmation.block_height,
        "Forkchoice updated with Celestia finality"
    );
}

/// Estimate the serialized size of a block.
#[must_use]
fn estimate_block_size(block: &Block) -> usize {
    // Header: height (8) + timestamp (8) + parent_hash (32) + proposer (20) = 68 bytes
    const HEADER_SIZE: usize = 68;
    const FRAMING_OVERHEAD: usize = 64;
    const TX_OVERHEAD: usize = 32;

    let tx_size: usize = block
        .transactions
        .iter()
        .map(|tx| tx.data().len() + TX_OVERHEAD)
        .sum();

    HEADER_SIZE + tx_size + FRAMING_OVERHEAD
}

/// Submit a batch of blocks to Celestia.
async fn submit_batch(celestia: &CelestiaClient, state: &mut BatchState) {
    let blocks = std::mem::take(&mut state.pending_blocks);
    state.pending_size = 0;

    if blocks.is_empty() {
        return;
    }

    log_batch_submission(&blocks);

    match celestia.submit_blocks(&blocks).await {
        Ok(submissions) => log_batch_success(celestia, submissions),
        Err(e) => restore_failed_batch(state, blocks, &e),
    }
}

/// Log batch submission start.
fn log_batch_submission(blocks: &[Block]) {
    info!(
        block_count = blocks.len(),
        first_height = blocks.first().map(Block::height),
        last_height = blocks.last().map(Block::height),
        "Submitting block batch to Celestia"
    );
}

/// Log successful batch submission and track finality.
fn log_batch_success(celestia: &CelestiaClient, submissions: Vec<sequencer_celestia::BlobSubmission>) {
    for submission in submissions {
        info!(
            block_height = submission.block_height,
            celestia_height = submission.celestia_height,
            "Block submitted to Celestia"
        );
        celestia.track_finality(submission.block_hash, submission);
    }
}

/// Restore blocks to state after failed submission for retry.
fn restore_failed_batch(state: &mut BatchState, blocks: Vec<Block>, error: &sequencer_celestia::CelestiaError) {
    error!(
        error = %error,
        block_count = blocks.len(),
        "Failed to submit block batch to Celestia"
    );
    state.pending_size = blocks.iter().map(estimate_block_size).sum();
    state.pending_blocks = blocks;
}

// =============================================================================
// Configuration Helpers
// =============================================================================

/// Build validator set from seeds and verify our key is included.
///
/// Returns (validators, our_address).
fn build_validator_set(
    consensus: &ConsensusConfig,
    our_pubkey: &ed25519::PublicKey,
) -> Result<(Vec<ed25519::PublicKey>, Address)> {
    let validators: Vec<ed25519::PublicKey> = consensus
        .validator_seeds
        .iter()
        .map(|seed| ed25519::PrivateKey::from_seed(*seed).public_key())
        .collect();

    if !validators.contains(our_pubkey) {
        eyre::bail!("our public key is not in the validator set");
    }

    let our_address = derive_address(our_pubkey);
    Ok((validators, our_address))
}

/// Resolve listen and dialable addresses.
fn resolve_network_addrs(consensus: &ConsensusConfig) -> Result<(SocketAddr, SocketAddr)> {
    let listen_addr: SocketAddr = consensus
        .listen_addr
        .parse()
        .wrap_err("invalid listen_addr")?;

    let dialable_addr: SocketAddr = consensus
        .dialable_addr
        .as_ref()
        .map_or_else(|| Ok(listen_addr), |s| resolve_address(s))
        .wrap_err("invalid dialable_addr")?;

    Ok((listen_addr, dialable_addr))
}

/// Build the Simplex runtime configuration from chain config.
fn build_runtime_config(consensus: &ConsensusConfig, initial_hash: B256) -> Result<SimplexRuntimeConfig> {
    let private_key = load_private_key(&consensus.private_key_path)?;
    let our_pubkey = private_key.public_key();

    let (validators, our_address) = build_validator_set(consensus, &our_pubkey)?;
    let (listen_addr, dialable_addr) = resolve_network_addrs(consensus)?;
    let bootstrappers = parse_bootstrappers(&consensus.peers)?;

    Ok(SimplexRuntimeConfig {
        private_key,
        validators,
        our_address,
        namespace: consensus.namespace.as_bytes().to_vec(),
        epoch: 1,
        listen_addr,
        dialable_addr,
        bootstrappers,
        storage_dir: consensus.storage_dir.clone(),
        leader_timeout: Duration::from_millis(consensus.leader_timeout_ms),
        notarization_timeout: Duration::from_millis(consensus.notarization_timeout_ms),
        nullify_retry: Duration::from_millis(consensus.nullify_retry_ms),
        activity_timeout: 100,
        skip_timeout: 50,
        fetch_timeout: Duration::from_secs(5),
        mailbox_size: 1024,
        allow_private_ips: consensus.allow_private_ips,
        max_message_size: 1024 * 1024,
        initial_hash,
        block_timing: consensus.block_timing,
    })
}

/// Load Ed25519 private key from file.
///
/// The file should contain a hex-encoded 32-byte seed.
fn load_private_key(path: &Path) -> Result<ed25519::PrivateKey> {
    let contents = std::fs::read_to_string(path)
        .wrap_err_with(|| format!("failed to read private key from {}", path.display()))?;

    let seed_hex = contents.trim();
    let seed_bytes = hex::decode(seed_hex).wrap_err("private key must be hex-encoded")?;

    if seed_bytes.len() != 32 {
        eyre::bail!(
            "private key must be exactly 32 bytes (got {})",
            seed_bytes.len()
        );
    }

    // Use the first 8 bytes as u64 seed for from_seed
    // This is a simplification - in production you'd want proper key loading
    let seed_array: [u8; 8] = seed_bytes[..8]
        .try_into()
        .expect("seed_bytes verified to be 32 bytes, slice of 8 always succeeds");
    let seed = u64::from_le_bytes(seed_array);

    Ok(ed25519::PrivateKey::from_seed(seed))
}

/// Derive Ethereum-style address from Ed25519 public key.
///
/// Takes the last 20 bytes of keccak256(public_key_bytes).
#[must_use]
fn derive_address(pubkey: &ed25519::PublicKey) -> Address {
    use alloy_primitives::keccak256;

    let hash = keccak256(pubkey.as_ref());
    Address::from_slice(&hash[12..])
}

/// Parse bootstrapper peers from config strings.
///
/// Format: "seed@host:port" where seed is a u64
fn parse_bootstrappers(peers: &[String]) -> Result<Vec<(ed25519::PublicKey, SocketAddr)>> {
    peers
        .iter()
        .map(|peer| {
            let (seed_str, addr_str) = peer
                .split_once('@')
                .ok_or_else(|| eyre::eyre!("invalid peer format '{}', expected 'seed@host:port'", peer))?;

            let seed: u64 = seed_str
                .parse()
                .wrap_err_with(|| format!("invalid seed in peer '{peer}'"))?;

            let addr = resolve_address(addr_str)
                .wrap_err_with(|| format!("invalid address in peer '{peer}'"))?;

            let pubkey = ed25519::PrivateKey::from_seed(seed).public_key();
            Ok((pubkey, addr))
        })
        .collect()
}

/// Resolve a hostname:port or IP:port string to a SocketAddr.
///
/// This supports both direct IP addresses and DNS hostnames (useful for Docker).
fn resolve_address(addr: &str) -> Result<SocketAddr> {
    // First try direct parse (for IP addresses)
    if let Ok(socket_addr) = addr.parse::<SocketAddr>() {
        return Ok(socket_addr);
    }

    // Otherwise, try DNS resolution
    addr.to_socket_addrs()
        .wrap_err_with(|| format!("failed to resolve address '{addr}'"))?
        .next()
        .ok_or_else(|| eyre::eyre!("no addresses found for '{addr}'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_derive_address() {
        let key = ed25519::PrivateKey::from_seed(0);
        let pubkey = Signer::public_key(&key);
        let addr = derive_address(&pubkey);

        // Address should be deterministic
        let addr2 = derive_address(&pubkey);
        assert_eq!(addr, addr2);

        // Should be 20 bytes
        assert_eq!(addr.len(), 20);
    }

    #[test]
    fn test_parse_bootstrappers() {
        let peers = vec![
            "1@127.0.0.1:26656".to_string(),
            "2@127.0.0.1:26657".to_string(),
        ];

        let bootstrappers = parse_bootstrappers(&peers).unwrap();
        assert_eq!(bootstrappers.len(), 2);

        let expected_key1 = Signer::public_key(&ed25519::PrivateKey::from_seed(1));
        let expected_key2 = Signer::public_key(&ed25519::PrivateKey::from_seed(2));

        assert_eq!(bootstrappers[0].0, expected_key1);
        assert_eq!(bootstrappers[1].0, expected_key2);

        assert_eq!(
            bootstrappers[0].1,
            "127.0.0.1:26656".parse::<SocketAddr>().unwrap()
        );
        assert_eq!(
            bootstrappers[1].1,
            "127.0.0.1:26657".parse::<SocketAddr>().unwrap()
        );
    }

    #[test]
    fn test_parse_invalid_peer_format() {
        let peers = vec!["invalid_peer".to_string()];
        assert!(parse_bootstrappers(&peers).is_err());
    }

    #[test]
    fn test_estimate_block_size() {
        use sequencer_types::BlockHeader;

        let header = BlockHeader {
            height: 1,
            timestamp: 0,
            parent_hash: alloy_primitives::B256::ZERO,
            proposer: Address::ZERO,
        };
        let block = Block::test_block(header, vec![]);

        let size = estimate_block_size(&block);
        assert!(size > 0);
        assert!(size < 1000); // Empty block should be small
    }
}
