//! Executor behavior with a mocked lastfm-edit client: full drain + local mirroring,
//! execution-time re-expansion, budget-bounded resume, rate-limit pause/retry without
//! consuming attempts, deferral under sustained rate limiting, mid-pass cancellation,
//! failure exhaustion → abandonment, and vanished instances.

use lastfm_edit::{
    EditResponse, ExactScrobbleEdit, MockLastFmEditClient, ScrobbleEdit, SingleEditResponse,
};
use scrobble_scrubber::queue::{QueueEvent, QueueEventKind};
use scrobble_scrubber::{
    ExecEnded, Executor, ExecutorOptions, IntentState, MemoryScrubberState, ScrubberEvent,
    ScrubberState, Subject,
};
use scrobble_store::{
    MemoryStorage, MirroredEditor, Provenanced, RecordSource, ScrobbleId, ScrobbleRecord, Storage,
};
use std::sync::Arc;
use uuid::Uuid;

const ARTIST: &str = "Queen";
const DIRTY: &str = "You And I - Remastered 2011";
const CLEAN: &str = "You And I";
const ALBUM: &str = "A Day at the Races";
const AA: &str = "Queen";

fn record(uts: u64) -> ScrobbleRecord {
    ScrobbleRecord {
        id: ScrobbleId::new(uts, ARTIST, DIRTY),
        uts,
        artist: ARTIST.to_string(),
        track: DIRTY.to_string(),
        album: Some(ALBUM.to_string()),
        // Verified so the executor's enrichment step needs no scraping mock.
        album_artist: Provenanced::Verified(AA.to_string()),
        source: RecordSource::Scrape,
        fetched_at: 1,
        deleted: false,
        v: 1,
    }
}

fn subject() -> Subject {
    Subject {
        artist: ARTIST.into(),
        track: DIRTY.into(),
        album: Some(ALBUM.into()),
        album_artist: Some(AA.into()),
    }
}

fn proposal() -> ScrobbleEdit {
    ScrobbleEdit {
        track_name_original: Some(DIRTY.into()),
        album_name_original: Some(ALBUM.into()),
        artist_name_original: ARTIST.into(),
        album_artist_name_original: Some(AA.into()),
        track_name: Some(CLEAN.into()),
        album_name: Some(ALBUM.into()),
        artist_name: ARTIST.into(),
        album_artist_name: Some(AA.into()),
        timestamp: None,
        edit_all: true, // notional; the executor only sends exact edits
    }
}

async fn queue_ready_intent(state: &dyn ScrubberState) -> Uuid {
    let id = Uuid::new_v4();
    state
        .append_queue_events(&[QueueEvent {
            id,
            at: 1,
            kind: QueueEventKind::Created {
                subject: subject(),
                proposed: Box::new(proposal()),
                provider: "rewrite_rules".into(),
                requires_approval: false,
            },
        }])
        .await
        .unwrap();
    id
}

fn success(edit: &ExactScrobbleEdit) -> EditResponse {
    EditResponse::from_results(vec![SingleEditResponse {
        success: true,
        message: None,
        album_info: None,
        exact_scrobble_edit: edit.clone(),
    }])
}

fn failure(edit: &ExactScrobbleEdit) -> EditResponse {
    EditResponse::from_results(vec![SingleEditResponse {
        success: false,
        message: Some("rejected".into()),
        album_info: None,
        exact_scrobble_edit: edit.clone(),
    }])
}

fn executor(
    store: Arc<MemoryStorage>,
    state: Arc<MemoryScrubberState>,
    client: MockLastFmEditClient,
    options: ExecutorOptions,
) -> Executor<MockLastFmEditClient> {
    // The watch client only provides rate-limit state; the mock's default watcher reads
    // Ready forever, which is what these tests want.
    let editor = MirroredEditor::new(store.clone(), client);
    Executor::from_parts(store, state, editor, MockLastFmEditClient::new()).with_options(options)
}

fn zero_delay() -> ExecutorOptions {
    ExecutorOptions {
        inter_edit_delay: std::time::Duration::ZERO,
        ..ExecutorOptions::default()
    }
}

