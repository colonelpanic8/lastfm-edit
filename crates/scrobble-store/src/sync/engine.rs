//! The synchronization engine: extends coverage to the present, backfills history, fills
//! gaps, verifies covered ranges, and stays resumable and rate-limit-aware throughout.
//!
//! ## The confirmation rule
//!
//! A window `[from, to)` may be marked covered only when a single fetch pass, with `to`
//! pinned before page 1, has *fetched past the window*: it either saw a scrobble older than
//! `from` or ran out of pages. Pinning `to` makes pagination deterministic — scrobbles
//! arriving during the pass land above the pin and cannot shift what the pass visits.
//!
//! ## Resumability
//!
//! After every page, the partial coverage `[oldest_on_page + 1, frontier)` is persisted.
//! A crashed or cancelled pass therefore leaves a truthful coverage map; the next run
//! computes the residual gap and its first fetch re-includes any same-second ties that
//! straddled the interruption (the `oldest + 1` rule).

use crate::coverage::{CoverageMap, Segment};
use crate::error::{Result, StoreError};
use crate::record::ScrobbleRecord;
use crate::source::ScrobbleSource;
use crate::storage::Storage;
use crate::sync::events::{
    PauseReason, SyncEvent, SyncEventBus, SyncEventReceiver, SyncMode, SyncStats,
};
use lastfm_edit::RateLimitState;
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Tuning knobs for the engine.
pub struct SyncOptions {
    /// Distance below "now" at which the `to` pin is placed, absorbing clock skew and
    /// scrobbles that are still materializing upstream. The margin is picked up by the
    /// next sync.
    pub safety_margin_secs: u64,
    /// Optional cap on pages fetched per engine call (across all windows). `None` = no cap.
    pub max_pages: Option<u32>,
}

impl Default for SyncOptions {
    fn default() -> Self {
        Self {
            safety_margin_secs: 60,
            max_pages: None,
        }
    }
}

/// Outcome of a [`SyncEngine::verify`] pass.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VerifyReport {
    /// Records fetched from upstream within the range.
    pub upstream_count: u64,
    /// New or changed records written to the store.
    pub written: u64,
    /// Local records no longer present upstream, now tombstoned.
    pub tombstoned: u64,
}

type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

fn system_clock() -> Clock {
    Arc::new(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    })
}

/// Drives synchronization between a [`ScrobbleSource`] (the upstream replica) and a
/// [`Storage`] (the local replica). All progress is observable via [`SyncEngine::subscribe`].
pub struct SyncEngine {
    store: Arc<dyn Storage>,
    source: Arc<dyn ScrobbleSource>,
    events: SyncEventBus,
    options: SyncOptions,
    cancelled: Arc<AtomicBool>,
    clock: Clock,
}

impl SyncEngine {
    pub fn new(store: Arc<dyn Storage>, source: Arc<dyn ScrobbleSource>) -> Self {
        Self::with_options(store, source, SyncOptions::default())
    }

    pub fn with_options(
        store: Arc<dyn Storage>,
        source: Arc<dyn ScrobbleSource>,
        options: SyncOptions,
    ) -> Self {
        Self {
            store,
            source,
            events: SyncEventBus::new(),
            options,
            cancelled: Arc::new(AtomicBool::new(false)),
            clock: system_clock(),
        }
    }

    /// Replace the wall clock (tests).
    pub fn with_clock(mut self, clock: impl Fn() -> u64 + Send + Sync + 'static) -> Self {
        self.clock = Arc::new(clock);
        self
    }

    /// Subscribe to progress events.
    pub fn subscribe(&self) -> SyncEventReceiver {
        self.events.subscribe()
    }

    /// Share this engine's event bus (e.g. with a [`MirroredEditor`](crate) so consumers
    /// get one ordered stream).
    pub fn event_bus(&self) -> SyncEventBus {
        self.events.clone()
    }

    /// A handle that cancels in-flight and future engine calls when flipped.
    pub fn cancel_handle(&self) -> Arc<AtomicBool> {
        self.cancelled.clone()
    }

    fn now(&self) -> u64 {
        (self.clock)()
    }

    fn to_pin(&self) -> u64 {
        self.now().saturating_sub(self.options.safety_margin_secs)
    }

    fn check_cancelled(&self) -> Result<()> {
        if self.cancelled.load(Ordering::Relaxed) {
            Err(StoreError::Cancelled)
        } else {
            Ok(())
        }
    }

