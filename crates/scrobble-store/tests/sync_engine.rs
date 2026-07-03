//! Sync engine behavior against a scripted in-memory source: determinism under mid-pass
//! injection, same-second ties at page boundaries, cancellation + resume, rate-limit
//! pause/resume, verification, and page budgets.

use lastfm_edit::{RateLimitState, RateLimitStateWatcher, RateLimitType, Track};
use scrobble_store::source::{ScrobbleSource, SourcePage};
use scrobble_store::{
    MemoryStorage, RecordSource, Result, Storage, SyncEngine, SyncEvent, SyncOptions,
};
use std::sync::{Arc, Mutex};
use tokio::sync::watch;

const NOW: u64 = 1_000_000;
const MARGIN: u64 = 60;
const PIN: u64 = NOW - MARGIN;

fn track(uts: u64, artist: &str, name: &str) -> Track {
    Track {
        name: name.to_string(),
        artist: artist.to_string(),
        playcount: 1,
        timestamp: Some(uts),
        album: Some("Album".to_string()),
        album_artist: None,
    }
}

type FetchHook = Box<dyn FnMut(u32) + Send>;

/// Scripted [`ScrobbleSource`]: a mutable timeline, page-size chunking, a controllable
/// rate-limit watch, per-fetch hooks, and optional scripted failures.
struct FakeSource {
    timeline: Arc<Mutex<Vec<Track>>>,
    page_size: usize,
    rate_tx: Arc<watch::Sender<RateLimitState>>,
    on_fetch: Mutex<Option<FetchHook>>,
    fail_next: Arc<Mutex<Option<lastfm_edit::LastFmError>>>,
    fetch_count: Arc<Mutex<u32>>,
}

impl FakeSource {
    fn new(tracks: Vec<Track>, page_size: usize) -> Arc<Self> {
        let (rate_tx, _) = watch::channel(RateLimitState::Ready);
        Arc::new(Self {
            timeline: Arc::new(Mutex::new(tracks)),
            page_size,
            rate_tx: Arc::new(rate_tx),
            on_fetch: Mutex::new(None),
            fail_next: Arc::new(Mutex::new(None)),
            fetch_count: Arc::new(Mutex::new(0)),
        })
    }

    fn set_hook(&self, hook: impl FnMut(u32) + Send + 'static) {
        *self.on_fetch.lock().unwrap() = Some(Box::new(hook));
    }
}

#[async_trait::async_trait(?Send)]
impl ScrobbleSource for FakeSource {
    fn record_source(&self) -> RecordSource {
        RecordSource::Api
    }

    async fn fetch_window(
        &self,
        _from: Option<u64>,
        to: Option<u64>,
        page: u32,
    ) -> Result<SourcePage> {
        *self.fetch_count.lock().unwrap() += 1;
        if let Some(hook) = self.on_fetch.lock().unwrap().as_mut() {
            hook(page);
        }
        if let Some(err) = self.fail_next.lock().unwrap().take() {
            return Err(err.into());
        }
        // Snapshot after the hook so injected scrobbles take effect on this fetch.
        let mut visible: Vec<Track> = self
            .timeline
            .lock()
            .unwrap()
            .iter()
            .filter(|t| t.timestamp.is_some_and(|ts| to.is_none_or(|pin| ts < pin)))
            .cloned()
            .collect();
        // Newest first, deterministic tiebreak, like the real feed.
        visible.sort_by(|a, b| {
            b.timestamp
                .cmp(&a.timestamp)
                .then_with(|| b.name.cmp(&a.name))
        });
        let start = (page as usize - 1) * self.page_size;
        let tracks: Vec<Track> = visible
            .iter()
            .skip(start)
            .take(self.page_size)
            .cloned()
            .collect();
        let has_next = start + self.page_size < visible.len();
        Ok(SourcePage { tracks, has_next })
    }

    fn rate_limit(&self) -> RateLimitStateWatcher {
        self.rate_tx.subscribe()
    }
}

fn engine(store: Arc<MemoryStorage>, source: Arc<FakeSource>) -> SyncEngine {
    SyncEngine::with_options(
        store,
        source,
        SyncOptions {
            safety_margin_secs: MARGIN,
            max_pages: None,
        },
    )
    .with_clock(|| NOW)
}

