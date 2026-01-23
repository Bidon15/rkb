//! Runtime integration for Simplex BFT consensus.
//!
//! This module provides the full integration between:
//! - commonware-runtime (async execution, storage, metrics)
//! - commonware-p2p (authenticated peer networking)
//! - commonware-consensus Simplex (BFT consensus)
//!
//! ## Usage
//!
//! ```ignore
//! use sequencer_consensus::runtime::{RuntimeConfig, start_consensus};
//!
//! let config = RuntimeConfig { ... };
//! start_consensus(config, |finalized_block, is_leader| async move {
//!     if is_leader {
//!         celestia.submit_block(&finalized_block).await?;
//!     }
//!     Ok(())
//! });
//! ```

use std::future::Future;
use std::net::SocketAddr;
use std::num::{NonZeroU16, NonZeroUsize};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use alloy_primitives::{Address, B256};
use sequencer_execution::ExecutionClient;
use commonware_consensus::{
    simplex::{
        config::Config as EngineConfig,
        elector::RoundRobin,
        scheme::ed25519::Scheme,
        Engine,
    },
    types::{Epoch, ViewDelta},
};
use commonware_cryptography::{ed25519, Sha256, Signer};
use commonware_p2p::{authenticated::discovery, Manager};
use commonware_parallel::Sequential;
use commonware_runtime::{
    buffer::PoolRef,
    tokio::{Config as RuntimeConfig, Runner},
    Clock, Metrics, Network as RNetwork, Resolver, Runner as RunnerTrait, Spawner, Storage,
};
use commonware_utils::{ordered::Set, union};
use rand::{CryptoRng, RngCore};
use sequencer_types::{Block, ConsensusConfig};

use crate::application::{Application, ApplicationConfig};
use crate::network::{backlog, channels, rate_limits};
use crate::reporter::ConsensusReporter;
use crate::{ConsensusError, Result};

/// Page size for buffer pool.
const PAGE_SIZE: usize = 4096;

/// Page cache size for buffer pool.
const PAGE_CACHE_SIZE: usize = 1024;

/// Configuration for the full Simplex BFT runtime.
#[derive(Clone)]
pub struct SimplexRuntimeConfig {
    /// Our Ed25519 private key for signing.
    pub private_key: ed25519::PrivateKey,

    /// All validator public keys.
    pub validators: Vec<ed25519::PublicKey>,

    /// Our validator address (Ethereum-style).
    pub our_address: Address,

    /// Network namespace for message signing.
    pub namespace: Vec<u8>,

    /// Current epoch.
    pub epoch: u64,

    /// P2P listen address.
    pub listen_addr: SocketAddr,

    /// P2P dialable address (how others reach us).
    pub dialable_addr: SocketAddr,

    /// Bootstrap peers (public key, address).
    pub bootstrappers: Vec<(ed25519::PublicKey, SocketAddr)>,

    /// Storage directory for consensus state.
    pub storage_dir: PathBuf,

    /// Leader timeout.
    pub leader_timeout: Duration,

    /// Notarization timeout.
    pub notarization_timeout: Duration,

    /// Nullify retry timeout.
    pub nullify_retry: Duration,

    /// Activity timeout in views.
    pub activity_timeout: u64,

    /// Skip timeout in views.
    pub skip_timeout: u64,

    /// Fetch timeout for resolver.
    pub fetch_timeout: Duration,

    /// Mailbox size for internal channels.
    pub mailbox_size: usize,

    /// Allow private IPs (for testing).
    pub allow_private_ips: bool,

    /// Maximum message size.
    pub max_message_size: u32,

    /// Initial block hash (parent of first block).
    /// Use B256::ZERO for genesis.
    pub initial_hash: B256,
}