#[tokio::test]
async fn applies_all_instances_mirrors_locally_and_completes() {
    let store = Arc::new(MemoryStorage::new());
    store
        .append_scrobbles(&[record(100), record(200), record(300)])
        .await
        .unwrap();
    let state = Arc::new(MemoryScrubberState::new());
    let intent_id = queue_ready_intent(state.as_ref()).await;

    let mut client = MockLastFmEditClient::new();
    client
        .expect_edit_scrobble_single()
        .times(3)
        .returning(|edit, _| {
            assert_eq!(edit.track_name, CLEAN);
            assert!(!edit.edit_all, "only exact edits may reach last.fm");
            Ok(success(edit))
        });

    let exec = executor(store.clone(), state.clone(), client, zero_delay());
    let mut rx = exec.subscribe();
    let report = exec.run_once().await.unwrap();

    assert_eq!(report.intents_processed, 1);
    assert_eq!(report.intents_completed, 1);
    assert_eq!(report.instances_applied, 3);
    assert_eq!(report.instances_failed, 0);
    assert_eq!(report.ended, ExecEnded::Completed);

    // Queue folded to Applied with three per-instance records.
    let queue = state.load_queue().await.unwrap();
    assert_eq!(queue[0].id, intent_id);
    assert_eq!(queue[0].state, IntentState::Applied);
    assert_eq!(queue[0].done_count(), 3);

    // Store mirrored: dirty identity gone, clean identity live.
    let live = store.scrobbles_in_range(0..u64::MAX).await.unwrap();
    assert_eq!(live.len(), 3);
    assert!(live.iter().all(|r| r.track == CLEAN));

    // Events: expansion, three applies, completion.
    let mut applied = 0;
    let mut completed = 0;
    while let Ok(event) = rx.try_recv() {
        match event {
            ScrubberEvent::EditApplied { .. } => applied += 1,
            ScrubberEvent::IntentCompleted { state, .. } => {
                assert_eq!(state, IntentState::Applied);
                completed += 1;
            }
            _ => {}
        }
    }
    assert_eq!(applied, 3);
    assert_eq!(completed, 1);
}

#[tokio::test]
async fn execution_time_expansion_includes_instances_added_after_planning() {
    let store = Arc::new(MemoryStorage::new());
    store.append_scrobbles(&[record(100)]).await.unwrap();
    let state = Arc::new(MemoryScrubberState::new());
    queue_ready_intent(state.as_ref()).await;

    // A new instance of the same subject arrives after the intent was planned.
    store.append_scrobbles(&[record(500)]).await.unwrap();

    let mut client = MockLastFmEditClient::new();
    client
        .expect_edit_scrobble_single()
        .times(2)
        .returning(|edit, _| Ok(success(edit)));

    let exec = executor(store.clone(), state.clone(), client, zero_delay());
    let report = exec.run_once().await.unwrap();
    assert_eq!(report.instances_applied, 2);
    assert_eq!(state.load_queue().await.unwrap()[0].done_count(), 2);
}

#[tokio::test]
async fn budget_stops_mid_intent_and_a_fresh_run_resumes_without_duplicates() {
    let store = Arc::new(MemoryStorage::new());
    store
        .append_scrobbles(&[record(100), record(200)])
        .await
        .unwrap();
    let state = Arc::new(MemoryScrubberState::new());
    queue_ready_intent(state.as_ref()).await;

    let mut client = MockLastFmEditClient::new();
    client
        .expect_edit_scrobble_single()
        .times(1) // budget allows exactly one attempt
        .returning(|edit, _| Ok(success(edit)));
    let exec = executor(
        store.clone(),
        state.clone(),
        client,
        ExecutorOptions {
            max_edits: Some(1),
            ..zero_delay()
        },
    );
    let report = exec.run_once().await.unwrap();
    assert_eq!(report.instances_applied, 1);
    assert_eq!(report.intents_completed, 0);
    assert_eq!(report.ended, ExecEnded::BudgetExhausted);
    assert_eq!(
        state.load_queue().await.unwrap()[0].state,
        IntentState::InProgress
    );

    // Fresh executor (crash simulation): only the remaining instance is edited.
    let mut client = MockLastFmEditClient::new();
    client
        .expect_edit_scrobble_single()
        .times(1)
        .returning(|edit, _| Ok(success(edit)));
    let exec = executor(store.clone(), state.clone(), client, zero_delay());
    let report = exec.run_once().await.unwrap();
    assert_eq!(report.instances_applied, 1);
    assert_eq!(report.intents_completed, 1);
    assert_eq!(report.ended, ExecEnded::Completed);
    assert_eq!(
        state.load_queue().await.unwrap()[0].state,
        IntentState::Applied
    );
    assert_eq!(state.load_queue().await.unwrap()[0].done_count(), 2);
}

