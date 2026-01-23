//! Consensus engine implementation.
//!
//! Simple `PoA` consensus using round-robin leader election and Ed25519 signatures.
//! This implementation can be upgraded to use commonware-consensus Simplex for
//! full BFT consensus in multi-validator deployments.

use std::sync::Arc;
use std::time::Duration;

use alloy_primitives::{keccak256, Address, B256};
use commonware_cryptography::{ed25519, Signer};
use futures::Stream;
use sequencer_types::{Block, BlockHeader, ConsensusConfig, Signature, Transaction};
use tokio::sync::{broadcast, mpsc, RwLock};
use tokio::time::interval;
use tokio_stream::StreamExt;

use crate::{BlockProducer, ConsensusError, Result};

/// Block production interval (milliseconds).
const BLOCK_INTERVAL_MS: u64 = 1000;

/// Signing namespace for block signatures.
const BLOCK_SIGNING_NAMESPACE: &[u8] = b"astria-sequencer-block";

/// Get current Unix timestamp in seconds.
fn current_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Check if a validator is the leader for a given height.
#[allow(clippy::cast_possible_truncation)]
fn is_leader_for_height(validator_index: Option<usize>, height: u64, validators_len: usize) -> bool {
    validator_index.is_some_and(|idx| (height as usize) % validators_len == idx)
}

/// Sign a block with the given signer.
fn sign_block(block: &mut Block, signer: &ed25519::PrivateKey, proposer: Address) {
    let block_hash = block.block_hash;
    let sig = signer.sign(BLOCK_SIGNING_NAMESPACE, block_hash.as_slice());
    let public_key = signer.public_key();
    let signature = Signature::new(proposer, &public_key, &sig);
    block.signatures.push(signature);
}

/// `PoA` consensus engine.
pub struct Consensus {
    config: ConsensusConfig,
    /// Signing key for blocks (Ed25519 from commonware).
    signer: Option<ed25519::PrivateKey>,
    /// Our validator index in the validator set.
    validator_index: Option<usize>,
    /// Mempool of pending transactions.
    mempool: Arc<RwLock<Vec<Transaction>>>,
    /// Channel for submitting transactions.
    tx_sender: mpsc::Sender<Transaction>,
    /// Receiver for transactions (moved to production loop).
    tx_receiver: Arc<RwLock<Option<mpsc::Receiver<Transaction>>>>,
    /// Broadcast channel for finalized blocks.
    block_sender: broadcast::Sender<Block>,
    /// Current block height.
    height: Arc<RwLock<u64>>,
    /// Last block hash.
    last_hash: Arc<RwLock<B256>>,
}

impl Consensus {
    /// Create a new consensus engine.
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration is invalid or key loading fails.
    pub fn new(config: ConsensusConfig) -> Result<Self> {
        if config.validators.is_empty() {
            return Err(ConsensusError::InvalidConfig(
                "validator set cannot be empty".to_string(),
            ));
        }

        // Try to load private key if path exists
        let (signer, validator_index) = Self::load_signer(&config)?;

        let (tx_sender, tx_receiver) = mpsc::channel(1000);
        let (block_sender, _) = broadcast::channel(100);

        Ok(Self {
            config,
            signer,
            validator_index,
            mempool: Arc::new(RwLock::new(Vec::new())),
            tx_sender,
            tx_receiver: Arc::new(RwLock::new(Some(tx_receiver))),
            block_sender,
            height: Arc::new(RwLock::new(1)), // Start at height 1 (genesis is 0)
            last_hash: Arc::new(RwLock::new(B256::ZERO)),
        })
    }

