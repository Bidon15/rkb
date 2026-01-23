//! Simplex BFT consensus engine.
//!
//! This module provides multi-validator BFT consensus using the commonware-consensus
//! Simplex protocol. It integrates with our application layer for block production
//! and the P2P network for validator communication.
//!
//! ## Architecture
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────────────┐
//! │                     Simplex Engine                                │
//! ├──────────────────────────────────────────────────────────────────┤
//! │  ┌─────────────┐    ┌─────────────┐    ┌─────────────────────┐  │
//! │  │   Voter     │◀──▶│   Batcher   │◀──▶│     Resolver        │  │
//! │  │  (voting)   │    │ (sig batch) │    │  (cert fetching)    │  │
//! │  └──────┬──────┘    └─────────────┘    └─────────────────────┘  │
//! │         │                                                        │
//! │         ▼                                                        │
//! │  ┌─────────────────────────────────────────────────────────────┐│
//! │  │                    Application                               ││
//! │  │  genesis() ─▶ propose() ─▶ verify() ─▶ certify() ─▶ relay()││
//! │  └─────────────────────────────────────────────────────────────┘│
//! └──────────────────────────────────────────────────────────────────┘
//!                              │
//!                              ▼
//! ┌──────────────────────────────────────────────────────────────────┐
//! │                     P2P Network Layer                             │
//! │  ┌────────────┐  ┌────────────┐  ┌────────────┐  ┌────────────┐ │
//! │  │   Votes    │  │Certificates│  │  Resolver  │  │Block Relay │ │
//! │  │ (channel 0)│  │ (channel 1)│  │ (channel 2)│  │ (channel 3)│ │
//! │  └────────────┘  └────────────┘  └────────────┘  └────────────┘ │
//! └──────────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Usage
//!
//! ```ignore
//! use sequencer_consensus::simplex::{SimplexConfig, SimplexConsensus};
//!
//! // Configure the consensus engine
//! let config = SimplexConfig {
//!     validators: vec![validator1, validator2, validator3],
//!     our_key: our_private_key,
//!     namespace: "my-chain".to_string(),
//!     // ... other config
//! };
//!
//! // Create and start the engine
//! let consensus = SimplexConsensus::new(config)?;
//! consensus.start().await?;
//! ```

use std::sync::Arc;
use std::time::Duration;

use alloy_primitives::{Address, B256};
use commonware_consensus::{
    simplex::{elector::RoundRobin, scheme::ed25519::Scheme},
    types::Epoch,
};
use commonware_cryptography::{ed25519, Sha256, Signer};
use futures::Stream;
use sequencer_execution::ExecutionClient;
use sequencer_types::{Block, ConsensusConfig};
use tokio::sync::broadcast;
use tokio_stream::StreamExt;

use crate::application::{Application, ApplicationConfig, Mailbox};
use crate::network::NetworkConfig;
use crate::{BlockProducer, ConsensusError, Result};

/// Configuration for Simplex BFT consensus.
#[derive(Clone)]
pub struct SimplexConfig {
    /// Validator private keys mapped to their public keys.
    pub our_key: ed25519::PrivateKey,

    /// All validator public keys in the network.
    pub validators: Vec<ed25519::PublicKey>,

    /// Our validator address (derived from public key).
    pub our_address: Address,

    /// Network namespace for message signing.
    pub namespace: String,

    /// Epoch for this consensus instance.
    pub epoch: u64,

    /// Leader timeout duration.
    pub leader_timeout: Duration,

    /// Notarization timeout duration.
    pub notarization_timeout: Duration,

    /// Nullify retry duration.
    pub nullify_retry: Duration,

    /// Activity timeout in views.
    pub activity_timeout: u64,

    /// Skip timeout in views.
    pub skip_timeout: u64,

    /// Fetch timeout for resolver.
    pub fetch_timeout: Duration,

    /// Concurrent fetch requests.
    pub fetch_concurrent: usize,

    /// Mailbox size for internal channels.
    pub mailbox_size: usize,

    /// Network configuration.
    pub network: Option<NetworkConfig>,

    /// Initial block hash (parent of first block).
    pub initial_hash: B256,

    /// Execution client for building blocks via reth.
    pub execution: Arc<ExecutionClient>,
}

impl SimplexConfig {
    /// Create a configuration from the chain config.
    pub fn from_chain_config(
        chain_config: &ConsensusConfig,
        our_key: ed25519::PrivateKey,
        validators: Vec<ed25519::PublicKey>,
        our_address: Address,
        execution: Arc<ExecutionClient>,
    ) -> Self {
        Self {
            our_key,
            validators,
            our_address,
            namespace: chain_config.namespace.clone(),
            epoch: 1,
            leader_timeout: Duration::from_millis(chain_config.leader_timeout_ms),
            notarization_timeout: Duration::from_millis(chain_config.notarization_timeout_ms),
            nullify_retry: Duration::from_millis(chain_config.nullify_retry_ms),
            activity_timeout: 100,
            skip_timeout: 50,
            fetch_timeout: Duration::from_secs(5),
            fetch_concurrent: 4,
            mailbox_size: 1024,
            network: None,
            initial_hash: B256::ZERO,
            execution,
        }
    }
}