impl SimplexRuntimeConfig {
    /// Create from chain consensus config.
    pub fn from_chain_config(
        chain_config: &ConsensusConfig,
        private_key: ed25519::PrivateKey,
        validators: Vec<ed25519::PublicKey>,
        our_address: Address,
        listen_addr: SocketAddr,
        dialable_addr: SocketAddr,
        bootstrappers: Vec<(ed25519::PublicKey, SocketAddr)>,
        storage_dir: PathBuf,
        initial_hash: B256,
    ) -> Self {
        Self {
            private_key,
            validators,
            our_address,
            namespace: chain_config.namespace.as_bytes().to_vec(),
            epoch: 1,
            listen_addr,
            dialable_addr,
            bootstrappers,
            storage_dir,
            leader_timeout: Duration::from_millis(chain_config.leader_timeout_ms),
            notarization_timeout: Duration::from_millis(chain_config.notarization_timeout_ms),
            nullify_retry: Duration::from_millis(chain_config.nullify_retry_ms),
            activity_timeout: 100,
            skip_timeout: 50,
            fetch_timeout: Duration::from_secs(5),
            mailbox_size: 1024,
            allow_private_ips: true,
            max_message_size: 1024 * 1024,
            initial_hash,
        }
    }

    /// Check if we are in the validator set.
    pub fn is_validator(&self) -> bool {
        let our_pubkey = self.private_key.public_key();
        self.validators.contains(&our_pubkey)
    }

    /// Get our index in the validator set.
    pub fn validator_index(&self) -> Option<usize> {
        let our_pubkey = self.private_key.public_key();
        self.validators.iter().position(|pk| pk == &our_pubkey)
    }
}

/// Callback for when a block is finalized.
///
/// The callback receives the finalized block and whether we are the leader
/// who should submit to Celestia.
pub trait OnFinalized: Send + 'static {
    /// Called when a block is finalized.
    fn on_finalized(
        &self,
        block: Block,
        is_leader: bool,
    ) -> impl Future<Output = Result<()>> + Send;
}

/// Simple callback implementation using a closure.
pub struct FinalizedCallback<F> {
    callback: F,
}

impl<F, Fut> OnFinalized for FinalizedCallback<F>
where
    F: Fn(Block, bool) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<()>> + Send,
{
    async fn on_finalized(&self, block: Block, is_leader: bool) -> Result<()> {
        (self.callback)(block, is_leader).await
    }
}

/// Create a finalized callback from a closure.
pub fn on_finalized<F, Fut>(f: F) -> FinalizedCallback<F>
where
    F: Fn(Block, bool) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<()>> + Send,
{
    FinalizedCallback { callback: f }
}

/// Start the Simplex BFT consensus engine.
///
/// This function:
/// 1. Creates the commonware runtime
/// 2. Sets up P2P networking with authenticated peers
/// 3. Registers consensus channels (votes, certificates, resolver)
/// 4. Starts the Simplex Engine
/// 5. Calls the callback when blocks are finalized
///
/// # Arguments
///
/// * `config` - Runtime configuration
/// * `mempool` - Shared mempool for transactions
/// * `on_finalized` - Callback when blocks are finalized
///
/// # Returns
///
/// This function blocks until the consensus engine shuts down.
pub fn start_simplex_runtime<C: OnFinalized>(
    config: SimplexRuntimeConfig,
    execution: Arc<ExecutionClient>,
    callback: C,
) -> Result<()> {
    if !config.is_validator() {
        return Err(ConsensusError::InvalidConfig(
            "not in validator set".to_string(),
        ));
    }

    if config.validators.is_empty() {
        return Err(ConsensusError::InvalidConfig(
            "validator set is empty".to_string(),
        ));
    }

    // Create runtime configuration
    let runtime_config = RuntimeConfig::new()
        .with_storage_directory(&config.storage_dir)
        .with_worker_threads(4)
        .with_catch_panics(true);

    // Start the runtime
    let runner = Runner::new(runtime_config);
    runner.start(|context| async move {
        run_simplex(context, config, execution, callback).await
    });

    Ok(())
}