    /// Extend coverage from the current frontier to (roughly) now.
    ///
    /// With empty coverage this becomes the initial full backfill: the window reaches back
    /// to the known history start (or 0).
    pub async fn extend_to_present(&self) -> Result<SyncStats> {
        let mode = SyncMode::ExtendToPresent;
        self.events.emit(SyncEvent::SyncStarted { mode });
        let result = self.extend_to_present_inner().await;
        self.finish(result).await
    }

    async fn extend_to_present_inner(&self) -> Result<SyncStats> {
        let coverage = self.store.load_coverage().await?;
        let to_pin = self.to_pin();
        let mut stats = SyncStats::default();
        let from = match coverage.last() {
            Some(seg) if seg.end >= to_pin => return Ok(stats), // already current
            Some(seg) => seg.end,
            None => {
                let sync_state = self.store.load_sync_state().await?;
                sync_state.history_start_uts.unwrap_or(0)
            }
        };
        let completed = self.fetch_pass(from..to_pin, &mut stats).await?;
        if completed {
            self.note_history_start_if_exhausted(from).await?;
        }
        Ok(stats)
    }

    /// Fill historical gaps, newest first, optionally stopping at `until` (a lower bound on
    /// how far back to reach). Resumable: interrupt it at any point and the next call
    /// continues from persisted coverage.
    pub async fn backfill(&self, until: Option<u64>) -> Result<SyncStats> {
        self.events.emit(SyncEvent::SyncStarted {
            mode: SyncMode::Backfill,
        });
        let result = self.backfill_inner(until).await;
        self.finish(result).await
    }

    async fn backfill_inner(&self, until: Option<u64>) -> Result<SyncStats> {
        let sync_state = self.store.load_sync_state().await?;
        let floor = until.or(sync_state.history_start_uts).unwrap_or(0);
        let mut stats = SyncStats::default();
        loop {
            let coverage = self.store.load_coverage().await?;
            let gaps = coverage.gaps(floor..self.to_pin());
            let Some(gap) = gaps.last().cloned() else {
                break; // fully covered down to the floor
            };
            let completed = self.fetch_pass(gap.clone(), &mut stats).await?;
            if completed {
                self.note_history_start_if_exhausted(gap.start).await?;
            }
            if self.pages_exhausted(&stats) {
                break;
            }
        }
        Ok(stats)
    }

    /// Fill gaps inside `within` (defaults to the full known timeline), oldest data last.
    pub async fn fill_gaps(&self, within: Option<Range<u64>>) -> Result<SyncStats> {
        self.events.emit(SyncEvent::SyncStarted {
            mode: SyncMode::GapFill,
        });
        let result = self.fill_gaps_inner(within).await;
        self.finish(result).await
    }

    async fn fill_gaps_inner(&self, within: Option<Range<u64>>) -> Result<SyncStats> {
        let sync_state = self.store.load_sync_state().await?;
        let within =
            within.unwrap_or_else(|| sync_state.history_start_uts.unwrap_or(0)..self.to_pin());
        let mut stats = SyncStats::default();
        loop {
            let coverage = self.store.load_coverage().await?;
            let gaps = coverage.gaps(within.clone());
            let Some(gap) = gaps.last().cloned() else {
                break;
            };
            let _completed = self.fetch_pass(gap, &mut stats).await?;
            if self.pages_exhausted(&stats) {
                break;
            }
        }
        Ok(stats)
    }

    /// Re-fetch `range` from upstream and reconcile the store with it: append what changed,
    /// tombstone local records that no longer exist upstream, and (re-)cover the range.
    /// This is the remedy for out-of-band edits made directly on the website.
    pub async fn verify(&self, range: Range<u64>) -> Result<VerifyReport> {
        self.events.emit(SyncEvent::SyncStarted {
            mode: SyncMode::Verify {
                range: range.clone(),
            },
        });
        let result = self.verify_inner(range).await;
        match result {
            Ok(report) => {
                self.events.emit(SyncEvent::SyncCompleted {
                    stats: SyncStats::default(),
                });
                Ok(report)
            }
            Err(err) => {
                self.events.emit(SyncEvent::SyncFailed {
                    error: err.to_string(),
                });
                Err(err)
            }
        }
    }

