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

use alloy_primitives::Address;
use commonware_consensus::{
    simplex::{elector::RoundRobin, scheme::ed25519::Scheme},
    types::Epoch,
};
use commonware_cryptography::{ed25519, Sha256, Signer};
use futures::Stream;
use sequencer_types::{Block, ConsensusConfig, Transaction};
use tokio::sync::{broadcast, RwLock};
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
}

impl SimplexConfig {
    /// Create a configuration from the chain config.
    pub fn from_chain_config(
        chain_config: &ConsensusConfig,
        our_key: ed25519::PrivateKey,
        validators: Vec<ed25519::PublicKey>,
        our_address: Address,
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

    /// Mempool of pending transactions.
    mempool: Arc<RwLock<Vec<Transaction>>>,

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

        // Create shared mempool
        let mempool = Arc::new(RwLock::new(Vec::new()));

        // Create application actor
        let app_config = ApplicationConfig {
            initial_height: 1,
            proposer: config.our_address,
            mailbox_size: config.mailbox_size,
        };
        let (application, mailbox, _block_receiver) =
            Application::new(app_config, Arc::clone(&mempool));

        // Get block sender for cloning
        let (block_sender, _) = broadcast::channel(100);

        Ok(Self {
            config,
            application: Some(application),
            mailbox,
            mempool,
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

    async fn submit_transaction(&self, tx: Transaction) -> Result<()> {
        // Add to mempool directly (Simplex will pick up during propose)
        self.mempool.write().await.push(tx);
        Ok(())
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
    use super::*;

    fn test_config() -> SimplexConfig {
        let key = ed25519::PrivateKey::from_seed(0);
        let pubkey = key.public_key();

        SimplexConfig {
            our_key: key,
            validators: vec![pubkey],
            our_address: Address::ZERO,
            namespace: "test".to_string(),
            epoch: 1,
            leader_timeout: Duration::from_secs(2),
            notarization_timeout: Duration::from_secs(3),
            nullify_retry: Duration::from_secs(10),
            activity_timeout: 100,
            skip_timeout: 50,
            fetch_timeout: Duration::from_secs(5),
            fetch_concurrent: 4,
            mailbox_size: 1024,
            network: None,
        }
    }

    #[test]
    fn test_simplex_new() {
        let config = test_config();
        let consensus = SimplexConsensus::new(config);
        assert!(consensus.is_ok());
    }

    #[test]
    fn test_empty_validators_fails() {
        let mut config = test_config();
        config.validators = vec![];
        let consensus = SimplexConsensus::new(config);
        assert!(consensus.is_err());
    }

    #[test]
    fn test_is_validator() {
        let config = test_config();
        let consensus = SimplexConsensus::new(config).unwrap();
        assert!(consensus.is_validator());
    }

    #[test]
    fn test_is_not_validator() {
        let mut config = test_config();
        // Use a different key in validators
        let other_key = ed25519::PrivateKey::from_seed(999);
        config.validators = vec![other_key.public_key()];

        let consensus = SimplexConsensus::new(config).unwrap();
        assert!(!consensus.is_validator());
    }
}
