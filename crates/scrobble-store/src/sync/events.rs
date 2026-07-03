//! Typed events emitted by synchronization, so consumers (scrobble-scrubber, CLIs, UIs)
//! can react to incremental progress — including understanding *why* sync is paused.

use crate::coverage::CoverageChange;
use serde::{Deserialize, Serialize};
use std::ops::Range;
use tokio::sync::broadcast;

/// What a sync run is trying to accomplish.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyncMode {
    /// Extend coverage from the current frontier to (roughly) now.
    ExtendToPresent,
    /// Fill historical gaps, working backwards.
    Backfill,
    /// Fill a specific set of interior gaps.
    GapFill,
    /// Re-fetch an already-covered range and diff it against the store.
    Verify { range: Range<u64> },
}

/// Why sync is currently not making requests.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PauseReason {
    /// Upstream rate limiting; `until_estimate` is the expected resume time (Unix seconds)
    /// when known.
    RateLimited { until_estimate: Option<u64> },
    /// Client-side backoff/pacing between requests.
    Backoff { delay_ms: u64 },
}

/// Cumulative counters for a sync run.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncStats {
    pub pages_fetched: u64,
    pub scrobbles_new: u64,
    pub scrobbles_updated: u64,
    pub scrobbles_unchanged: u64,
    /// Seconds of timeline newly marked covered by this run.
    pub seconds_covered: u64,
}

/// Events broadcast during synchronization and mirrored editing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SyncEvent {
    SyncStarted {
        mode: SyncMode,
    },
    /// A page of upstream data was fetched for `window`.
    PageFetched {
        window: Range<u64>,
        page: u32,
        count: usize,
    },
    /// Scrobbles were appended to the store.
    ScrobblesDiscovered {
        new: u64,
        updated: u64,
        /// Timestamp range observed on this batch (oldest, newest), when non-empty.
        oldest: Option<u64>,
        newest: Option<u64>,
    },
    /// Coverage changed (extension, merge, invalidation, ...).
    CoverageChanged(CoverageChange),
    /// Sync is paused (rate limiting or pacing); a UI should show this instead of
    /// appearing stuck.
    SyncPaused {
        reason: PauseReason,
    },
    SyncResumed,
    /// A mirrored edit was durably queued.
    EditQueued {
        edit_id: String,
    },
    /// A mirrored edit was applied upstream and reflected locally.
    EditApplied {
        edit_id: String,
    },
    /// A mirrored edit attempt failed.
    EditFailed {
        edit_id: String,
        error: String,
        will_retry: bool,
    },
    SyncCompleted {
        stats: SyncStats,
    },
    SyncFailed {
        error: String,
    },
}

/// Receiver half for [`SyncEvent`] subscriptions.
pub type SyncEventReceiver = broadcast::Receiver<SyncEvent>;

/// A broadcast sender with subscribe/emit conveniences, shared by the sync engine and the
/// mirrored editor so consumers get one ordered event stream.
#[derive(Clone, Debug)]
pub struct SyncEventBus {
    tx: broadcast::Sender<SyncEvent>,
}

impl SyncEventBus {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(256);
        Self { tx }
    }

    pub fn subscribe(&self) -> SyncEventReceiver {
        self.tx.subscribe()
    }

    /// Emit an event; delivery is best-effort (no subscribers is not an error).
    pub fn emit(&self, event: SyncEvent) {
        let _ = self.tx.send(event);
    }
}

impl Default for SyncEventBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn events_reach_subscribers() {
        let bus = SyncEventBus::new();
        let mut rx = bus.subscribe();
        bus.emit(SyncEvent::SyncStarted {
            mode: SyncMode::ExtendToPresent,
        });
        assert_eq!(
            rx.recv().await.unwrap(),
            SyncEvent::SyncStarted {
                mode: SyncMode::ExtendToPresent
            }
        );
    }

    #[test]
    fn emit_without_subscribers_is_fine() {
        let bus = SyncEventBus::new();
        bus.emit(SyncEvent::SyncResumed);
    }

    #[test]
    fn events_serialize() {
        let event = SyncEvent::SyncPaused {
            reason: PauseReason::RateLimited {
                until_estimate: Some(1_700_000_000),
            },
        };
        let json = serde_json::to_string(&event).unwrap();
        let back: SyncEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back, event);
    }
}
