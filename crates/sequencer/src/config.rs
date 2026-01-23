//! Configuration loading and validation.

use std::path::Path;

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

    let config: Config = toml::from_str(&contents).wrap_err("failed to parse configuration")?;

    validate(&config)?;
    Ok(config)
}

/// Validate the configuration.
///
/// # Errors
///
/// Returns an error if validation fails.
pub fn validate(config: &Config) -> Result<()> {
    // Validate chain config
    if config.chain_id == 0 {
        eyre::bail!("chain_id must be non-zero");
    }

    // Validate consensus config
    if config.consensus.validators.is_empty() {
        eyre::bail!("at least one validator is required");
    }

    // Validate Celestia config
    if config.celestia.rpc_endpoint.is_empty() {
        eyre::bail!("celestia rpc_endpoint is required");
    }

    if config.celestia.grpc_endpoint.is_empty() {
        eyre::bail!("celestia grpc_endpoint is required");
    }

    // Validate execution config
    if config.execution.reth_url.is_empty() {
        eyre::bail!("reth_url is required");
    }

    Ok(())
}

/// Generate a default configuration.
#[must_use]
pub fn default_config() -> Config {
    use alloy_primitives::Address;
    use sequencer_types::{CelestiaConfig, ConsensusConfig, ExecutionConfig};

    Config {
        chain_id: 1337,
        consensus: ConsensusConfig {
            validator_seeds: vec![0],
            validators: vec![Address::ZERO],
            private_key_path: "validator.key".into(),
            validator_seed: Some(0),
            listen_addr: "0.0.0.0:26656".to_string(),
            dialable_addr: None,
            p2p_port: 26656,
            peers: vec![],
            leader_timeout_ms: 2000,
            notarization_timeout_ms: 3000,
            nullify_retry_ms: 10000,
            namespace: "astria-sequencer".to_string(),
            storage_dir: "./consensus-data".into(),
            allow_private_ips: true,
        },
        celestia: CelestiaConfig {
            rpc_endpoint: "http://localhost:26657".to_string(),
            grpc_endpoint: "http://localhost:9090".to_string(),
            celestia_key_path: "celestia.key".into(),
            namespace: "00000000000000000000000000000000000000617374726961".to_string(), // "astria" padded
            gas_price: 0.002,
        },
        execution: ExecutionConfig {
            reth_url: "http://localhost:8551".to_string(),
            jwt_secret_path: "jwt.hex".into(),
        },
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
