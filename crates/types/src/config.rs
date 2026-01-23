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
/// Supports separate endpoints for reading (bridge node) and submitting (core gRPC).
/// Uses celestia-client (Lumina) for blob operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CelestiaConfig {
    /// Bridge node address for reading blobs (e.g., "https://node.example.com/").
    /// This is the DA node RPC endpoint.
    #[serde(alias = "rpc_endpoint")]
    pub bridge_addr: String,

    /// Auth token for bridge node (empty string if not required).
    #[serde(default)]
    pub bridge_auth_token: String,

    /// Enable TLS for bridge connection.
    #[serde(default)]
    pub bridge_tls_enabled: bool,

    /// Core gRPC address for submitting blobs (e.g., "node.example.com:9090").
    /// This is the consensus node gRPC endpoint for PayForBlobs transactions.
    #[serde(alias = "grpc_endpoint")]
    pub core_grpc_addr: String,

    /// Auth token for core gRPC (used as metadata header).
    #[serde(default)]
    pub core_grpc_auth_token: String,

    /// Enable TLS for core gRPC connection.
    #[serde(default)]
    pub core_grpc_tls_enabled: bool,

    /// Path to Celestia account private key (hex-encoded secp256k1).
    /// This key is used to sign PayForBlobs transactions.
    pub celestia_key_path: PathBuf,

    /// Namespace for this chain's blobs (hex-encoded, max 10 bytes).
    /// Will be auto-padded to Celestia's v0 namespace format.
    pub namespace: String,

    /// Gas price for submissions (in utia).
    #[serde(default = "default_gas_price")]
    pub gas_price: f64,

    /// Batch submission interval in milliseconds.
    ///
    /// Blocks are accumulated and submitted as a batch at this interval.
    /// Set to 0 to submit every block immediately (not recommended).
    /// Default: 5000ms (5 seconds, slightly less than Celestia's 6s block time).
    #[serde(default = "default_batch_interval_ms")]
    pub batch_interval_ms: u64,

    /// Maximum batch size in bytes before triggering early submission.
    ///
    /// If the accumulated batch exceeds this size, it will be submitted
    /// immediately rather than waiting for the batch interval.
    /// Default: 1.5MB (well under Celestia's ~2MB blob limit).
    #[serde(default = "default_max_batch_size_bytes")]
    pub max_batch_size_bytes: usize,

    /// Celestia height where this chain's genesis block was posted.
    ///
    /// New validators use this to sync historical blocks from Celestia DA.
    /// Set to 0 to skip DA sync (only for genesis validators).
    #[serde(default)]
    pub genesis_da_height: u64,

    /// Expected hash of the first sequencer block (optional).
    ///
    /// If set, the sync process will verify the first block matches this hash.
    /// Format: hex-encoded 32-byte hash (e.g., "0x1234...").
    #[serde(default)]
    pub genesis_block_hash: Option<String>,

    /// Path to the block index database.
    ///
    /// The index maps sequencer block heights/hashes to Celestia heights,
    /// enabling efficient lookups without scanning all DA blobs.
    #[serde(default = "default_index_path")]
    pub index_path: PathBuf,
}

fn default_gas_price() -> f64 {
    0.002
}

fn default_batch_interval_ms() -> u64 {
    5000 // 5 seconds - just under Celestia's 6s block time
}

fn default_max_batch_size_bytes() -> usize {
    1_500_000 // 1.5MB - well under Celestia's ~2MB limit
}

fn default_index_path() -> PathBuf {
    PathBuf::from("data/celestia-index")
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