/// Internal function to run Simplex within the commonware runtime.
async fn run_simplex<E, C>(
    context: E,
    config: SimplexRuntimeConfig,
    execution: Arc<ExecutionClient>,
    callback: C,
) where
    E: Spawner + Clock + CryptoRng + RngCore + RNetwork + Resolver + Metrics + Storage + Clone + Send + 'static,
    C: OnFinalized,
{
    tracing::info!(
        validators = config.validators.len(),
        epoch = config.epoch,
        "Starting Simplex BFT consensus"
    );

    // Create P2P network configuration
    let bootstrappers: Vec<_> = config
        .bootstrappers
        .iter()
        .map(|(pk, addr)| (pk.clone(), commonware_p2p::Ingress::from(*addr)))
        .collect();

    let p2p_config = if config.allow_private_ips {
        discovery::Config::local(
            config.private_key.clone(),
            &union(&config.namespace, b"_P2P"),
            config.listen_addr,
            config.dialable_addr,
            bootstrappers,
            config.max_message_size,
        )
    } else {
        discovery::Config::recommended(
            config.private_key.clone(),
            &union(&config.namespace, b"_P2P"),
            config.listen_addr,
            config.dialable_addr,
            bootstrappers,
            config.max_message_size,
        )
    };

    // Create P2P network
    let (mut network, mut oracle) =
        discovery::Network::new(context.clone().with_label("p2p"), p2p_config);

    // Create ordered participant set for scheme and elector
    let participants: Set<ed25519::PublicKey> = Set::from_iter_dedup(config.validators.iter().cloned());

    // Register all validators with the oracle (uses Manager trait)
    oracle.update(config.epoch, participants.clone()).await;

    // Register consensus channels (channel IDs are u64)
    let (vote_sender, vote_receiver) = network.register(
        u64::from(channels::VOTE),
        rate_limits::vote(),
        backlog::VOTE,
    );
    let (cert_sender, cert_receiver) = network.register(
        u64::from(channels::CERTIFICATE),
        rate_limits::certificate(),
        backlog::CERTIFICATE,
    );
    let (resolver_sender, resolver_receiver) = network.register(
        u64::from(channels::RESOLVER),
        rate_limits::resolver(),
        backlog::RESOLVER,
    );

    // Start P2P network
    let network_handle = network.start();

    // Create application with execution client for vanilla Ethereum block building
    let app_config = ApplicationConfig {
        initial_height: 1,
        initial_hash: config.initial_hash,
        proposer: config.our_address,
        execution,
        mailbox_size: config.mailbox_size,
    };
    let (application, app_mailbox, mut block_receiver) = Application::new(app_config);

    // Spawn application actor
    let app_context = context.clone().with_label("application");
    app_context.spawn(|_| async move {
        application.run().await;
    });

    // Create scheme, elector config, reporter
    // Note: RoundRobin is an elector Config trait - Engine builds it internally
    let scheme = create_scheme(&config, &participants).expect("our key must be in validator set");
    let elector = RoundRobin::<Sha256>::default();
    // Reporter receives finalization events and forwards them to the application
    let reporter = ConsensusReporter::new(true, app_mailbox.clone());

    // Create Simplex Engine config
    // Note: oracle implements Blocker trait, so we pass it directly
    let engine_config = EngineConfig {
        scheme,
        elector,
        blocker: oracle,
        automaton: app_mailbox.clone(),
        relay: app_mailbox,
        reporter,
        strategy: Sequential,
        partition: config.private_key.public_key().to_string(),
        mailbox_size: config.mailbox_size,
        epoch: Epoch::new(config.epoch),
        leader_timeout: config.leader_timeout,
        notarization_timeout: config.notarization_timeout,
        nullify_retry: config.nullify_retry,
        activity_timeout: ViewDelta::new(config.activity_timeout),
        skip_timeout: ViewDelta::new(config.skip_timeout),
        fetch_timeout: config.fetch_timeout,
        fetch_concurrent: 4,
        replay_buffer: NonZeroUsize::new(1024 * 1024).expect("1MB is non-zero"),
        write_buffer: NonZeroUsize::new(1024 * 1024).expect("1MB is non-zero"),
        buffer_pool: PoolRef::new(
            NonZeroU16::new(PAGE_SIZE as u16).expect("PAGE_SIZE is non-zero"),
            NonZeroUsize::new(PAGE_CACHE_SIZE).expect("PAGE_CACHE_SIZE is non-zero"),
        ),
    };

    // Create and start the engine
    let engine = Engine::new(context.clone().with_label("simplex"), engine_config);
    let engine_handle = engine.start(
        (vote_sender, vote_receiver),
        (cert_sender, cert_receiver),
        (resolver_sender, resolver_receiver),
    );

    // Track current view for leader detection
    // SAFETY: is_validator() was verified at the start of start_simplex_runtime()
    let our_index = config.validator_index().expect("already verified we are a validator");
    let num_validators = config.validators.len();

    // Handle finalized blocks
    loop {
        tokio::select! {
            block_result = block_receiver.recv() => {
                match block_result {
                    Ok(block) => {
                        // Determine if we are the leader for this block's height
                        let leader_index = (block.height() as usize) % num_validators;
                        let is_leader = leader_index == our_index;

                        tracing::info!(
                            height = block.height(),
                            is_leader,
                            tx_count = block.tx_count(),
                            "Block finalized"
                        );

                        // Call the finalization callback
                        if let Err(e) = callback.on_finalized(block, is_leader).await {
                            tracing::error!(error = %e, "Finalization callback failed");
                        }
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "Block receiver error");
                        break;
                    }
                }
            }
            _ = context.stopped() => {
                tracing::info!("Runtime stopped, shutting down consensus");
                break;
            }
        }
    }

    // Wait for handles to complete
    drop(engine_handle);
    drop(network_handle);
}

