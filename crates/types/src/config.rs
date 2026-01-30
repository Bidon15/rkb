//! Configuration types.

use std::path::PathBuf;

use alloy_primitives::Address;
use serde::{Deserialize, Serialize};

/// Block timing mode for proposal generation.
///
/// Controls how the sequencer handles Ethereum's timestamp constraint
/// (`block.timestamp > parent.timestamp`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum BlockTiming {
    /// Vanilla Reth mode: Only propose when wall clock > parent timestamp.
    /// Results in ~1 block per second. No Reth modifications needed.
    #[default]
    Vanilla,
    /// Subsecond mode: Propose when wall clock >= parent timestamp.
    /// Allows multiple blocks per second with same timestamp.
    /// Requires forked Reth with relaxed timestamp validation.
    Subsecond,
}

/// Default configuration values.
///
/// All configuration defaults are centralized here for visibility and reuse in tests/docs.
pub mod defaults {
    use std::path::PathBuf;

    // === Consensus defaults ===

    /// Default P2P port.
    pub const P2P_PORT: u16 = 26656;

    /// Default leader timeout in milliseconds.
    pub const LEADER_TIMEOUT_MS: u64 = 2000;

    /// Default notarization timeout in milliseconds.
    pub const NOTARIZATION_TIMEOUT_MS: u64 = 3000;

    /// Default nullify retry timeout in milliseconds.
    pub const NULLIFY_RETRY_MS: u64 = 10000;

    /// Default: allow private IPs (useful for local testing).
    pub const ALLOW_PRIVATE_IPS: bool = true;

    // === Celestia defaults ===

    /// Default gas price for Celestia submissions (in utia).
    pub const GAS_PRICE: f64 = 0.002;

    /// Default batch submission interval in milliseconds.
    /// 5 seconds - just under Celestia's 6s block time.
    pub const BATCH_INTERVAL_MS: u64 = 5000;

    /// Default maximum batch size in bytes.
    /// 1.5MB - well under Celestia's ~2MB limit.
    pub const MAX_BATCH_SIZE_BYTES: usize = 1_500_000;

    // === Path defaults (functions needed for PathBuf) ===

    /// Default consensus storage directory.
    #[must_use]
    pub fn storage_dir() -> PathBuf {
        PathBuf::from("./consensus-data")
    }

    /// Default Celestia block index path.
    #[must_use]
    pub fn index_path() -> PathBuf {
        PathBuf::from("data/celestia-index")
    }

    /// Default application namespace.
    #[must_use]
    pub fn namespace() -> String {
        "rkb-sequencer".to_string()
    }
}

// === Serde wrapper functions ===
// These call into the defaults module for the actual values.

fn default_p2p_port() -> u16 {
    defaults::P2P_PORT
}

fn default_leader_timeout_ms() -> u64 {
    defaults::LEADER_TIMEOUT_MS
}

fn default_notarization_timeout_ms() -> u64 {
    defaults::NOTARIZATION_TIMEOUT_MS
}

fn default_nullify_retry_ms() -> u64 {
    defaults::NULLIFY_RETRY_MS
}

fn default_allow_private_ips() -> bool {
    defaults::ALLOW_PRIVATE_IPS
}

fn default_namespace() -> String {
    defaults::namespace()
}

fn default_storage_dir() -> PathBuf {
    defaults::storage_dir()
}

fn default_gas_price() -> f64 {
    defaults::GAS_PRICE
}

fn default_batch_interval_ms() -> u64 {
    defaults::BATCH_INTERVAL_MS
}

fn default_max_batch_size_bytes() -> usize {
    defaults::MAX_BATCH_SIZE_BYTES
}

fn default_index_path() -> PathBuf {
    defaults::index_path()
}

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

    /// Block timing mode.
    ///
    /// - `vanilla`: Only build when timestamp can strictly advance (1s minimum blocks).
    ///   Works with unmodified Reth. This is the default.
    /// - `subsecond`: Allow same-second timestamps (sub-second blocks possible).
    ///   Requires forked Reth with relaxed timestamp validation (`>=` instead of `>`).
    #[serde(default)]
    pub block_timing: BlockTiming,
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
