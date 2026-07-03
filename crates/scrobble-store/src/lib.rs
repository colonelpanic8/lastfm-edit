//! Offline synchronization and local storage for Last.fm scrobbles.
//!
//! `scrobble-store` maintains a local mirror of a user's scrobbles that can be
//! synced from Last.fm via the [`lastfm-edit`](lastfm_edit) client, queried and
//! edited offline, and replayed back to Last.fm.
//!
//! This crate is currently a scaffold; the storage and sync APIs are still being
//! designed.

use lastfm_edit::Track;

/// Errors that can occur while storing or synchronizing scrobbles.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// Error originating from the underlying Last.fm client.
    #[error(transparent)]
    LastFm(#[from] lastfm_edit::LastFmError),
}

/// A local, offline-capable store of scrobbles.
///
/// This is a placeholder for the forthcoming storage implementation.
#[derive(Debug, Default)]
pub struct ScrobbleStore {
    tracks: Vec<Track>,
}

impl ScrobbleStore {
    /// Create a new, empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of tracks currently held in the store.
    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    /// Whether the store holds no tracks.
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }
}