#[tokio::test(start_paused = true)]
async fn rate_limit_pauses_retries_and_consumes_no_attempt() {
    let store = Arc::new(MemoryStorage::new());
    store.append_scrobbles(&[record(100)]).await.unwrap();
    let state = Arc::new(MemoryScrubberState::new());
    queue_ready_intent(state.as_ref()).await;

    let mut client = MockLastFmEditClient::new();
    let mut calls = 0;
    client
        .expect_edit_scrobble_single()
        .times(2)
        .returning(move |edit, _| {
            calls += 1;
            if calls == 1 {
                Err(lastfm_edit::LastFmError::RateLimit { retry_after: 5 })
            } else {
                Ok(success(edit))
            }
        });

    let exec = executor(store.clone(), state.clone(), client, zero_delay());
    let mut rx = exec.subscribe();
    let report = exec.run_once().await.unwrap();

    assert_eq!(report.instances_applied, 1);
    assert_eq!(report.instances_failed, 0, "rate limits are not failures");
    let intent = &state.load_queue().await.unwrap()[0];
    assert_eq!(intent.state, IntentState::Applied);
    assert_eq!(
        intent.failed_count(),
        0,
        "no attempt consumed by the rate limit"
    );

    let mut paused = false;
    let mut resumed = false;
    while let Ok(event) = rx.try_recv() {
        match event {
            ScrubberEvent::ExecutorPaused { .. } => paused = true,
            ScrubberEvent::ExecutorResumed => resumed = true,
            _ => {}
        }
    }
    assert!(paused && resumed);
}

#[tokio::test(start_paused = true)]
async fn sustained_rate_limiting_defers_the_pass_leaving_intents_in_progress() {
    let store = Arc::new(MemoryStorage::new());
    store.append_scrobbles(&[record(100)]).await.unwrap();
    let state = Arc::new(MemoryScrubberState::new());
    let first = queue_ready_intent(state.as_ref()).await;
    // A second intent queued behind the rate-limited one must not be touched.
    let second = queue_ready_intent(state.as_ref()).await;

    // last.fm rate-limits every attempt: two pauses fit the budget, the third defers.
    let mut client = MockLastFmEditClient::new();
    client
        .expect_edit_scrobble_single()
        .times(3)
        .returning(|_, _| Err(lastfm_edit::LastFmError::RateLimit { retry_after: 5 }));

    let exec = executor(
        store.clone(),
        state.clone(),
        client,
        ExecutorOptions {
            max_rate_limit_pauses_per_pass: 2,
            ..zero_delay()
        },
    );
    let mut rx = exec.subscribe();
    let report = exec.run_once().await.unwrap();

    assert_eq!(report.intents_processed, 1, "the pass stopped at the defer");
    assert_eq!(report.instances_applied, 0);
    assert_eq!(report.instances_failed, 0, "rate limits are not failures");
    assert_eq!(report.intents_abandoned, 0);
    assert_eq!(report.ended, ExecEnded::Deferred);

    // The deferred intent stays InProgress with no attempts consumed; the intent
    // behind it was never reached and is still Ready.
    let queue = state.load_queue().await.unwrap();
    let deferred = queue.iter().find(|i| i.id == first).unwrap();
    assert_eq!(deferred.state, IntentState::InProgress);
    assert_eq!(deferred.failed_count(), 0);
    let untouched = queue.iter().find(|i| i.id == second).unwrap();
    assert_eq!(untouched.state, IntentState::Ready);

    // The pass surfaced the rate limit and still completed (no Error event).
    let mut paused = false;
    let mut completed = false;
    while let Ok(event) = rx.try_recv() {
        match event {
            ScrubberEvent::ExecutorPaused { .. } => paused = true,
            ScrubberEvent::ExecCompleted { .. } => completed = true,
            ScrubberEvent::EditFailed { .. } | ScrubberEvent::Error { .. } => {
                panic!("deferral must not report failures")
            }
            _ => {}
        }
    }
    assert!(paused && completed);
}

