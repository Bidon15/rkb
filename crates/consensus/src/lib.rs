//! PoA consensus module.
//!
//! This crate provides BFT consensus for the sequencer using the
//! commonware-consensus Simplex protocol.

#![warn(missing_docs)]

pub mod application;
mod error;
mod leader;
pub mod network;
mod reporter;
pub mod runtime;

pub use application::{AppContext, AppDigest, Application, ApplicationConfig, Mailbox, ProposedBlock};
pub use error::ConsensusError;
pub use leader::{is_leader, leader_index};
pub use network::NetworkConfig;
pub use reporter::ConsensusReporter;
pub use runtime::{on_finalized, start_simplex_runtime, FinalizedCallback, OnFinalized, SimplexRuntimeConfig};

/// Result type for consensus operations.
pub type Result<T> = std::result::Result<T, ConsensusError>;
