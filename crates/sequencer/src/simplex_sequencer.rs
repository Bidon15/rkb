//! Simplex BFT sequencer.
//!
//! Uses commonware-consensus Simplex protocol for multi-validator BFT consensus.
//! The leader for each block submits finalized blocks to Celestia.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use alloy_primitives::Address;
use commonware_cryptography::{ed25519, Signer};
use eyre::{Context, Result};
use tokio::sync::{mpsc, RwLock};
use tracing::{error, info, warn};

use sequencer_celestia::{CelestiaClient, DataAvailability, FinalityConfirmation};
use sequencer_consensus::{on_finalized, start_simplex_runtime, SimplexRuntimeConfig};
use sequencer_execution::ExecutionClient;
use sequencer_types::{ConsensusConfig, Transaction};

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
    /// 3. Celestia finality tracker
    /// 4. Leader-only blob submission on finalization
    ///
    /// # Errors
    ///
    /// Returns an error if any component fails to start.
    pub async fn run(self) -> Result<()> {
        info!(chain_id = self.config.chain_id, "Simplex sequencer starting");

        // Create finality channel for Celestia confirmations
        let (finality_tx, mut finality_rx) = mpsc::channel::<FinalityConfirmation>(100);

        // Initialize Celestia client
        let celestia = Arc::new(
            CelestiaClient::new(self.config.celestia.clone(), finality_tx)
                .await
                .wrap_err("failed to create Celestia client")?,
        );

        // Start Celestia finality tracker in background
        let celestia_finality = Arc::clone(&celestia);
        tokio::spawn(async move {
            if let Err(e) = celestia_finality.run_finality_tracker().await {
                error!(error = %e, "Celestia finality tracker failed");
            }
        });
        info!("Celestia finality tracker started");

        // Initialize execution client
        let _execution = ExecutionClient::new(self.config.execution.clone())?;

        // Build Simplex runtime config
        let runtime_config = build_runtime_config(&self.config.consensus)
            .wrap_err("failed to build Simplex runtime config")?;

        info!(
            validators = runtime_config.validators.len(),
            listen_addr = %runtime_config.listen_addr,
            "Starting Simplex BFT consensus"
        );

        // Create shared mempool
        let mempool: Arc<RwLock<Vec<Transaction>>> = Arc::new(RwLock::new(Vec::new()));

        // Create finalization callback - leader submits to Celestia
        let celestia_submit = Arc::clone(&celestia);
        let callback = on_finalized(move |block, is_leader| {
            let celestia = Arc::clone(&celestia_submit);
            async move {
                if is_leader {
                    info!(
                        height = block.height(),
                        tx_count = block.tx_count(),
                        "Leader submitting block to Celestia"
                    );

                    match celestia.submit_block(&block).await {
                        Ok(submission) => {
                            info!(
                                block_height = block.height(),
                                celestia_height = submission.celestia_height,
                                "Block submitted to Celestia"
                            );

                            // Track finality
                            celestia.track_finality(block.hash(), submission);
                        }
                        Err(e) => {
                            error!(
                                block_height = block.height(),
                                error = %e,
                                "Failed to submit block to Celestia"
                            );
                        }
                    }
                } else {
                    info!(
                        height = block.height(),
                        tx_count = block.tx_count(),
                        "Block finalized (not leader, skipping Celestia submission)"
                    );
                }

                Ok(())
            }
        });

        // Spawn the Simplex runtime in a separate thread (it blocks)
        let runtime_handle = std::thread::spawn(move || {
            if let Err(e) = start_simplex_runtime(runtime_config, mempool, callback) {
                error!(error = %e, "Simplex runtime failed");
            }
        });

        // Handle finality confirmations and shutdown
        loop {
            tokio::select! {
                Some(confirmation) = finality_rx.recv() => {
                    info!(
                        block_height = confirmation.block_height,
                        celestia_height = confirmation.celestia_height,
                        latency_ms = confirmation.latency_ms(),
                        "Block finalized on Celestia"
                    );
                }

                _ = tokio::signal::ctrl_c() => {
                    warn!("Shutdown signal received");
                    break;
                }
            }
        }

        // Wait for runtime thread to finish
        drop(runtime_handle);
        info!("Simplex sequencer shutting down");

        Ok(())
    }
}