    async fn verify_inner(&self, range: Range<u64>) -> Result<VerifyReport> {
        let range = range.start..range.end.min(self.to_pin());
        let mut report = VerifyReport::default();

        // Fetch the entire window upstream first (no partial coverage claims here: a verify
        // is all-or-nothing).
        let mut upstream: Vec<ScrobbleRecord> = Vec::new();
        let mut page = 1;
        loop {
            self.check_cancelled()?;
            let source_page = self.fetch_page_patiently(&range, page).await?;
            let mut oldest: Option<u64> = None;
            for track in &source_page.tracks {
                if let Some(rec) =
                    ScrobbleRecord::from_track(track, self.source.record_source(), self.now())
                {
                    oldest = Some(oldest.map_or(rec.uts, |o: u64| o.min(rec.uts)));
                    if range.contains(&rec.uts) {
                        upstream.push(rec);
                    }
                }
            }
            let past_boundary = oldest.is_some_and(|o| o < range.start);
            if !source_page.has_next || past_boundary {
                break;
            }
            page += 1;
        }
        report.upstream_count = upstream.len() as u64;

        // Reconcile.
        let local = self.store.scrobbles_in_range(range.clone()).await?;
        let upstream_ids: std::collections::HashSet<_> =
            upstream.iter().map(|r| r.id.clone()).collect();
        let stats = self.store.append_scrobbles(&upstream).await?;
        report.written = stats.total_written();
        let now = self.now();
        let tombstones: Vec<ScrobbleRecord> = local
            .into_iter()
            .filter(|rec| !upstream_ids.contains(&rec.id))
            .map(|rec| rec.into_tombstone(now))
            .collect();
        report.tombstoned = tombstones.len() as u64;
        if !tombstones.is_empty() {
            self.store.append_scrobbles(&tombstones).await?;
        }

        // The range is now agreed; mark it covered.
        let mut coverage = self.store.load_coverage().await?;
        let changes = coverage.insert(Segment::new(range.start, range.end, now));
        self.store.save_coverage(&coverage).await?;
        for change in changes {
            self.events.emit(SyncEvent::CoverageChanged(change));
        }
        Ok(report)
    }

    /// Invalidate a range of coverage so the next sync re-fetches it.
    pub async fn invalidate(&self, range: Range<u64>) -> Result<()> {
        let mut coverage = self.store.load_coverage().await?;
        let changes = coverage.subtract(range);
        self.store.save_coverage(&coverage).await?;
        for change in changes {
            self.events.emit(SyncEvent::CoverageChanged(change));
        }
        Ok(())
    }

    // ---- internals ---------------------------------------------------------------------

    async fn finish(&self, result: Result<SyncStats>) -> Result<SyncStats> {
        match result {
            Ok(stats) => {
                let mut sync_state = self.store.load_sync_state().await?;
                sync_state.last_sync_at = Some(self.now());
                self.store.save_sync_state(&sync_state).await?;
                self.events.emit(SyncEvent::SyncCompleted {
                    stats: stats.clone(),
                });
                Ok(stats)
            }
            Err(err) => {
                self.events.emit(SyncEvent::SyncFailed {
                    error: err.to_string(),
                });
                Err(err)
            }
        }
    }

    fn pages_exhausted(&self, stats: &SyncStats) -> bool {
        self.options
            .max_pages
            .is_some_and(|cap| stats.pages_fetched >= cap as u64)
    }

    /// One deterministic pass over `[window.start, window.end)`. See module docs for the
    /// confirmation rule and the per-page persistence that makes this resumable.
    ///
    /// Returns `true` when the window was fully confirmed, `false` when the pass stopped
    /// early on the page budget (partial coverage is persisted either way).
    async fn fetch_pass(&self, window: Range<u64>, stats: &mut SyncStats) -> Result<bool> {
        if window.start >= window.end {
            return Ok(true);
        }
        let mut coverage = self.store.load_coverage().await?;
        let mut frontier = window.end;
        let mut page: u32 = 1;

        loop {
            self.check_cancelled()?;
            let source_page = self.fetch_page_patiently(&window, page).await?;
            stats.pages_fetched += 1;
            self.events.emit(SyncEvent::PageFetched {
                window: window.clone(),
                page,
                count: source_page.tracks.len(),
            });

            let now = self.now();
            let records: Vec<ScrobbleRecord> = source_page
                .tracks
                .iter()
                .filter_map(|track| {
                    ScrobbleRecord::from_track(track, self.source.record_source(), now)
                })
                .collect();
            let oldest = records.iter().map(|r| r.uts).min();
            let newest = records.iter().map(|r| r.uts).max();

            if !records.is_empty() {
                let append = self.store.append_scrobbles(&records).await?;
                stats.scrobbles_new += append.new;
                stats.scrobbles_updated += append.updated;
                stats.scrobbles_unchanged += append.unchanged;
                self.events.emit(SyncEvent::ScrobblesDiscovered {
                    new: append.new,
                    updated: append.updated,
                    oldest,
                    newest,
                });
            }

            let past_boundary = oldest.is_some_and(|o| o < window.start);
            if !source_page.has_next || past_boundary {
                // Fetched past the window: the whole thing is confirmed.
                let segment = Segment::new(window.start, window.end, now);
                stats.seconds_covered += frontier.saturating_sub(window.start);
                self.apply_coverage(&mut coverage, segment).await?;
                return Ok(true);
            }

            if let Some(oldest) = oldest {
                let confirmed_from = oldest.saturating_add(1).max(window.start);
                if confirmed_from < frontier {
                    // Claim only strictly above the oldest scrobble seen: same-second ties
                    // may straddle the page break, and a resume re-fetches that second.
                    let segment = Segment::new(confirmed_from, frontier, now);
                    stats.seconds_covered += frontier - confirmed_from;
                    self.apply_coverage(&mut coverage, segment).await?;
                    frontier = confirmed_from;
                }
            }

            page += 1;
            if self.pages_exhausted(stats) {
                // Budget spent; everything confirmed so far is already persisted.
                return Ok(false);
            }
        }
    }

