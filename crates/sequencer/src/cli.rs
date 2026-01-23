//! Command-line interface.

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use eyre::Result;
use tracing::info;

use crate::config;
use crate::sequencer::Sequencer;
use crate::simplex_sequencer::SimplexSequencer;

/// Celestia-native PoA EVM sequencer.
#[derive(Parser)]
#[command(name = "sequencer")]
#[command(version, about, long_about = None)]
pub struct Cli {
    /// Path to configuration file.
    #[arg(short, long, default_value = "config.toml")]
    config: PathBuf,

    /// Log level (trace, debug, info, warn, error).
    #[arg(short, long, default_value = "info")]
    log_level: String,

    #[command(subcommand)]
    command: Option<Commands>,
}

/// Available subcommands.
#[derive(Subcommand)]
enum Commands {
    /// Run the sequencer node (single-validator PoA mode).
    Run,

    /// Run in Simplex BFT mode (multi-validator consensus).
    Simplex,

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
}

impl Cli {
    /// Execute the CLI command.
    ///
    /// # Errors
    ///
    /// Returns an error if the command fails.
    pub async fn run(self) -> Result<()> {
        match self.command {
            Some(Commands::Run) | None => self.run_sequencer().await,
            Some(Commands::Simplex) => self.run_simplex().await,
            Some(Commands::Init { output }) => Self::init_config(output),
            Some(Commands::Keygen { output }) => Self::generate_key(output),
        }
    }

    async fn run_sequencer(self) -> Result<()> {
        info!(config = %self.config.display(), "Loading configuration");

        let cfg = config::load(&self.config)?;
        let sequencer = Sequencer::new(cfg).await?;

        info!("Starting single-validator PoA sequencer");
        sequencer.run().await
    }

    async fn run_simplex(self) -> Result<()> {
        info!(config = %self.config.display(), "Loading configuration");

        let cfg = config::load(&self.config)?;
        let sequencer = SimplexSequencer::new(cfg);

        info!("Starting Simplex BFT sequencer");
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
}