    /// Load the Ed25519 signer from the configured key path.
    fn load_signer(
        config: &ConsensusConfig,
    ) -> Result<(Option<ed25519::PrivateKey>, Option<usize>)> {
        // Try to read key file
        let Ok(key_data) = std::fs::read_to_string(&config.private_key_path) else {
            tracing::warn!(
                path = %config.private_key_path.display(),
                "Could not load private key, running as non-validator"
            );
            return Ok((None, None));
        };
        let key_data = key_data.trim();

        // Parse hex-encoded key (32 bytes = 64 hex chars)
        let key_bytes = hex::decode(key_data).map_err(|e| {
            ConsensusError::InvalidConfig(format!("invalid private key hex: {e}"))
        })?;

        if key_bytes.len() != 32 {
            return Err(ConsensusError::InvalidConfig(format!(
                "private key must be 32 bytes, got {}",
                key_bytes.len()
            )));
        }

        // Create commonware ed25519 private key from seed
        // Use first 8 bytes of the 32-byte seed as u64 for commonware's from_seed()
        let seed_u64 = u64::from_le_bytes(
            key_bytes[0..8]
                .try_into()
                .expect("key_bytes verified to be 32 bytes, first 8 always succeed"),
        );
        let signer = ed25519::PrivateKey::from_seed(seed_u64);
        let public_key = signer.public_key();

        // Derive Ethereum address from Ed25519 public key
        let addr = Self::pubkey_to_address(&public_key);

        // Find our index in the validator set
        let validator_index = config.validators.iter().position(|v| *v == addr);

        if validator_index.is_none() {
            tracing::warn!(
                address = %addr,
                "Our address not in validator set, running as non-validator"
            );
        }

        Ok((Some(signer), validator_index))
    }

    /// Convert Ed25519 public key to Ethereum address.
    ///
    /// Uses keccak256 of the public key bytes, taking last 20 bytes.
    fn pubkey_to_address(pubkey: &ed25519::PublicKey) -> Address {
        use commonware_codec::Write;

        let mut bytes = [0u8; 32];
        let mut buf = &mut bytes[..];
        pubkey.write(&mut buf);

        let hash = keccak256(bytes);
        Address::from_slice(&hash[12..])
    }

    /// Get the validator set.
    #[must_use]
    pub fn validators(&self) -> &[Address] {
        &self.config.validators
    }

    /// Check if this node is a validator.
    #[must_use]
    pub fn is_validator(&self) -> bool {
        self.validator_index.is_some()
    }

    /// Get the current leader for a given height using round-robin.
    #[must_use]
    #[allow(clippy::cast_possible_truncation)]
    fn leader_for_height(&self, height: u64) -> Address {
        let idx = (height as usize) % self.config.validators.len();
        self.config.validators[idx]
    }

    /// Build a new block from the mempool.
    #[allow(dead_code)]
    async fn build_block(&self) -> Result<Block> {
        let height = *self.height.read().await;
        let parent_hash = *self.last_hash.read().await;
        let proposer = self.leader_for_height(height);

        let transactions: Vec<Transaction> = self.mempool.write().await.drain(..).collect();

        let header = BlockHeader {
            height,
            timestamp: current_timestamp(),
            parent_hash,
            proposer,
        };

        let mut block = Block::test_block(header, transactions);

        if let Some(ref signer) = self.signer {
            sign_block(&mut block, signer, proposer);
        }

        Ok(block)
    }

    /// Static production loop that can be spawned as a task.
    #[allow(clippy::too_many_arguments, clippy::cast_possible_truncation)]
    async fn production_loop(
        mut tx_receiver: mpsc::Receiver<Transaction>,
        mempool: Arc<RwLock<Vec<Transaction>>>,
        height: Arc<RwLock<u64>>,
        last_hash: Arc<RwLock<B256>>,
        block_sender: broadcast::Sender<Block>,
        signer: Option<ed25519::PrivateKey>,
        validators: Vec<Address>,
        validator_index: Option<usize>,
    ) {
        let mut block_interval = interval(Duration::from_millis(BLOCK_INTERVAL_MS));

        loop {
            tokio::select! {
                Some(tx) = tx_receiver.recv() => {
                    mempool.write().await.push(tx);
                }

                _ = block_interval.tick() => {
                    let current_height = *height.read().await;

                    if !is_leader_for_height(validator_index, current_height, validators.len()) {
                        continue;
                    }

                    tracing::debug!(height = current_height, "We are the leader, producing block");

                    let parent_hash = *last_hash.read().await;
                    let transactions: Vec<Transaction> = mempool.write().await.drain(..).collect();
                    let proposer = validators[(current_height as usize) % validators.len()];

                    let header = BlockHeader {
                        height: current_height,
                        timestamp: current_timestamp(),
                        parent_hash,
                        proposer,
                    };

                    let mut block = Block::test_block(header, transactions);

                    if let Some(ref signing_key) = signer {
                        sign_block(&mut block, signing_key, proposer);
                    }

                    let block_hash = block.block_hash;
                    let tx_count = block.tx_count();

                    *height.write().await = current_height + 1;
                    *last_hash.write().await = block_hash;

                    if block_sender.send(block).is_err() {
                        tracing::debug!("No block receivers");
                    }

                    tracing::info!(
                        height = current_height,
                        %block_hash,
                        tx_count,
                        "Block produced"
                    );
                }
            }
        }
    }
}

