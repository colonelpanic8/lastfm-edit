//! Error type for scrobble-store operations.

/// Errors that can occur while working with the scrobble store.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StoreError {
    /// An error bubbled up from the lastfm-edit client.
    #[error(transparent)]
    LastFm(#[from] lastfm_edit::LastFmError),

    /// Filesystem I/O failure.
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    /// A record or state file could not be (de)serialized.
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    /// The store's on-disk state is inconsistent or unexpectedly shaped.
    #[error("corrupt store state: {0}")]
    Corrupt(String),

    /// An operation referenced a scrobble that is not in the store.
    #[error("scrobble not found: {0}")]
    NotFound(String),

    /// A mirrored edit's original values no longer match the store/upstream; the caller
    /// must re-derive the edit from current state.
    #[error("edit needs rebase: {0}")]
    NeedsRebase(String),

    /// The operation was cancelled before completion.
    #[error("cancelled")]
    Cancelled,
}

/// Convenience alias used throughout the crate.
pub type Result<T> = std::result::Result<T, StoreError>;