/// Create the Ed25519 signing scheme for Simplex.
///
/// Returns `None` if our private key is not in the validator set.
fn create_scheme(
    config: &SimplexRuntimeConfig,
    participants: &Set<ed25519::PublicKey>,
) -> Option<Scheme> {
    // Create a signing scheme - returns None if our key isn't in the set
    Scheme::signer(&config.namespace, participants.clone(), config.private_key.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> SimplexRuntimeConfig {
        let key = ed25519::PrivateKey::from_seed(0);
        let pubkey = key.public_key();

        SimplexRuntimeConfig {
            private_key: key,
            validators: vec![pubkey],
            our_address: Address::ZERO,
            namespace: b"test".to_vec(),
            epoch: 1,
            listen_addr: "127.0.0.1:26656".parse().unwrap(),
            dialable_addr: "127.0.0.1:26656".parse().unwrap(),
            bootstrappers: vec![],
            storage_dir: std::env::temp_dir().join("test_consensus"),
            leader_timeout: Duration::from_secs(2),
            notarization_timeout: Duration::from_secs(3),
            nullify_retry: Duration::from_secs(10),
            activity_timeout: 100,
            skip_timeout: 50,
            fetch_timeout: Duration::from_secs(5),
            mailbox_size: 1024,
            allow_private_ips: true,
            max_message_size: 1024 * 1024,
            initial_hash: B256::ZERO,
        }
    }

    #[test]
    fn test_is_validator() {
        let config = test_config();
        assert!(config.is_validator());
    }

    #[test]
    fn test_validator_index() {
        let config = test_config();
        assert_eq!(config.validator_index(), Some(0));
    }

    #[test]
    fn test_not_validator() {
        let mut config = test_config();
        let other_key = ed25519::PrivateKey::from_seed(999);
        config.validators = vec![other_key.public_key()];
        assert!(!config.is_validator());
        assert_eq!(config.validator_index(), None);
    }
}
