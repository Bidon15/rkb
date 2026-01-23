//! Celestia-native PoA EVM sequencer.
//!
//! This binary orchestrates:
//! - PoA consensus (block production)
//! - Celestia DA (blob submission + finality tracking)
//! - reth execution (Engine API)

#![warn(missing_docs)]

mod cli;
mod config;
mod sequencer;
mod simplex_sequencer;

use clap::Parser;
use eyre::Result;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

use crate::cli::Cli;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::registry().with(fmt::layer()).with(EnvFilter::from_default_env()).init();

    let cli = Cli::parse();
    cli.run().await
}
