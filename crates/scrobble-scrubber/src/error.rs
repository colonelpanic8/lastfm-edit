//! Error type for scrubber operations.

/// Errors that can occur while planning or executing scrubs.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ScrubberError {
    /// Bubbled up from the scrobble store (including mirrored-edit failures and
    /// rate-limit propagation from non-blocking clients).
    #[error(transparent)]
    Store(#[from] scrobble_store::StoreError),

    /// A rewrite rule failed to compile or apply.
    #[error(transparent)]
    Rewrite(#[from] crate::rewrite::RewriteError),

    /// Filesystem I/O failure in scrubber state.
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    /// State or intent (de)serialization failure.
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    /// An action provider failed.
    #[error("provider '{provider}' failed: {message}")]
    Provider { provider: String, message: String },

    /// A referenced intent/rule/record was not found.
    #[error("not found: {0}")]
    NotFound(String),

    /// An operation was attempted against an intent in the wrong state.
    #[error("invalid state transition: {0}")]
    InvalidState(String),

    /// The operation was cancelled before completion.
    #[error("cancelled")]
    Cancelled,
}

/// Convenience alias used throughout the crate.
pub type Result<T> = std::result::Result<T, ScrubberError>;
