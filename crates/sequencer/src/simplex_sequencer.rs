//! Simplex BFT sequencer.
//!
//! Uses commonware-consensus Simplex protocol for multi-validator BFT consensus.
//! Blocks are batched and submitted to Celestia at configurable intervals.

use std::net::{SocketAddr, ToSocketAddrs};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use alloy_primitives::Address;
use commonware_cryptography::{ed25519, Signer};
use eyre::{Context, Result};
use tokio::sync::{mpsc, RwLock};
use tracing::{error, info, warn};

use sequencer_celestia::{CelestiaClient, DataAvailability, FinalityConfirmation};
use sequencer_consensus::{on_finalized, start_simplex_runtime, OnFinalized, SimplexRuntimeConfig};
use sequencer_execution::{Execution, ExecutionClient, ForkchoiceState};
use sequencer_types::{Block, ConsensusConfig, Transaction};

use crate::config::Config;

/// Simplex BFT sequencer node.
///
/// Uses multi-validator BFT consensus with leader-only Celestia submission.
pub struct SimplexSequencer {
    config: Config,
}

impl SimplexSequencer {
    /// Create a new Simplex sequencer.
    pub fn new(config: Config) -> Self {
        Self { config }
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
    /// # Errors
    ///
    /// Returns an error if any component fails to start.
    pub async fn run(self) -> Result<()> {
        info!(chain_id = self.config.chain_id, "Simplex sequencer starting");

        // Initialize components
        let (celestia, finality_rx) = self.init_celestia().await?;
        let execution = self.init_execution()?;
        let runtime_config = build_runtime_config(&self.config.consensus)?;

        info!(
            validators = runtime_config.validators.len(),
            listen_addr = %runtime_config.listen_addr,
            "Starting Simplex BFT consensus"
        );

        // Create shared state
        let mempool: Arc<RwLock<Vec<Transaction>>> = Arc::new(RwLock::new(Vec::new()));
        let forkchoice = Arc::new(RwLock::new(ForkchoiceState::default()));

        // Spawn batch submission task and get channel for sending blocks
        let batch_tx = self.spawn_batch_task(&celestia);

        // Create finalization callback
        let callback = Self::create_finalization_callback(
            Arc::clone(&execution),
            Arc::clone(&forkchoice),
            Arc::clone(&celestia),
            batch_tx,
            self.config.celestia.batch_interval_ms > 0,
        );

        // Spawn consensus runtime in separate thread (it blocks)
        let runtime_handle = std::thread::spawn(move || {
            if let Err(e) = start_simplex_runtime(runtime_config, mempool, callback) {
                error!(error = %e, "Simplex runtime failed");
            }
        });

        // Run main event loop
        self.run_event_loop(finality_rx, execution, forkchoice).await;

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
        batch_tx: Option<mpsc::Sender<Block>>,
        batch_enabled: bool,
    ) -> impl OnFinalized {
        on_finalized(move |block, is_leader| {
            let execution = Arc::clone(&execution);
            let forkchoice = Arc::clone(&forkchoice);
            let batch_tx = batch_tx.clone();
            let celestia = Arc::clone(&celestia);

            async move {
                let block_hash = block.hash();
                let block_height = block.height();

                // Execute block on reth
                if let Err(e) = execute_and_update_forkchoice(
                    &execution,
                    &forkchoice,
                    &block,
                    block_hash,
                    block_height,
                )
                .await
                {
                    error!(height = block_height, error = %e, "Block execution failed");
                    return Ok(());
                }

                // Leader handles Celestia submission
                if is_leader {
                    submit_to_celestia(
                        &celestia,
                        batch_tx.as_ref(),
                        batch_enabled,
                        block,
                        block_hash,
                        block_height,
                    )
                    .await;
                } else {
                    info!(
                        height = block_height,
                        "Block finalized (not leader, skipping Celestia submission)"
                    );
                }

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
    ) {
        loop {
            tokio::select! {
                Some(confirmation) = finality_rx.recv() => {
                    handle_finality_confirmation(
                        &confirmation,
                        &execution,
                        &forkchoice,
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

/// Run the batch submission loop.
async fn run_batch_submission_loop(
    mut batch_rx: mpsc::Receiver<Block>,
    celestia: Arc<CelestiaClient>,
    batch_interval_ms: u64,
    max_batch_size_bytes: usize,
) {
    let interval = Duration::from_millis(batch_interval_ms);
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // Pre-allocate for typical batch sizes (5 second interval at ~10 blocks/sec = ~50 blocks)
    let mut pending_blocks: Vec<Block> = Vec::with_capacity(64);
    let mut pending_size: usize = 0;

    loop {
        let should_submit = tokio::select! {
            _ = ticker.tick() => !pending_blocks.is_empty(),
            block = batch_rx.recv() => {
                match block {
                    Some(block) => {
                        let block_size = estimate_block_size(&block);
                        pending_blocks.push(block);
                        pending_size += block_size;

                        if pending_size >= max_batch_size_bytes {
                            info!(
                                pending_size,
                                max_batch_size_bytes,
                                block_count = pending_blocks.len(),
                                "Batch size limit reached, submitting early"
                            );
                            true
                        } else {
                            false
                        }
                    }
                    None => {
                        // Channel closed, submit remaining and exit
                        if !pending_blocks.is_empty() {
                            submit_batch(&celestia, &mut pending_blocks, &mut pending_size).await;
                        }
                        break;
                    }
                }
            }
        };

        if should_submit {
            submit_batch(&celestia, &mut pending_blocks, &mut pending_size).await;
            ticker.reset();
        }
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
        if let Some(tx) = batch_tx {
            if let Err(e) = tx.send(block).await {
                error!(
                    height = block_height,
                    error = %e,
                    "Failed to queue block for batch submission"
                );
            } else {
                info!(height = block_height, "Block queued for batch submission");
            }
        }
    } else {
        // Submit immediately
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
                error!(
                    block_height,
                    error = %e,
                    "Failed to submit block to Celestia"
                );
            }
        }
    }
}

/// Handle a finality confirmation from Celestia.
async fn handle_finality_confirmation(
    confirmation: &FinalityConfirmation,
    execution: &ExecutionClient,
    forkchoice: &RwLock<ForkchoiceState>,
) {
    info!(
        block_height = confirmation.block_height,
        celestia_height = confirmation.celestia_height,
        latency_ms = confirmation.latency_ms(),
        "Block finalized on Celestia"
    );

    let new_forkchoice = {
        let current = forkchoice.read().await;
        current.with_firm(confirmation.block_hash)
    };

    if let Err(e) = execution.update_forkchoice(new_forkchoice).await {
        error!(
            block_height = confirmation.block_height,
            error = %e,
            "Failed to update finalized forkchoice"
        );
    } else {
        *forkchoice.write().await = new_forkchoice;
        info!(
            finalized_height = confirmation.block_height,
            "Forkchoice updated with Celestia finality"
        );
    }
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
async fn submit_batch(
    celestia: &CelestiaClient,
    pending_blocks: &mut Vec<Block>,
    pending_size: &mut usize,
) {
    let blocks = std::mem::take(pending_blocks);
    *pending_size = 0;

    if blocks.is_empty() {
        return;
    }

    info!(
        block_count = blocks.len(),
        first_height = blocks.first().map(Block::height),
        last_height = blocks.last().map(Block::height),
        "Submitting block batch to Celestia"
    );

    match celestia.submit_blocks(&blocks).await {
        Ok(submissions) => {
            for submission in submissions {
                info!(
                    block_height = submission.block_height,
                    celestia_height = submission.celestia_height,
                    "Block submitted to Celestia"
                );
                celestia.track_finality(submission.block_hash, submission);
            }
        }
        Err(e) => {
            error!(
                error = %e,
                block_count = blocks.len(),
                "Failed to submit block batch to Celestia"
            );
            // Re-add blocks to pending for retry on next interval
            *pending_blocks = blocks;
            *pending_size = pending_blocks.iter().map(estimate_block_size).sum();
        }
    }
}

// =============================================================================
// Configuration Helpers
// =============================================================================

/// Build the Simplex runtime configuration from chain config.
fn build_runtime_config(consensus: &ConsensusConfig) -> Result<SimplexRuntimeConfig> {
    let private_key = load_private_key(&consensus.private_key_path)?;
    let our_pubkey = private_key.public_key();

    let validators: Vec<ed25519::PublicKey> = consensus
        .validator_seeds
        .iter()
        .map(|seed| ed25519::PrivateKey::from_seed(*seed).public_key())
        .collect();

    if !validators.contains(&our_pubkey) {
        eyre::bail!("our public key is not in the validator set");
    }

    let our_address = derive_address(&our_pubkey);

    let listen_addr: SocketAddr = consensus
        .listen_addr
        .parse()
        .wrap_err("invalid listen_addr")?;

    let dialable_addr: SocketAddr = consensus
        .dialable_addr
        .as_ref()
        .map_or_else(
            || Ok(listen_addr),
            |s| resolve_address(s),
        )
        .wrap_err("invalid dialable_addr")?;

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
        let block = Block::new(header, vec![]);

        let size = estimate_block_size(&block);
        assert!(size > 0);
        assert!(size < 1000); // Empty block should be small
    }
}
