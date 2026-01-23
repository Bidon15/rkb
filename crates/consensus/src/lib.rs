//! `PoA` consensus module.
//!
//! This crate provides block production and ordering for the sequencer
//! using a Proof-of-Authority consensus mechanism.
//!
//! ## Architecture
//!
//! The consensus module provides two modes of operation:
//!
//! 1. **Simple `PoA`** (current): Single-validator block production with round-robin
//!    leader election. This is implemented in [`engine::Consensus`].
//!
//! 2. **Simplex BFT**: Multi-validator BFT consensus using the
//!    commonware-consensus Simplex protocol. The [`application`] module provides
//!    the `CertifiableAutomaton` implementation, and [`network`] provides P2P.
//!    See [`simplex::SimplexConsensus`] for the full BFT engine.

#![warn(missing_docs)]

pub mod application;
mod blocker;
mod engine;
mod error;
pub mod network;
mod reporter;
pub mod runtime;
pub mod simplex;

pub use application::{AppContext, AppDigest, Application, ApplicationConfig, Mailbox};
pub use blocker::ConsensusBlocker;
pub use engine::Consensus;
pub use error::ConsensusError;
pub use network::NetworkConfig;
pub use reporter::ConsensusReporter;
pub use runtime::{on_finalized, start_simplex_runtime, FinalizedCallback, OnFinalized, SimplexRuntimeConfig};
pub use simplex::{SimplexConfig, SimplexConsensus};
use futures::Stream;
use sequencer_types::{Block, Transaction};

/// Result type for consensus operations.
pub type Result<T> = std::result::Result<T, ConsensusError>;

/// Trait for consensus block production.
#[async_trait::async_trait]
pub trait BlockProducer: Send + Sync {
    /// Start the consensus engine.
    ///
    /// # Errors
    ///
    /// Returns an error if the engine fails to start.
    async fn start(&self) -> Result<()>;

    /// Submit a transaction to the mempool.
    ///
    /// # Errors
    ///
    /// Returns an error if the transaction cannot be submitted.
    async fn submit_transaction(&self, tx: Transaction) -> Result<()>;

    /// Get a stream of finalized blocks.
    fn block_stream(&self) -> Box<dyn Stream<Item = Block> + Send + Unpin>;
}
