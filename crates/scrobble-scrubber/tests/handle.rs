//! The actor-style surface: pushed work via plan_records/Consider, command dispatch,
//! and the sync-event bridge — all without any feed pulling from the store.

use lastfm_edit::{EditResponse, MockLastFmEditClient, SingleEditResponse};
use scrobble_scrubber::{
    bridge_sync_events, Executor, ExecutorOptions, IntentState, MemoryScrubberState, Planner,
    RewriteRule, RewriteRulesScrubActionProvider, ScrubberActor, ScrubberCommand, ScrubberEvent,
    ScrubberState, SdRule,
};
use scrobble_store::{
    MemoryStorage, MirroredEditor, Provenanced, RecordSource, ScrobbleId, ScrobbleRecord, Storage,
    SyncEvent, SyncEventBus,
};
use std::sync::Arc;

fn record(uts: u64, track: &str) -> ScrobbleRecord {
    ScrobbleRecord {
        id: ScrobbleId::new(uts, "Queen", track),
        uts,
        artist: "Queen".to_string(),
        track: track.to_string(),
        album: Some("A Day at the Races".to_string()),
        album_artist: Provenanced::Verified("Queen".to_string()),
        source: RecordSource::Scrape,
        fetched_at: 1,
        deleted: false,
        v: 1,
    }
}

fn remaster_rule() -> RewriteRule {
    RewriteRule::new()
        .with_name("strip remaster")
        .with_track_name(SdRule::new(r"^(?P<base>.+) - Remastered \d+$", "${base}"))
}

fn planner(store: Arc<MemoryStorage>, state: Arc<MemoryScrubberState>) -> Planner {
    Planner::new(store, state).with_provider(RewriteRulesScrubActionProvider::from_rules(vec![
        remaster_rule(),
    ]))
}

#[tokio::test]
async fn plan_records_pushes_work_without_feeds_or_coverage() {
    let store = Arc::new(MemoryStorage::new());
    let state = Arc::new(MemoryScrubberState::new());
    // Note: the records are NOT in the store and there is NO sync coverage — pushed
    // work is planner input all by itself.
    let pushed = vec![
        record(100, "You And I - Remastered 2011"),
        record(200, "You And I - Remastered 2011"),
        record(300, "Clean Song"),
    ];

    let planner = planner(store, state.clone());
    let report = planner.plan_records(&pushed).await.unwrap();

    assert_eq!(report.subjects_seen, 2);
    assert_eq!(report.queued_ready, 1);
    let queue = state.load_queue().await.unwrap();
    assert_eq!(queue.len(), 1);
    assert_eq!(queue[0].proposed.track_name.as_deref(), Some("You And I"));

    // No coverage claims from pushed work.
    let coverage = state.load_provider_coverage("rewrite_rules").await.unwrap();
    assert!(coverage.coverage.is_empty());

    // Re-pushing the same records is deduplicated by the open intent.
    let report = planner.plan_records(&pushed).await.unwrap();
    assert_eq!(report.queued_ready, 0);
    assert_eq!(state.load_queue().await.unwrap().len(), 1);
}

fn success_response(edit: &lastfm_edit::ExactScrobbleEdit) -> EditResponse {
    EditResponse::from_results(vec![SingleEditResponse {
        success: true,
        message: None,
        album_info: None,
        exact_scrobble_edit: edit.clone(),
    }])
}

#[tokio::test]
async fn actor_processes_consider_and_execute_commands() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let store = Arc::new(MemoryStorage::new());
            let state = Arc::new(MemoryScrubberState::new());
            let dirty = record(100, "You And I - Remastered 2011");
            store.append_scrobbles(&[dirty.clone()]).await.unwrap();

            let mut client = MockLastFmEditClient::new();
            client
                .expect_edit_scrobble_single()
                .times(1)
                .returning(|edit, _| Ok(success_response(edit)));
            let editor = MirroredEditor::new(store.clone() as Arc<dyn Storage>, client);

            let planner = planner(store.clone(), state.clone());
            let bus = planner.event_bus();
            let executor = Executor::from_parts(
                store.clone() as Arc<dyn Storage>,
                state.clone() as Arc<dyn ScrubberState>,
                editor,
                MockLastFmEditClient::new(),
            )
            .with_options(ExecutorOptions {
                inter_edit_delay: std::time::Duration::ZERO,
                ..ExecutorOptions::default()
            })
            .with_event_bus(bus);

            let (handle, actor) =
                ScrubberActor::new(planner, executor, state.clone() as Arc<dyn ScrubberState>);
            let mut events = handle.subscribe();
            let actor_task = tokio::task::spawn_local(actor.run());

            // Push work, execute it, stop.
            handle
                .send(ScrubberCommand::Consider(vec![dirty]))
                .await
                .unwrap();
            handle
                .send(ScrubberCommand::ExecuteOnce { max_edits: None })
                .await
                .unwrap();
            handle.send(ScrubberCommand::Stop).await.unwrap();
            actor_task.await.unwrap();

            // Intent went Ready → Applied and the store mirrored the rename.
            let queue = state.load_queue().await.unwrap();
            assert_eq!(queue.len(), 1);
            assert_eq!(queue[0].state, IntentState::Applied);
            let live = store.scrobbles_in_range(0..u64::MAX).await.unwrap();
            assert_eq!(live[0].track, "You And I");

            // The one ordered event stream saw queueing, execution, and shutdown.
            let mut saw_queued = false;
            let mut saw_applied = false;
            let mut saw_stopped = false;
            while let Ok(event) = events.try_recv() {
                match event {
                    ScrubberEvent::IntentQueued { .. } => saw_queued = true,
                    ScrubberEvent::EditApplied { .. } => saw_applied = true,
                    ScrubberEvent::Stopped { .. } => saw_stopped = true,
                    _ => {}
                }
            }
            assert!(saw_queued && saw_applied && saw_stopped);
        })
        .await;
}

