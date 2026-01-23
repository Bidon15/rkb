//! Configuration types.

use std::path::PathBuf;

use alloy_primitives::Address;
use serde::{Deserialize, Serialize};

/// Top-level chain configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainConfig {
    /// Chain ID.
    pub chain_id: u64,

    /// Consensus configuration.
    pub consensus: ConsensusConfig,

    /// Celestia DA configuration.
    pub celestia: CelestiaConfig,

    /// Execution layer configuration.
    pub execution: ExecutionConfig,
}

/// Consensus configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsensusConfig {
    /// Validator seeds (used to derive ed25519 keys deterministically).
    /// In production, this would be replaced with actual public keys.
    pub validator_seeds: Vec<u64>,

    /// Validator addresses (derived from public keys).
    #[serde(default)]
    pub validators: Vec<Address>,

    /// Path to validator private key (hex-encoded ed25519 seed).
    pub private_key_path: PathBuf,

    /// Our validator seed (for key derivation).
    #[serde(default)]
    pub validator_seed: Option<u64>,

    /// P2P listen address (host:port).
    pub listen_addr: String,

    /// P2P dialable address (how peers reach us). Defaults to listen_addr.
    #[serde(default)]
    pub dialable_addr: Option<String>,

    /// P2P port.
    #[serde(default = "default_p2p_port")]
    pub p2p_port: u16,

    /// Peer addresses to connect to (bootstrappers).
    /// Format: "seed@host:port"
    pub peers: Vec<String>,

    /// Leader timeout in milliseconds.
    #[serde(default = "default_leader_timeout_ms")]
    pub leader_timeout_ms: u64,

    /// Notarization timeout in milliseconds.
    #[serde(default = "default_notarization_timeout_ms")]
    pub notarization_timeout_ms: u64,

    /// Nullify retry timeout in milliseconds.
    #[serde(default = "default_nullify_retry_ms")]
    pub nullify_retry_ms: u64,

    /// Application namespace (for message signing).
    #[serde(default = "default_namespace")]
    pub namespace: String,

    /// Storage directory for consensus state.
    #[serde(default = "default_storage_dir")]
    pub storage_dir: PathBuf,

    /// Allow private IPs for P2P (useful for local testing).
    #[serde(default = "default_allow_private_ips")]
    pub allow_private_ips: bool,
}

fn default_storage_dir() -> PathBuf {
    PathBuf::from("./consensus-data")
}

fn default_allow_private_ips() -> bool {
    true
}

fn default_p2p_port() -> u16 {
    26656
}

fn default_leader_timeout_ms() -> u64 {
    2000
}

fn default_notarization_timeout_ms() -> u64 {
    3000
}

fn default_nullify_retry_ms() -> u64 {
    10000
}

fn default_namespace() -> String {
    "astria-sequencer".to_string()
}

/// Celestia DA configuration.
///
/// Uses celestia-client (Lumina) for direct gRPC blob submission,
/// which doesn't require a bridge node auth token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CelestiaConfig {
    /// RPC endpoint for Celestia node (e.g., "http://localhost:26657").
    /// Used for header subscription and queries.
    pub rpc_endpoint: String,

    /// gRPC endpoint for Celestia node (e.g., "http://localhost:9090").
    /// Used for blob submission via PayForBlobs transaction.
    pub grpc_endpoint: String,

    /// Path to Celestia account private key (hex-encoded secp256k1).
    /// This key is used to sign PayForBlobs transactions.
    pub celestia_key_path: PathBuf,

    /// Namespace for this chain's blobs (hex-encoded).
    pub namespace: String,

    /// Gas price for submissions (in utia).
    #[serde(default = "default_gas_price")]
    pub gas_price: f64,
}

fn default_gas_price() -> f64 {
    0.002
}

/// Execution layer configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionConfig {
    /// Reth Engine API URL.
    pub reth_url: String,

    /// Path to JWT secret for Engine API auth.
    pub jwt_secret_path: PathBuf,
}

impl ChainConfig {
    /// Load configuration from a TOML file.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or parsed.
    pub fn load(path: &std::path::Path) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path).map_err(ConfigError::Io)?;
        toml::from_str(&content).map_err(ConfigError::Parse)
    }
}

/// Configuration error.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// IO error reading config file.
    #[error("failed to read config file: {0}")]
    Io(#[from] std::io::Error),

    /// Parse error in config file.
    #[error("failed to parse config file: {0}")]
    Parse(#[from] toml::de::Error),
}