fn drain(rx: &mut scrobble_store::SyncEventReceiver) -> Vec<SyncEvent> {
    let mut events = Vec::new();
    while let Ok(event) = rx.try_recv() {
        events.push(event);
    }
    events
}

#[tokio::test]
async fn initial_sync_covers_everything_and_is_idempotent() {
    let tracks: Vec<Track> = (0..10)
        .map(|i| track(1000 + i * 100, "A", &format!("t{i}")))
        .collect();
    let source = FakeSource::new(tracks, 3);
    let store = Arc::new(MemoryStorage::new());
    let engine = engine(store.clone(), source.clone());
    let mut rx = engine.subscribe();

    let stats = engine.extend_to_present().await.unwrap();
    assert_eq!(stats.scrobbles_new, 10);
    assert_eq!(stats.pages_fetched, 4);

    // Coverage is one segment spanning [0, PIN).
    let coverage = store.load_coverage().await.unwrap();
    assert_eq!(coverage.segments().len(), 1);
    assert_eq!(coverage.segments()[0].range(), 0..PIN);

    // History start recorded.
    let sync_state = store.load_sync_state().await.unwrap();
    assert_eq!(sync_state.history_start_uts, Some(1000));

    let events = drain(&mut rx);
    assert!(matches!(
        events.first(),
        Some(SyncEvent::SyncStarted { .. })
    ));
    assert!(matches!(
        events.last(),
        Some(SyncEvent::SyncCompleted { .. })
    ));
    assert!(events
        .iter()
        .any(|e| matches!(e, SyncEvent::CoverageChanged(_))));

    // Second run: already current, no fetches.
    let before = *source.fetch_count.lock().unwrap();
    let stats = engine.extend_to_present().await.unwrap();
    assert_eq!(stats.pages_fetched, 0);
    assert_eq!(*source.fetch_count.lock().unwrap(), before);
}

#[tokio::test]
async fn scrobbles_injected_mid_pass_do_not_disturb_the_pass() {
    let tracks: Vec<Track> = (0..9)
        .map(|i| track(1000 + i * 100, "A", &format!("t{i}")))
        .collect();
    let source = FakeSource::new(tracks, 3);
    let store = Arc::new(MemoryStorage::new());

    // While page 2 is being fetched, a brand-new scrobble lands "now" (above the pin) and
    // another lands mid-timeline BELOW the pin (simulating an offline device flushing).
    {
        let timeline = source.timeline.clone();
        source.set_hook(move |page| {
            if page == 2 {
                let mut tl = timeline.lock().unwrap();
                tl.push(track(NOW, "A", "brand-new"));
                tl.push(track(1450, "A", "late-arrival"));
            }
        });
    }

    let engine = engine(store.clone(), source.clone());
    engine.extend_to_present().await.unwrap();

    let all = store.scrobbles_in_range(0..u64::MAX).await.unwrap();
    // The above-pin scrobble is excluded by the pin; the mid-timeline one appears on a
    // later page of THIS pass (timestamps below the already-confirmed frontier are only
    // guaranteed by the visited-at-least-once contract) — with our fake feed it lands on
    // page 2's re-sorted view, so it must be present.
    assert!(all.iter().any(|r| r.track == "late-arrival"));
    assert!(!all.iter().any(|r| r.track == "brand-new"));
    assert_eq!(all.len(), 10);

    // The next extend picks up the above-pin scrobble once the clock moves past it.
    let engine2 = SyncEngine::with_options(
        store.clone(),
        source.clone(),
        SyncOptions {
            safety_margin_secs: MARGIN,
            max_pages: None,
        },
    )
    .with_clock(|| NOW + 120);
    engine2.extend_to_present().await.unwrap();
    let all = store.scrobbles_in_range(0..u64::MAX).await.unwrap();
    assert!(all.iter().any(|r| r.track == "brand-new"));
}

