//! # scrobble-store
//!
//! A synchronizable local mirror of a user's Last.fm scrobble history.
//!
//! The store keeps scrobbles in append-only, git-friendly flat files (the source of truth),
//! tracks which time ranges are known to agree with Last.fm via a [`CoverageMap`], emits
//! typed [`SyncEvent`]s so consumers can react to incremental progress (including rate-limit
//! pauses), and mirrors edits to both Last.fm and the local store through a durable edit log.

pub mod coverage;
pub mod edits;
pub mod error;
pub mod id;
pub mod record;
pub mod source;
pub mod storage;
pub mod sync;

pub use coverage::{CoverageChange, CoverageMap, Segment};
pub use edits::{
    EditEventKind, EditLogEntry, EditLogEvent, EditOp, EditOutcome, EditState, MirroredEditor,
};
pub use error::{Result, StoreError};
pub use id::ScrobbleId;
pub use record::{Provenanced, RecordSource, ScrobbleRecord};
pub use source::{ApiSource, ScrapeSource, ScrobbleSource, SourcePage};
pub use storage::{
    AlbumCount, AppendStats, ArtistCount, FsStorage, MemoryStorage, Storage, SyncState, TrackCount,
};
pub use sync::{
    PauseReason, SyncEngine, SyncEvent, SyncEventBus, SyncEventReceiver, SyncMode, SyncOptions,
    SyncStats, VerifyReport,
};