#[tokio::test(start_paused = true)]
async fn cancellation_stops_the_pass_cleanly_and_the_next_pass_resumes() {
    let store = Arc::new(MemoryStorage::new());
    store
        .append_scrobbles(&[record(100), record(200)])
        .await
        .unwrap();
    let state = Arc::new(MemoryScrubberState::new());
    queue_ready_intent(state.as_ref()).await;

    // Two calls total: one before the cancel, one from the resuming second pass.
    let mut client = MockLastFmEditClient::new();
    client
        .expect_edit_scrobble_single()
        .times(2)
        .returning(|edit, _| Ok(success(edit)));

    let exec = executor(
        store.clone(),
        state.clone(),
        client,
        ExecutorOptions {
            inter_edit_delay: std::time::Duration::from_secs(1),
            ..ExecutorOptions::default()
        },
    );

    // Flip the cancel handle while the pass sits in its inter-edit delay.
    let cancel = exec.cancel_handle();
    let canceller = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    });
    let report = exec.run_once().await.unwrap(); // clean return, not an error
    canceller.await.unwrap();

    assert_eq!(report.instances_applied, 1);
    assert_eq!(report.instances_failed, 0);
    assert_eq!(report.ended, ExecEnded::Cancelled);
    assert_eq!(
        state.load_queue().await.unwrap()[0].state,
        IntentState::InProgress
    );

    // The flag resets at the start of the next pass: the same executor finishes.
    let report = exec.run_once().await.unwrap();
    assert_eq!(report.instances_applied, 1);
    assert_eq!(report.intents_completed, 1);
    assert_eq!(report.ended, ExecEnded::Completed);
    let intent = &state.load_queue().await.unwrap()[0];
    assert_eq!(intent.state, IntentState::Applied);
    assert_eq!(intent.done_count(), 2);
}

#[tokio::test]
async fn exhausted_failures_abandon_the_intent() {
    let store = Arc::new(MemoryStorage::new());
    store.append_scrobbles(&[record(100)]).await.unwrap();
    let state = Arc::new(MemoryScrubberState::new());
    queue_ready_intent(state.as_ref()).await;

    let mut client = MockLastFmEditClient::new();
    client
        .expect_edit_scrobble_single()
        .times(1)
        .returning(|edit, _| Ok(failure(edit)));

    let exec = executor(
        store.clone(),
        state.clone(),
        client,
        ExecutorOptions {
            max_attempts_per_instance: 1,
            ..zero_delay()
        },
    );
    let report = exec.run_once().await.unwrap();
    assert_eq!(report.instances_failed, 1);
    assert_eq!(report.intents_abandoned, 1);
    assert!(matches!(
        state.load_queue().await.unwrap()[0].state,
        IntentState::Abandoned { .. }
    ));
}

// A second, distinct subject so the two intents in the ordering test don't collide.
const ARTIST_B: &str = "David Bowie";
const DIRTY_B: &str = "Heroes - 2017 Remaster";
const CLEAN_B: &str = "Heroes";
const ALBUM_B: &str = "\"Heroes\"";
const AA_B: &str = "David Bowie";

fn record_for(artist: &str, track: &str, album: &str, aa: &str, uts: u64) -> ScrobbleRecord {
    ScrobbleRecord {
        id: ScrobbleId::new(uts, artist, track),
        uts,
        artist: artist.to_string(),
        track: track.to_string(),
        album: Some(album.to_string()),
        album_artist: Provenanced::Verified(aa.to_string()),
        source: RecordSource::Scrape,
        fetched_at: 1,
        deleted: false,
        v: 1,
    }
}

fn subject_for(artist: &str, track: &str, album: &str, aa: &str) -> Subject {
    Subject {
        artist: artist.into(),
        track: track.into(),
        album: Some(album.into()),
        album_artist: Some(aa.into()),
    }
}

fn proposal_for(artist: &str, dirty: &str, clean: &str, album: &str, aa: &str) -> ScrobbleEdit {
    ScrobbleEdit {
        track_name_original: Some(dirty.into()),
        album_name_original: Some(album.into()),
        artist_name_original: artist.into(),
        album_artist_name_original: Some(aa.into()),
        track_name: Some(clean.into()),
        album_name: Some(album.into()),
        artist_name: artist.into(),
        album_artist_name: Some(aa.into()),
        timestamp: None,
        edit_all: true,
    }
}

