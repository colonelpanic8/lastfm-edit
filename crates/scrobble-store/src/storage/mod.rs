//! Storage abstraction: the source of truth for scrobbles, coverage, and sync state.
//!
//! [`FsStorage`] is the real implementation — append-only JSONL flat files in a git-friendly
//! directory, with a derived (disposable, gitignored) SQLite index answering the query
//! methods. [`MemoryStorage`] implements the same trait for tests and ephemeral use.

mod fs;
mod index;
mod memory;

pub use fs::FsStorage;
pub use memory::MemoryStorage;

use crate::coverage::CoverageMap;
use crate::error::Result;
use crate::id::ScrobbleId;
use crate::record::ScrobbleRecord;
use serde::{Deserialize, Serialize};
use std::ops::Range;

/// Outcome of an append batch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AppendStats {
    /// Records whose id was not present before.
    pub new: u64,
    /// Records that superseded an existing id with different content.
    pub updated: u64,
    /// Records identical to (or older than) what the store already held; not written.
    pub unchanged: u64,
}

impl AppendStats {
    pub fn total_written(&self) -> u64 {
        self.new + self.updated
    }
}

/// Persistent sync engine state that is not derivable from records or coverage.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncState {
    /// Timestamp of the user's first scrobble, once discovered by a backfill that exhausted
    /// history. Lets gap computations stop probing before the beginning of time.
    pub history_start_uts: Option<u64>,
    /// When a sync last completed (any mode).
    pub last_sync_at: Option<u64>,
}

/// An artist with an aggregate scrobble count.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtistCount {
    pub artist: String,
    pub count: u64,
}

/// A track (artist + title) with an aggregate scrobble count.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackCount {
    pub artist: String,
    pub track: String,
    pub count: u64,
}

/// An album (artist + album) with an aggregate scrobble count.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlbumCount {
    pub artist: String,
    pub album: String,
    pub count: u64,
}

/// The storage backend contract.
///
/// Writes are last-write-wins by [`ScrobbleId`] with `fetched_at` as the tiebreaker;
/// tombstoned records (`deleted: true`) are excluded from range reads and aggregates but
/// still returned by [`Storage::get_scrobble`] so callers can distinguish "deleted" from
/// "never seen".
#[async_trait::async_trait]
pub trait Storage: Send + Sync {
    // ---- scrobbles -------------------------------------------------------------------

    /// Append a batch of observations. Idempotent: re-appending existing records is
    /// counted as `unchanged` and not written again.
    async fn append_scrobbles(&self, records: &[ScrobbleRecord]) -> Result<AppendStats>;

    /// The current (LWW) view of live scrobbles with `uts` in `range`, ascending by `uts`.
    async fn scrobbles_in_range(&self, range: Range<u64>) -> Result<Vec<ScrobbleRecord>>;

    /// The current (LWW) record for an id, including tombstones.
    async fn get_scrobble(&self, id: &ScrobbleId) -> Result<Option<ScrobbleRecord>>;

    /// Timestamp of the newest live scrobble, if any.
    async fn latest_uts(&self) -> Result<Option<u64>>;

    // ---- coverage & sync state -------------------------------------------------------

    async fn load_coverage(&self) -> Result<CoverageMap>;
    async fn save_coverage(&self, coverage: &CoverageMap) -> Result<()>;

    async fn load_sync_state(&self) -> Result<SyncState>;
    async fn save_sync_state(&self, state: &SyncState) -> Result<()>;

    // ---- edit log ----------------------------------------------------------------------

    /// Append events to the durable edit log.
    async fn append_edit_events(&self, events: &[crate::edits::EditLogEvent]) -> Result<()>;

    /// The edit log folded into per-edit entries, in first-queued order.
    async fn load_edit_log(&self) -> Result<Vec<crate::edits::EditLogEntry>>;

    // ---- maintenance -----------------------------------------------------------------

    /// Rewrite storage keeping only the LWW winner per id (tombstones included — they are
    /// load-bearing for multi-machine merges). Returns the number of superseded lines
    /// dropped. A no-op for backends without redundant representation.
    async fn compact(&self) -> Result<u64>;

    // ---- indexed queries -------------------------------------------------------------

    /// Most-scrobbled artists (live records only), optionally restricted to a time range.
    async fn top_artists(
        &self,
        limit: usize,
        range: Option<Range<u64>>,
    ) -> Result<Vec<ArtistCount>>;

    /// Most-scrobbled tracks, optionally restricted to one artist and/or a time range.
    async fn top_tracks(
        &self,
        artist: Option<&str>,
        limit: usize,
        range: Option<Range<u64>>,
    ) -> Result<Vec<TrackCount>>;

    /// Most-scrobbled albums, optionally restricted to one artist and/or a time range.
    async fn top_albums(
        &self,
        artist: Option<&str>,
        limit: usize,
        range: Option<Range<u64>>,
    ) -> Result<Vec<AlbumCount>>;

    /// Count of live scrobbles, optionally restricted to a time range.
    async fn scrobble_count(&self, range: Option<Range<u64>>) -> Result<u64>;

    /// All live scrobbles of an artist, ascending by `uts`, optionally within a range.
    async fn artist_scrobbles(
        &self,
        artist: &str,
        range: Option<Range<u64>>,
    ) -> Result<Vec<ScrobbleRecord>>;

    /// Drop and rebuild any derived index from the source of truth. A no-op for backends
    /// without derived state.
    async fn reindex(&self) -> Result<()>;
}