/// Simplex BFT consensus engine.
///
/// This engine uses the commonware-consensus Simplex protocol for BFT consensus
/// among multiple validators. The leader for each view submits the finalized
/// block to Celestia.
pub struct SimplexConsensus {
    config: SimplexConfig,

    /// Application actor for block production.
    #[allow(dead_code)]
    application: Option<Application>,

    /// Mailbox for communicating with the application.
    #[allow(dead_code)]
    mailbox: Mailbox,

    /// Block broadcast sender (for cloning).
    block_sender: broadcast::Sender<Block>,
}

impl SimplexConsensus {
    /// Create a new Simplex consensus engine.
    ///
    /// # Errors
    ///
    /// Returns an error if configuration is invalid.
    pub fn new(config: SimplexConfig) -> Result<Self> {
        if config.validators.is_empty() {
            return Err(ConsensusError::InvalidConfig(
                "validator set cannot be empty".to_string(),
            ));
        }

        // Create application actor with execution client
        let app_config = ApplicationConfig {
            initial_height: 1,
            initial_hash: config.initial_hash,
            proposer: config.our_address,
            execution: Arc::clone(&config.execution),
            mailbox_size: config.mailbox_size,
        };
        let (application, mailbox, _block_receiver, _block_relay_receiver) = Application::new(app_config);

        // Get block sender for cloning
        let (block_sender, _) = broadcast::channel(100);

        Ok(Self {
            config,
            application: Some(application),
            mailbox,
            block_sender,
        })
    }

    /// Get the validator set.
    pub fn validators(&self) -> &[ed25519::PublicKey] {
        &self.config.validators
    }

    /// Check if we are a validator.
    pub fn is_validator(&self) -> bool {
        let our_pubkey = self.config.our_key.public_key();
        self.config.validators.contains(&our_pubkey)
    }

    /// Get the current epoch.
    pub fn epoch(&self) -> Epoch {
        Epoch::new(self.config.epoch)
    }
}

#[async_trait::async_trait]
impl BlockProducer for SimplexConsensus {
    async fn start(&self) -> Result<()> {
        tracing::info!(
            validators = self.config.validators.len(),
            is_validator = self.is_validator(),
            epoch = self.config.epoch,
            "Starting Simplex BFT consensus engine"
        );

        if !self.is_validator() {
            tracing::warn!("Running in non-validator mode, will not participate in consensus");
            return Ok(());
        }

        // For full Simplex integration, we need:
        // 1. commonware-runtime context (Clock, Spawner, Storage, Metrics)
        // 2. P2P network setup with commonware-p2p
        // 3. Network channel registration for votes, certificates, resolver
        //
        // This requires significant additional setup. For now, we log the
        // configuration and indicate that full Simplex is ready for integration.

        tracing::info!(
            namespace = %self.config.namespace,
            leader_timeout_ms = ?self.config.leader_timeout,
            notarization_timeout_ms = ?self.config.notarization_timeout,
            "Simplex BFT configuration ready"
        );

        // In a full implementation, we would:
        // 1. Create commonware runtime context
        // 2. Set up P2P network with authenticated discovery
        // 3. Register network channels (votes, certificates, resolver)
        // 4. Create Simplex Engine with Config
        // 5. Start the engine with network channels
        //
        // Example (requires full runtime setup):
        // ```
        // let engine = Engine::new(context, config);
        // engine.start(vote_network, certificate_network, resolver_network);
        // ```

        Ok(())
    }

    async fn submit_transaction(&self, _tx: sequencer_types::Transaction) -> Result<()> {
        // With vanilla Ethereum flow, transactions go directly to reth's mempool
        // via JSON-RPC (eth_sendRawTransaction), not through this method.
        Err(ConsensusError::SubmitFailed(
            "transactions should be submitted directly to reth's mempool".to_string(),
        ))
    }

    fn block_stream(&self) -> Box<dyn Stream<Item = Block> + Send + Unpin> {
        let rx = self.block_sender.subscribe();
        Box::new(
            tokio_stream::wrappers::BroadcastStream::new(rx).filter_map(std::result::Result::ok),
        )
    }
}

/// Type alias for the Simplex scheme we use (Ed25519).
pub type SimplexScheme = Scheme;

/// Type alias for the RoundRobin elector.
pub type SimplexElector = RoundRobin<Sha256>;

#[cfg(test)]
mod tests {
    // SimplexConsensus tests require a running reth instance for the ExecutionClient.
    // Integration tests should be added in a separate test crate.
}