/// Build the Simplex runtime configuration from chain config.
fn build_runtime_config(consensus: &ConsensusConfig) -> Result<SimplexRuntimeConfig> {
    // Load private key
    let private_key = load_private_key(&consensus.private_key_path)?;
    let our_pubkey = private_key.public_key();

    // Derive validator public keys from seeds
    let validators: Vec<ed25519::PublicKey> = consensus
        .validator_seeds
        .iter()
        .map(|seed| ed25519::PrivateKey::from_seed(*seed).public_key())
        .collect();

    // Check we're in the validator set
    if !validators.contains(&our_pubkey) {
        eyre::bail!("our public key is not in the validator set");
    }

    // Derive our address from public key
    let our_address = derive_address(&our_pubkey);

    // Parse listen address
    let listen_addr: SocketAddr = consensus
        .listen_addr
        .parse()
        .wrap_err("invalid listen_addr")?;

    // Parse dialable address (defaults to listen_addr)
    let dialable_addr: SocketAddr = consensus
        .dialable_addr
        .as_ref()
        .map_or_else(|| Ok(listen_addr), |s| s.parse())
        .wrap_err("invalid dialable_addr")?;

    // Parse bootstrappers from peers config
    // Format: "seed@host:port"
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
        leader_timeout: std::time::Duration::from_millis(consensus.leader_timeout_ms),
        notarization_timeout: std::time::Duration::from_millis(consensus.notarization_timeout_ms),
        nullify_retry: std::time::Duration::from_millis(consensus.nullify_retry_ms),
        activity_timeout: 100,
        skip_timeout: 50,
        fetch_timeout: std::time::Duration::from_secs(5),
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
    let seed_bytes = hex::decode(seed_hex)
        .wrap_err("private key must be hex-encoded")?;

    if seed_bytes.len() != 32 {
        eyre::bail!("private key must be exactly 32 bytes (got {})", seed_bytes.len());
    }

    // Use the first 8 bytes as u64 seed for from_seed
    // This is a simplification - in production you'd want proper key loading
    let seed = u64::from_le_bytes(seed_bytes[..8].try_into().unwrap());

    Ok(ed25519::PrivateKey::from_seed(seed))
}

/// Derive Ethereum-style address from Ed25519 public key.
///
/// Takes the last 20 bytes of keccak256(public_key_bytes).
fn derive_address(pubkey: &ed25519::PublicKey) -> Address {
    use alloy_primitives::keccak256;

    let hash = keccak256(pubkey.as_ref());
    Address::from_slice(&hash[12..])
}

/// Parse bootstrapper peers from config strings.
///
/// Format: "seed@host:port" where seed is a u64
fn parse_bootstrappers(peers: &[String]) -> Result<Vec<(ed25519::PublicKey, SocketAddr)>> {
    let mut bootstrappers = Vec::with_capacity(peers.len());

    for peer in peers {
        let parts: Vec<&str> = peer.split('@').collect();
        if parts.len() != 2 {
            eyre::bail!("invalid peer format '{}', expected 'seed@host:port'", peer);
        }

        let seed: u64 = parts[0]
            .parse()
            .wrap_err_with(|| format!("invalid seed in peer '{}'", peer))?;

        let addr: SocketAddr = parts[1]
            .parse()
            .wrap_err_with(|| format!("invalid address in peer '{}'", peer))?;

        let pubkey = ed25519::PrivateKey::from_seed(seed).public_key();
        bootstrappers.push((pubkey, addr));
    }

    Ok(bootstrappers)
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
        let peers = vec!["1@127.0.0.1:26656".to_string(), "2@127.0.0.1:26657".to_string()];

        let bootstrappers = parse_bootstrappers(&peers).unwrap();
        assert_eq!(bootstrappers.len(), 2);

        // Check that public keys are derived correctly
        let expected_key1 = Signer::public_key(&ed25519::PrivateKey::from_seed(1));
        let expected_key2 = Signer::public_key(&ed25519::PrivateKey::from_seed(2));

        assert_eq!(bootstrappers[0].0, expected_key1);
        assert_eq!(bootstrappers[1].0, expected_key2);

        // Check addresses
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
}