#[tokio::test]
async fn actor_survives_command_errors() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let store = Arc::new(MemoryStorage::new());
            let state = Arc::new(MemoryScrubberState::new());
            let planner = planner(store.clone(), state.clone());
            let bus = planner.event_bus();
            let executor = Executor::from_parts(
                store.clone() as Arc<dyn Storage>,
                state.clone() as Arc<dyn ScrubberState>,
                MirroredEditor::new(
                    store.clone() as Arc<dyn Storage>,
                    MockLastFmEditClient::new(),
                ),
                MockLastFmEditClient::new(),
            )
            .with_event_bus(bus);

            let (handle, actor) =
                ScrubberActor::new(planner, executor, state.clone() as Arc<dyn ScrubberState>);
            let mut events = handle.subscribe();
            let actor_task = tokio::task::spawn_local(actor.run());

            // Approving a nonexistent intent errors — the actor reports and keeps going.
            handle
                .send(ScrubberCommand::Approve(uuid::Uuid::new_v4()))
                .await
                .unwrap();
            handle
                .send(ScrubberCommand::Consider(vec![record(
                    100,
                    "You And I - Remastered 2011",
                )]))
                .await
                .unwrap();
            handle.send(ScrubberCommand::Stop).await.unwrap();
            actor_task.await.unwrap();

            let mut saw_error = false;
            while let Ok(event) = events.try_recv() {
                if matches!(event, ScrubberEvent::Error { .. }) {
                    saw_error = true;
                }
            }
            assert!(saw_error, "error was reported on the bus");
            assert_eq!(
                state.load_queue().await.unwrap().len(),
                1,
                "commands after the failing one still ran"
            );
        })
        .await;
}

#[tokio::test]
async fn sync_bridge_turns_discoveries_into_intents_and_forwards_events() {
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async {
            let store = Arc::new(MemoryStorage::new());
            let state = Arc::new(MemoryScrubberState::new());
            let dirty = record(100, "You And I - Remastered 2011");
            store.append_scrobbles(&[dirty.clone()]).await.unwrap();

            let planner = planner(store.clone(), state.clone());
            let bus = planner.event_bus();
            let executor = Executor::from_parts(
                store.clone() as Arc<dyn Storage>,
                state.clone() as Arc<dyn ScrubberState>,
                MirroredEditor::new(
                    store.clone() as Arc<dyn Storage>,
                    MockLastFmEditClient::new(),
                ),
                MockLastFmEditClient::new(),
            )
            .with_event_bus(bus);

            let (handle, actor) =
                ScrubberActor::new(planner, executor, state.clone() as Arc<dyn ScrubberState>);
            let mut events = handle.subscribe();
            let actor_task = tokio::task::spawn_local(actor.run());

            // A store-side sync bus, as SyncEngine would use.
            let sync_bus = SyncEventBus::new();
            let bridge = tokio::task::spawn_local(bridge_sync_events(
                sync_bus.subscribe(),
                store.clone() as Arc<dyn Storage>,
                handle.clone(),
            ));

            // The store "discovers" the scrobble.
            sync_bus.emit(SyncEvent::ScrobblesDiscovered {
                new: 1,
                updated: 0,
                oldest: Some(100),
                newest: Some(100),
            });

            // Give the bridge + actor a moment on this thread, then shut down.
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            drop(sync_bus); // closes the broadcast → bridge exits
            bridge.await.unwrap();
            handle.send(ScrubberCommand::Stop).await.unwrap();
            actor_task.await.unwrap();

            // The discovery became a queued intent without any feed involvement...
            let queue = state.load_queue().await.unwrap();
            assert_eq!(queue.len(), 1);
            assert_eq!(queue[0].subject.track, "You And I - Remastered 2011");

            // ...and the sync event was forwarded onto the scrubber bus.
            let mut saw_forwarded_sync = false;
            while let Ok(event) = events.try_recv() {
                if matches!(
                    event,
                    ScrubberEvent::Sync(SyncEvent::ScrobblesDiscovered { .. })
                ) {
                    saw_forwarded_sync = true;
                }
            }
            assert!(saw_forwarded_sync);
        })
        .await;
}