#[tokio::test]
async fn cancellation_persists_partial_coverage_and_resume_completes() {
    // Three same-second scrobbles straddle the page-2/3 boundary (page size 2): page 2
    // ends with tie-c; tie-b and tie-a live on page 3, which never arrives because the
    // pass dies mid-flight (simulated crash).
    let mut tracks: Vec<Track> = vec![
        track(5000, "A", "newest"),
        track(4000, "A", "older"),
        track(3000, "A", "old"),
        track(2000, "A", "tie-a"),
        track(2000, "A", "tie-b"),
        track(2000, "A", "tie-c"),
        track(1000, "A", "oldest"),
        track(500, "A", "ancient"),
    ];
    tracks.reverse(); // order in the vec is irrelevant; the source sorts
    let source = FakeSource::new(tracks, 2);
    let store = Arc::new(MemoryStorage::new());
    let crashing_engine = engine(store.clone(), source.clone());
    {
        let fail = source.fail_next.clone();
        source.set_hook(move |page| {
            if page == 3 {
                *fail.lock().unwrap() =
                    Some(lastfm_edit::LastFmError::Http("simulated crash".into()));
            }
        });
    }
    assert!(crashing_engine.extend_to_present().await.is_err());

    // Partial coverage persisted, starting strictly above the straddling second — the
    // 2000 second is NOT claimed even though tie-c was already stored.
    let coverage = store.load_coverage().await.unwrap();
    assert_eq!(coverage.segments().len(), 1);
    assert_eq!(coverage.segments()[0].range(), 2001..PIN);
    let stored = store.scrobbles_in_range(0..u64::MAX).await.unwrap();
    assert_eq!(stored.iter().filter(|r| r.uts == 2000).count(), 1);

    // Fresh engine, resume via backfill: the residual gap [0, 2001) re-fetches the 2000
    // second, catching the ties the crashed pass never saw.
    source.set_hook(|_| {});
    let engine2 = engine(store.clone(), source.clone());
    engine2.backfill(None).await.unwrap();

    let all = store.scrobbles_in_range(0..u64::MAX).await.unwrap();
    assert_eq!(all.len(), 8);
    assert_eq!(
        all.iter().filter(|r| r.uts == 2000).count(),
        3,
        "all three same-second ties present after resume"
    );
    let coverage = store.load_coverage().await.unwrap();
    assert_eq!(coverage.segments().len(), 1);
    assert_eq!(coverage.segments()[0].range(), 0..PIN);
}

#[tokio::test(start_paused = true)]
async fn proactive_rate_limit_pause_and_resume() {
    let source = FakeSource::new(vec![track(1000, "A", "x")], 10);
    let store = Arc::new(MemoryStorage::new());
    let engine = engine(store.clone(), source.clone());
    let mut rx = engine.subscribe();

    // Source starts parked until well past the engine's (fixed) clock.
    source.rate_tx.send_replace(RateLimitState::RateLimited {
        since: NOW - 10,
        until_estimate: NOW + 300,
        kind: RateLimitType::Http429,
    });

    // Run the engine and a controller concurrently on this thread (futures are !Send).
    let run = engine.extend_to_present();
    let controller = async {
        // Give the engine a moment to observe the parked state and emit SyncPaused.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        source.rate_tx.send_replace(RateLimitState::Ready);
    };
    let (result, ()) = tokio::join!(run, controller);
    result.unwrap();

    let events = drain(&mut rx);
    let paused = events
        .iter()
        .position(|e| matches!(e, SyncEvent::SyncPaused { .. }))
        .expect("SyncPaused emitted");
    let resumed = events
        .iter()
        .position(|e| matches!(e, SyncEvent::SyncResumed))
        .expect("SyncResumed emitted");
    assert!(paused < resumed);
    assert_eq!(
        store.scrobbles_in_range(0..u64::MAX).await.unwrap().len(),
        1
    );
}

#[tokio::test(start_paused = true)]
async fn reactive_rate_limit_error_pauses_and_retries() {
    let source = FakeSource::new(vec![track(1000, "A", "x")], 10);
    *source.fail_next.lock().unwrap() =
        Some(lastfm_edit::LastFmError::RateLimit { retry_after: 5 });
    let store = Arc::new(MemoryStorage::new());
    let engine = engine(store.clone(), source.clone());
    let mut rx = engine.subscribe();

    engine.extend_to_present().await.unwrap();

    let events = drain(&mut rx);
    assert!(events.iter().any(|e| matches!(
        e,
        SyncEvent::SyncPaused {
            reason: scrobble_store::PauseReason::RateLimited { .. }
        }
    )));
    assert!(events.iter().any(|e| matches!(e, SyncEvent::SyncResumed)));
    assert_eq!(
        store.scrobbles_in_range(0..u64::MAX).await.unwrap().len(),
        1
    );
}

