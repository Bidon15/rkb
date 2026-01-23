//! Consensus error types.

/// Errors that can occur in the consensus module.
#[derive(Debug, thiserror::Error)]
pub enum ConsensusError {
    /// Failed to start the consensus engine.
    #[error("failed to start consensus engine: {0}")]
    StartFailed(String),

    /// Failed to submit transaction.
    #[error("failed to submit transaction: {0}")]
    SubmitFailed(String),

    /// Invalid validator configuration.
    #[error("invalid validator configuration: {0}")]
    InvalidConfig(String),

    /// Network error.
    #[error("network error: {0}")]
    Network(String),

    /// Internal error.
    #[error("internal error: {0}")]
    Internal(String),
}
