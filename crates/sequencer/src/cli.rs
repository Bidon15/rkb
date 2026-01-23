//! Command-line interface.

use std::path::PathBuf;

use bech32::{Bech32, Hrp};
use clap::{Parser, Subcommand};
use eyre::Result;
use k256::ecdsa::SigningKey;
use ripemd::Ripemd160;
use sha2::{Digest, Sha256};
use tracing::info;

use crate::config;
use crate::simplex_sequencer::SimplexSequencer;

/// Celestia-native PoA EVM sequencer.
#[derive(Parser)]
#[command(name = "sequencer")]
#[command(version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

/// Available subcommands.
#[derive(Subcommand)]
enum Commands {
    /// Run the sequencer node.
    Run {
        /// Path to configuration file.
        #[arg(short, long, default_value = "config.toml")]
        config: PathBuf,

        /// Log level (trace, debug, info, warn, error).
        #[arg(short, long, default_value = "info")]
        log_level: String,
    },

    /// Initialize a new configuration file.
    Init {
        /// Output path for the config file.
        #[arg(short, long, default_value = "config.toml")]
        output: PathBuf,
    },

    /// Generate a new validator key.
    Keygen {
        /// Output path for the key file.
        #[arg(short, long, default_value = "validator.key")]
        output: PathBuf,
    },

    /// Derive Celestia address from a private key file.
    CelestiaAddress {
        /// Path to Celestia key file (hex-encoded secp256k1 private key).
        #[arg(short, long)]
        key: PathBuf,
    },
}

impl Cli {
    /// Execute the CLI command.
    ///
    /// # Errors
    ///
    /// Returns an error if the command fails.
    pub async fn run(self) -> Result<()> {
        match self.command {
            Some(Commands::Run { config, log_level }) => {
                Self::run_sequencer(config, log_level).await
            }
            Some(Commands::Init { output }) => Self::init_config(output),
            Some(Commands::Keygen { output }) => Self::generate_key(output),
            Some(Commands::CelestiaAddress { key }) => Self::celestia_address(key),
            None => {
                // Default: run with default config
                Self::run_sequencer(PathBuf::from("config.toml"), "info".to_string()).await
            }
        }
    }

    async fn run_sequencer(config: PathBuf, _log_level: String) -> Result<()> {
        info!(config = %config.display(), "Loading configuration");

        let cfg = config::load(&config)?;
        let sequencer = SimplexSequencer::new(cfg);

        info!("Starting PoA sequencer");
        sequencer.run().await
    }

    fn init_config(output: PathBuf) -> Result<()> {
        info!(path = %output.display(), "Generating default configuration");

        let cfg = config::default_config();
        let toml_str = toml::to_string_pretty(&cfg)?;
        std::fs::write(&output, toml_str)?;

        info!("Configuration written");
        Ok(())
    }

    fn generate_key(output: PathBuf) -> Result<()> {
        use rand::RngCore;

        info!(path = %output.display(), "Generating validator key");

        // Generate random 32-byte seed
        let mut seed = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut seed);

        // Write as hex
        let hex_key = hex::encode(seed);
        std::fs::write(&output, &hex_key)?;

        info!(path = %output.display(), "Ed25519 validator key generated");
        Ok(())
    }

    /// Derive Celestia bech32 address from a secp256k1 private key file.
    ///
    /// The key file should contain a hex-encoded 32-byte private key.
    /// The address is derived as: bech32("celestia", ripemd160(sha256(compressed_pubkey)))
    fn celestia_address(key_path: PathBuf) -> Result<()> {
        // Read hex-encoded private key
        let key_hex = std::fs::read_to_string(&key_path)
            .map_err(|e| eyre::eyre!("failed to read key file: {e}"))?;
        let key_bytes = hex::decode(key_hex.trim())
            .map_err(|e| eyre::eyre!("invalid hex in key file: {e}"))?;

        // Parse as secp256k1 signing key
        let signing_key = SigningKey::from_slice(&key_bytes)
            .map_err(|e| eyre::eyre!("invalid secp256k1 key: {e}"))?;

        // Get compressed public key (33 bytes)
        let verifying_key = signing_key.verifying_key();
        let pubkey_bytes = verifying_key.to_sec1_bytes();

        // SHA256 -> RIPEMD160
        let sha256_hash = Sha256::digest(&pubkey_bytes);
        let ripemd_hash = Ripemd160::digest(sha256_hash);

        // bech32 encode with "celestia" prefix
        let hrp = Hrp::parse("celestia").expect("valid hrp");
        let address = bech32::encode::<Bech32>(hrp, &ripemd_hash)
            .map_err(|e| eyre::eyre!("bech32 encoding failed: {e}"))?;

        println!("{address}");
        Ok(())
    }
}