    async fn apply_coverage(&self, coverage: &mut CoverageMap, segment: Segment) -> Result<()> {
        let changes = coverage.insert(segment);
        if changes.is_empty() {
            return Ok(());
        }
        self.store.save_coverage(coverage).await?;
        for change in changes {
            self.events.emit(SyncEvent::CoverageChanged(change));
        }
        Ok(())
    }

    /// Fetch one page, waiting out rate limits (both proactively, via the source's state
    /// watch, and reactively, when the fetch itself reports `RateLimit`). Emits
    /// `SyncPaused`/`SyncResumed` around every wait so UIs can show why nothing is moving.
    async fn fetch_page_patiently(
        &self,
        window: &Range<u64>,
        page: u32,
    ) -> Result<crate::source::SourcePage> {
        loop {
            self.check_cancelled()?;
            self.await_rate_limit_clearance().await?;
            match self
                .source
                .fetch_window(Some(window.start), Some(window.end), page)
                .await
            {
                Ok(page) => return Ok(page),
                Err(StoreError::LastFm(lastfm_edit::LastFmError::RateLimit { retry_after })) => {
                    // Non-blocking client mode surfaces the limit as an error; park here.
                    let until = self.now() + retry_after;
                    self.events.emit(SyncEvent::SyncPaused {
                        reason: PauseReason::RateLimited {
                            until_estimate: Some(until),
                        },
                    });
                    self.sleep_or_cancelled(retry_after.min(60)).await?;
                    self.events.emit(SyncEvent::SyncResumed);
                }
                Err(err) => return Err(err),
            }
        }
    }

    async fn await_rate_limit_clearance(&self) -> Result<()> {
        let mut watcher = self.source.rate_limit();
        let mut paused = false;
        loop {
            let state = watcher.borrow_and_update().clone();
            let now = self.now();
            if !state.is_rate_limited_at(now) {
                if paused {
                    self.events.emit(SyncEvent::SyncResumed);
                }
                return Ok(());
            }
            if let RateLimitState::RateLimited { until_estimate, .. } = &state {
                if !paused {
                    paused = true;
                    self.events.emit(SyncEvent::SyncPaused {
                        reason: PauseReason::RateLimited {
                            until_estimate: Some(*until_estimate),
                        },
                    });
                }
                let wait = state
                    .remaining_at(now)
                    .map(|d| d.as_secs().clamp(1, 30))
                    .unwrap_or(1);
                // Wake on either a state change or the estimate elapsing (the state can go
                // stale if no further requests happen to refresh it).
                tokio::select! {
                    _ = watcher.changed() => {}
                    _ = tokio::time::sleep(std::time::Duration::from_secs(wait)) => {}
                }
                self.check_cancelled()?;
            }
        }
    }

    async fn sleep_or_cancelled(&self, secs: u64) -> Result<()> {
        let deadline = std::time::Duration::from_secs(secs);
        let step = std::time::Duration::from_millis(200);
        let mut waited = std::time::Duration::ZERO;
        while waited < deadline {
            self.check_cancelled()?;
            let chunk = step.min(deadline - waited);
            tokio::time::sleep(chunk).await;
            waited += chunk;
        }
        self.check_cancelled()
    }

    /// After a pass whose window was bounded below by our absolute floor, record where
    /// history actually starts so later gap computations stop probing below it.
    async fn note_history_start_if_exhausted(&self, window_start: u64) -> Result<()> {
        if window_start != 0 {
            return Ok(());
        }
        let mut sync_state = self.store.load_sync_state().await?;
        if sync_state.history_start_uts.is_some() {
            return Ok(());
        }
        if let Some(first) = self.store.scrobbles_in_range(0..u64::MAX).await?.first() {
            sync_state.history_start_uts = Some(first.uts);
            self.store.save_sync_state(&sync_state).await?;
        }
        Ok(())
    }
}