/// Queue an intent for `subject` at logical time `at`. If `expand` is set, append an
/// `Expanded` event (with the given store instances) so the intent folds to `InProgress`;
/// otherwise it folds to `Ready`.
async fn queue_intent_for(
    state: &dyn ScrubberState,
    at: u64,
    subject: Subject,
    proposed: ScrobbleEdit,
    expand: Option<Vec<ScrobbleId>>,
) -> Uuid {
    let id = Uuid::new_v4();
    state
        .append_queue_events(&[QueueEvent {
            id,
            at,
            kind: QueueEventKind::Created {
                subject,
                proposed: Box::new(proposed),
                provider: "rewrite_rules".into(),
                requires_approval: false,
            },
        }])
        .await
        .unwrap();
    if let Some(instance_ids) = expand {
        state
            .append_queue_events(&[QueueEvent {
                id,
                at: at + 1,
                kind: QueueEventKind::Expanded { instance_ids },
            }])
            .await
            .unwrap();
    }
    id
}

#[tokio::test]
async fn resumes_in_progress_before_ready() {
    use std::sync::Mutex;

    let store = Arc::new(MemoryStorage::new());
    // Intent R's subject (the original Queen fixtures) and intent P's distinct subject.
    let rec_r = record(100);
    let rec_p = record_for(ARTIST_B, DIRTY_B, ALBUM_B, AA_B, 200);
    store
        .append_scrobbles(&[rec_r.clone(), rec_p.clone()])
        .await
        .unwrap();

    let state = Arc::new(MemoryScrubberState::new());
    // R created FIRST, left Ready (no Expanded event).
    let r = queue_intent_for(state.as_ref(), 1, subject(), proposal(), None).await;
    // P created SECOND (later) with an Expanded event → folds to InProgress.
    let p = queue_intent_for(
        state.as_ref(),
        10,
        subject_for(ARTIST_B, DIRTY_B, ALBUM_B, AA_B),
        proposal_for(ARTIST_B, DIRTY_B, CLEAN_B, ALBUM_B, AA_B),
        Some(vec![rec_p.id.clone()]),
    )
    .await;

    // Sanity: the folded states are what the ordering test hinges on.
    {
        let queue = state.load_queue().await.unwrap();
        assert_eq!(
            queue.iter().find(|i| i.id == r).unwrap().state,
            IntentState::Ready
        );
        assert_eq!(
            queue.iter().find(|i| i.id == p).unwrap().state,
            IntentState::InProgress
        );
    }

    // The mock records the artist of each edit in call order.
    let order = Arc::new(Mutex::new(Vec::<String>::new()));
    let recorder = order.clone();
    let mut client = MockLastFmEditClient::new();
    client
        .expect_edit_scrobble_single()
        .times(2)
        .returning(move |edit, _| {
            recorder.lock().unwrap().push(edit.artist_name.clone());
            Ok(success(edit))
        });

    let exec = executor(store.clone(), state.clone(), client, zero_delay());
    let report = exec.run_once().await.unwrap();
    assert_eq!(report.instances_applied, 2);

    // Priority override of creation order: the later-created InProgress intent P is
    // edited BEFORE the earlier-created Ready intent R.
    let seen = order.lock().unwrap().clone();
    assert_eq!(
        seen,
        vec![ARTIST_B.to_string(), ARTIST.to_string()],
        "InProgress intent P must be resumed before Ready intent R"
    );
}

#[tokio::test]
async fn vanished_subject_completes_without_touching_lastfm() {
    let store = Arc::new(MemoryStorage::new());
    let rec = record(100);
    store
        .append_scrobbles(std::slice::from_ref(&rec))
        .await
        .unwrap();
    let state = Arc::new(MemoryScrubberState::new());
    queue_ready_intent(state.as_ref()).await;

    // The only instance is deleted before execution.
    store
        .append_scrobbles(&[rec.into_tombstone(50)])
        .await
        .unwrap();

    // No expectations set: any client call would panic the mock.
    let client = MockLastFmEditClient::new();
    let exec = executor(store.clone(), state.clone(), client, zero_delay());
    let report = exec.run_once().await.unwrap();
    assert_eq!(report.intents_completed, 1);
    assert_eq!(report.instances_applied, 0);
    assert_eq!(
        state.load_queue().await.unwrap()[0].state,
        IntentState::Applied
    );
}
