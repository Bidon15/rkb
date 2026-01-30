//! Configuration loading and validation.

use std::path::Path;

use alloy_primitives::{keccak256, Address};
use commonware_cryptography::{ed25519, Signer};
use eyre::{Context, Result};
use sequencer_types::ChainConfig;

/// Complete sequencer configuration.
pub type Config = ChainConfig;

/// Load configuration from a TOML file.
///
/// # Errors
///
/// Returns an error if the file cannot be read or parsed.
pub fn load(path: &Path) -> Result<Config> {
    let contents = std::fs::read_to_string(path)
        .wrap_err_with(|| format!("failed to read config file: {}", path.display()))?;

    let mut config: Config = toml::from_str(&contents).wrap_err("failed to parse configuration")?;

    // Derive validator addresses from seeds if not explicitly set
    if config.consensus.validators.is_empty() && !config.consensus.validator_seeds.is_empty() {
        config.consensus.validators = config
            .consensus
            .validator_seeds
            .iter()
            .map(|seed| {
                let key = ed25519::PrivateKey::from_seed(*seed);
                derive_address(&key.public_key())
            })
            .collect();
    }

    validate(&config)?;
    Ok(config)
}

/// Derive Ethereum-style address from Ed25519 public key.
fn derive_address(pubkey: &ed25519::PublicKey) -> Address {
    let hash = keccak256(pubkey.as_ref());
    Address::from_slice(&hash[12..])
}

/// Validate the configuration.
///
/// # Errors
///
/// Returns an error if validation fails.
pub fn validate(config: &Config) -> Result<()> {
    validate_chain(config)?;
    validate_consensus(config)?;
    validate_celestia(config)?;
    validate_execution(config)?;
    Ok(())
}

/// Validate chain-level configuration.
fn validate_chain(config: &Config) -> Result<()> {
    if config.chain_id == 0 {
        eyre::bail!("chain_id must be non-zero");
    }
    Ok(())
}

/// Validate consensus configuration.
fn validate_consensus(config: &Config) -> Result<()> {
    if config.consensus.validators.is_empty() {
        eyre::bail!("at least one validator is required (set validator_seeds or validators)");
    }
    Ok(())
}

/// Validate Celestia configuration.
fn validate_celestia(config: &Config) -> Result<()> {
    if config.celestia.bridge_addr.is_empty() {
        eyre::bail!("celestia bridge_addr is required");
    }
    if config.celestia.core_grpc_addr.is_empty() {
        eyre::bail!("celestia core_grpc_addr is required");
    }
    Ok(())
}

/// Validate execution configuration.
fn validate_execution(config: &Config) -> Result<()> {
    if config.execution.reth_url.is_empty() {
        eyre::bail!("reth_url is required");
    }
    Ok(())
}

/// Generate a default configuration.
#[must_use]
pub fn default_config() -> Config {
    let validator_addr = derive_address(&ed25519::PrivateKey::from_seed(0).public_key());

    Config {
        chain_id: 1337,
        consensus: default_consensus(validator_addr),
        celestia: default_celestia(),
        execution: default_execution(),
    }
}

/// Default consensus configuration.
fn default_consensus(validator_addr: Address) -> sequencer_types::ConsensusConfig {
    use sequencer_types::{BlockTiming, ConsensusConfig};

    ConsensusConfig {
        validator_seeds: vec![0],
        validators: vec![validator_addr],
        private_key_path: "validator.key".into(),
        validator_seed: Some(0),
        listen_addr: "0.0.0.0:26656".to_string(),
        dialable_addr: None,
        p2p_port: 26656,
        peers: vec![],
        leader_timeout_ms: 2000,
        notarization_timeout_ms: 3000,
        nullify_retry_ms: 10000,
        namespace: "rkb-sequencer".to_string(),
        storage_dir: "./consensus-data".into(),
        allow_private_ips: true,
        block_timing: BlockTiming::Vanilla,
    }
}

/// Default Celestia configuration.
fn default_celestia() -> sequencer_types::CelestiaConfig {
    use sequencer_types::CelestiaConfig;

    CelestiaConfig {
        bridge_addr: "http://localhost:26658".to_string(),
        bridge_auth_token: String::new(),
        bridge_tls_enabled: false,
        core_grpc_addr: "localhost:9090".to_string(),
        core_grpc_auth_token: String::new(),
        core_grpc_tls_enabled: false,
        celestia_key_path: "celestia.key".into(),
        namespace: "726b62".to_string(), // "rkb" in hex (auto-padded by Celestia)
        gas_price: 0.002,
        batch_interval_ms: 5000,         // 5 seconds
        max_batch_size_bytes: 1_500_000, // 1.5MB
        genesis_da_height: 0,            // 0 = skip DA sync (genesis validators)
        genesis_block_hash: None,
        index_path: "data/celestia-index".into(),
    }
}

/// Default execution configuration.
fn default_execution() -> sequencer_types::ExecutionConfig {
    use sequencer_types::ExecutionConfig;

    ExecutionConfig {
        reth_url: "http://localhost:8551".to_string(),
        jwt_secret_path: "jwt.hex".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_serializes() {
        let config = default_config();
        let toml = toml::to_string_pretty(&config);
        assert!(toml.is_ok());
    }
}