#[async_trait::async_trait]
impl BlockProducer for Consensus {
    async fn start(&self) -> Result<()> {
        tracing::info!(
            validators = self.config.validators.len(),
            is_validator = self.is_validator(),
            "Starting consensus engine"
        );

        if !self.is_validator() {
            tracing::warn!("Running in non-validator mode, will not produce blocks");
            return Ok(());
        }

        // Take the receiver from the option (only once)
        let tx_receiver = self
            .tx_receiver
            .write()
            .await
            .take()
            .ok_or_else(|| ConsensusError::StartFailed("already started".to_string()))?;

        // Clone Arc references for the spawned task
        let mempool = Arc::clone(&self.mempool);
        let height = Arc::clone(&self.height);
        let last_hash = Arc::clone(&self.last_hash);
        let block_sender = self.block_sender.clone();
        let signer = self.signer.clone();
        let validators = self.config.validators.clone();
        let validator_index = self.validator_index;

        // Spawn the production loop
        tokio::spawn(async move {
            Self::production_loop(
                tx_receiver,
                mempool,
                height,
                last_hash,
                block_sender,
                signer,
                validators,
                validator_index,
            )
            .await;
        });

        tracing::info!("Block production loop started");
        Ok(())
    }

    async fn submit_transaction(&self, tx: Transaction) -> Result<()> {
        self.tx_sender
            .send(tx)
            .await
            .map_err(|e| ConsensusError::SubmitFailed(e.to_string()))?;
        Ok(())
    }

    fn block_stream(&self) -> Box<dyn Stream<Item = Block> + Send + Unpin> {
        let rx = self.block_sender.subscribe();
        Box::new(
            tokio_stream::wrappers::BroadcastStream::new(rx).filter_map(std::result::Result::ok),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> ConsensusConfig {
        ConsensusConfig {
            validator_seeds: vec![0],
            validators: vec![Address::ZERO],
            private_key_path: "/tmp/nonexistent".into(),
            validator_seed: Some(0),
            listen_addr: "0.0.0.0:26656".to_string(),
            dialable_addr: None,
            p2p_port: 26656,
            peers: vec![],
            leader_timeout_ms: 2000,
            notarization_timeout_ms: 3000,
            nullify_retry_ms: 10000,
            namespace: "test".to_string(),
            storage_dir: "/tmp/consensus-test".into(),
            allow_private_ips: true,
        }
    }

    #[test]
    fn test_new_consensus() {
        let config = test_config();
        let consensus = Consensus::new(config);
        assert!(consensus.is_ok());
    }

    #[test]
    fn test_empty_validators_fails() {
        let mut config = test_config();
        config.validators = vec![];
        let consensus = Consensus::new(config);
        assert!(consensus.is_err());
    }

    #[test]
    fn test_leader_rotation() {
        let mut config = test_config();
        config.validators = vec![
            Address::repeat_byte(0x01),
            Address::repeat_byte(0x02),
            Address::repeat_byte(0x03),
        ];
        let consensus = Consensus::new(config).unwrap();

        assert_eq!(consensus.leader_for_height(0), Address::repeat_byte(0x01));
        assert_eq!(consensus.leader_for_height(1), Address::repeat_byte(0x02));
        assert_eq!(consensus.leader_for_height(2), Address::repeat_byte(0x03));
        assert_eq!(consensus.leader_for_height(3), Address::repeat_byte(0x01));
    }
}