#[tokio::test]
async fn verify_reconciles_out_of_band_changes() {
    let source = FakeSource::new(
        vec![
            track(1000, "A", "kept"),
            track(2000, "A", "renamed-upstream"),
        ],
        10,
    );
    let store = Arc::new(MemoryStorage::new());
    let engine = engine(store.clone(), source.clone());

    // Local store believes something else: has a record deleted upstream, lacks the rename.
    let deleted_locally_present = scrobble_store::ScrobbleRecord::from_track(
        &track(1500, "A", "deleted-upstream"),
        RecordSource::Api,
        1,
    )
    .unwrap();
    let stale_name = scrobble_store::ScrobbleRecord::from_track(
        &track(2000, "A", "old-name"),
        RecordSource::Api,
        1,
    )
    .unwrap();
    store
        .append_scrobbles(&[deleted_locally_present.clone(), stale_name.clone()])
        .await
        .unwrap();

    let report = engine.verify(500..3000).await.unwrap();
    assert_eq!(report.upstream_count, 2);
    // Both upstream records are new ids locally ("renamed-upstream" has a different id
    // than "old-name"), and both stale local ids get tombstoned.
    assert_eq!(report.written, 2);
    assert_eq!(report.tombstoned, 2);

    let live = store.scrobbles_in_range(0..u64::MAX).await.unwrap();
    let names: Vec<&str> = live.iter().map(|r| r.track.as_str()).collect();
    assert_eq!(names, vec!["kept", "renamed-upstream"]);
    let coverage = store.load_coverage().await.unwrap();
    assert!(coverage.covers(500..3000));
}

#[tokio::test]
async fn page_budget_stops_early_and_resumes_cleanly() {
    let tracks: Vec<Track> = (0..20)
        .map(|i| track(1000 + i * 10, "A", &format!("t{i}")))
        .collect();
    let source = FakeSource::new(tracks, 4);
    let store = Arc::new(MemoryStorage::new());
    let limited = SyncEngine::with_options(
        store.clone(),
        source.clone(),
        SyncOptions {
            safety_margin_secs: MARGIN,
            max_pages: Some(2),
        },
    )
    .with_clock(|| NOW);

    let stats = limited.extend_to_present().await.unwrap();
    assert_eq!(stats.pages_fetched, 2);
    let coverage = store.load_coverage().await.unwrap();
    assert_eq!(coverage.segments().len(), 1);
    assert!(coverage.segments()[0].start > 0, "partial coverage only");

    // Unbudgeted resume finishes the job.
    let full = engine(store.clone(), source.clone());
    full.backfill(None).await.unwrap();
    let coverage = store.load_coverage().await.unwrap();
    assert_eq!(coverage.segments()[0].range(), 0..PIN);
    assert_eq!(
        store.scrobbles_in_range(0..u64::MAX).await.unwrap().len(),
        20
    );
}

#[tokio::test]
async fn invalidate_forces_refetch() {
    let source = FakeSource::new(vec![track(1000, "A", "x"), track(2000, "A", "y")], 10);
    let store = Arc::new(MemoryStorage::new());
    let engine = engine(store.clone(), source.clone());
    engine.extend_to_present().await.unwrap();
    let before = *source.fetch_count.lock().unwrap();

    engine.invalidate(1500..2500).await.unwrap();
    let coverage = store.load_coverage().await.unwrap();
    assert_eq!(
        coverage.segments().len(),
        2,
        "segment split by invalidation"
    );

    engine.fill_gaps(None).await.unwrap();
    assert!(*source.fetch_count.lock().unwrap() > before);
    let coverage = store.load_coverage().await.unwrap();
    assert_eq!(coverage.segments().len(), 1);
    assert_eq!(coverage.segments()[0].range(), 0..PIN);
}
