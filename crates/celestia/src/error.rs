//! Celestia error types.

/// Errors that can occur in the Celestia module.
#[derive(Debug, thiserror::Error)]
pub enum CelestiaError {
    /// Failed to connect to Celestia.
    #[error("failed to connect to Celestia: {0}")]
    ConnectionFailed(String),

    /// Failed to submit blob.
    #[error("failed to submit blob: {0}")]
    SubmitFailed(String),

    /// Failed to submit blob to Celestia.
    #[error("submission failed: {0}")]
    SubmissionFailed(String),

    /// Failed to subscribe to headers.
    #[error("subscription failed: {0}")]
    SubscriptionFailed(String),

    /// Failed to encode block.
    #[error("failed to encode block: {0}")]
    EncodeFailed(String),

    /// Failed to decode block.
    #[error("failed to decode block: {0}")]
    DecodeFailed(String),

    /// Blob not found.
    #[error("blob not found at height {height}")]
    BlobNotFound {
        /// Celestia height.
        height: u64,
    },

    /// Invalid configuration.
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),

    /// Internal error.
    #[error("internal error: {0}")]
    Internal(String),
}
