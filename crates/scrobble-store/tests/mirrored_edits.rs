//! MirroredEditor behavior against a mocked lastfm-edit client: enrichment, the
//! queue→apply→mirror lifecycle, failure/retry, crash-recovery convergence, deletes, and
//! rate-limit propagation.

use lastfm_edit::{
    EditResponse, ExactScrobbleEdit, MockLastFmEditClient, RateLimitState, RateLimitStateWatcher,
    SingleEditResponse, Track,
};
use scrobble_store::source::{ScrobbleSource, SourcePage};
use scrobble_store::{
    EditOutcome, EditState, MemoryStorage, MirroredEditor, Provenanced, RecordSource, Result,
    ScrobbleId, ScrobbleRecord, Storage, StoreError,
};
use std::sync::Arc;

const UTS: u64 = 1_700_000_000;

fn seeded_record() -> ScrobbleRecord {
    ScrobbleRecord {
        id: ScrobbleId::new(UTS, "Cam'ron", "Oh Boy"),
        uts: UTS,
        artist: "Cam'ron".to_string(),
        track: "Oh Boy".to_string(),
        album: Some("Come Home With Me".to_string()),
        album_artist: Provenanced::Unknown, // API-sourced: needs enrichment before editing
        source: RecordSource::Api,
        fetched_at: 1,
        deleted: false,
        v: 1,
    }
}

fn variation(album_artist: &str) -> ExactScrobbleEdit {
    ExactScrobbleEdit {
        track_name_original: "Oh Boy".to_string(),
        album_name_original: "Come Home With Me".to_string(),
        artist_name_original: "Cam'ron".to_string(),
        album_artist_name_original: album_artist.to_string(),
        track_name: "Oh Boy".to_string(),
        album_name: "Come Home With Me".to_string(),
        artist_name: "Cam'ron".to_string(),
        album_artist_name: album_artist.to_string(),
        timestamp: UTS,
        edit_all: false,
    }
}

fn success_response(edit: &ExactScrobbleEdit) -> EditResponse {
    EditResponse::from_results(vec![SingleEditResponse {
        success: true,
        message: None,
        album_info: None,
        exact_scrobble_edit: edit.clone(),
    }])
}

fn failure_response(edit: &ExactScrobbleEdit, message: &str) -> EditResponse {
    EditResponse::from_results(vec![SingleEditResponse {
        success: false,
        message: Some(message.to_string()),
        album_info: None,
        exact_scrobble_edit: edit.clone(),
    }])
}

/// Minimal scripted source for `resume_pending`'s crash-window check.
struct StubSource {
    tracks: Vec<Track>,
}

#[async_trait::async_trait(?Send)]
impl ScrobbleSource for StubSource {
    fn record_source(&self) -> RecordSource {
        RecordSource::Api
    }

    async fn fetch_window(
        &self,
        _from: Option<u64>,
        to: Option<u64>,
        _page: u32,
    ) -> Result<SourcePage> {
        let tracks = self
            .tracks
            .iter()
            .filter(|t| t.timestamp.is_some_and(|ts| to.is_none_or(|pin| ts < pin)))
            .cloned()
            .collect();
        Ok(SourcePage {
            tracks,
            has_next: false,
        })
    }

    fn rate_limit(&self) -> RateLimitStateWatcher {
        let (tx, rx) = tokio::sync::watch::channel(RateLimitState::Ready);
        std::mem::forget(tx);
        rx
    }
}

async fn seeded_store() -> Arc<MemoryStorage> {
    let store = Arc::new(MemoryStorage::new());
    store.append_scrobbles(&[seeded_record()]).await.unwrap();
    store
}

#[tokio::test]
async fn apply_edit_enriches_applies_and_mirrors() {
    let store = seeded_store().await;
    let mut client = MockLastFmEditClient::new();
    client
        .expect_get_scrobble_edit_variations()
        .times(1)
        .returning(|_, _| Ok(vec![variation("The Diplomats")]));
    client
        .expect_edit_scrobble_single()
        .times(1)
        .returning(|edit, _| Ok(success_response(edit)));

    let editor = MirroredEditor::new(store.clone(), client);
    let mut rx = editor.subscribe();

    // prepare_edit enriches (scraping the real album artist) and gives us the template.
    let mut edit = editor.prepare_edit(&seeded_record().id).await.unwrap();
    assert_eq!(edit.album_artist_name_original, "The Diplomats");
    edit.track_name = "Oh Boy (Clean)".to_string();

    let outcome = editor.apply_edit(edit).await.unwrap();
    let new_id = ScrobbleId::new(UTS, "Cam'ron", "Oh Boy (Clean)");
    assert_eq!(
        outcome,
        EditOutcome::Applied {
            result_ids: vec![new_id.clone()]
        }
    );

    // Local mirror: new identity live and Verified, old identity tombstoned.
    let new_record = store.get_scrobble(&new_id).await.unwrap().unwrap();
    assert_eq!(new_record.track, "Oh Boy (Clean)");
    assert!(new_record.album_artist.is_verified());
    assert_eq!(new_record.source, RecordSource::EditMirror);
    let old = store
        .get_scrobble(&seeded_record().id)
        .await
        .unwrap()
        .unwrap();
    assert!(old.deleted);

    // Durable log: entry folded to Applied.
    let log = store.load_edit_log().await.unwrap();
    assert_eq!(log.len(), 1);
    assert!(matches!(log[0].state, EditState::Applied { .. }));

    // Events: queued then applied.
    let mut kinds = Vec::new();
    while let Ok(event) = rx.try_recv() {
        kinds.push(format!("{event:?}"));
    }
    assert!(kinds.iter().any(|k| k.starts_with("EditQueued")));
    assert!(kinds.iter().any(|k| k.starts_with("EditApplied")));
}

#[tokio::test]
async fn stale_album_artist_needs_rebase() {
    let store = seeded_store().await;
    let mut client = MockLastFmEditClient::new();
    client
        .expect_get_scrobble_edit_variations()
        .returning(|_, _| Ok(vec![variation("The Diplomats")]));

    let editor = MirroredEditor::new(store, client);
    // Caller guessed the album artist instead of preparing the edit properly.
    let mut edit = variation("Cam'ron");
    edit.track_name = "Renamed".to_string();
    let err = editor.apply_edit(edit).await.unwrap_err();
    assert!(matches!(err, StoreError::NeedsRebase(_)));
}

#[tokio::test]
async fn failed_attempt_stays_pending_then_resume_retries_and_succeeds() {
    let store = seeded_store().await;
    let mut client = MockLastFmEditClient::new();
    client
        .expect_get_scrobble_edit_variations()
        .returning(|_, _| Ok(vec![variation("The Diplomats")]));
    let mut attempts = 0;
    client
        .expect_edit_scrobble_single()
        .times(2)
        .returning(move |edit, _| {
            attempts += 1;
            if attempts == 1 {
                Ok(failure_response(edit, "temporary failure"))
            } else {
                Ok(success_response(edit))
            }
        });

    let editor = MirroredEditor::new(store.clone(), client);
    let mut edit = editor.prepare_edit(&seeded_record().id).await.unwrap();
    edit.album_name = "Come Home With Me (Deluxe)".to_string();

    let outcome = editor.apply_edit(edit).await.unwrap();
    assert!(matches!(outcome, EditOutcome::Failed { .. }));
    let log = store.load_edit_log().await.unwrap();
    assert_eq!(
        log[0].state,
        EditState::Pending {
            attempts: 1,
            last_error: Some("temporary failure".to_string())
        }
    );

    // Upstream still shows the ORIGINAL values (the edit really didn't land), so resume
    // re-submits — and this time it succeeds.
    let upstream = StubSource {
        tracks: vec![Track {
            name: "Oh Boy".to_string(),
            artist: "Cam'ron".to_string(),
            playcount: 1,
            timestamp: Some(UTS),
            album: Some("Come Home With Me".to_string()),
            album_artist: None,
        }],
    };
    let outcomes = editor.resume_pending(&upstream).await.unwrap();
    assert_eq!(outcomes.len(), 1);
    assert!(matches!(outcomes[0].1, EditOutcome::Applied { .. }));
    let log = store.load_edit_log().await.unwrap();
    assert!(matches!(log[0].state, EditState::Applied { .. }));
}

#[tokio::test]
async fn crash_after_upstream_success_converges_without_resubmitting() {
    let store = seeded_store().await;

    // Simulate the crash: intent was queued, upstream applied, then we died before
    // mirroring. The log holds a bare Queued event.
    let mut edit = variation("The Diplomats");
    edit.track_name = "Oh Boy (Clean)".to_string();
    store
        .append_edit_events(&[scrobble_store::EditLogEvent {
            edit_id: "e-crash-test".to_string(),
            at: 100,
            kind: scrobble_store::EditEventKind::Queued {
                op: scrobble_store::EditOp::Edit(edit.clone()),
                target_ids: vec![seeded_record().id],
            },
        }])
        .await
        .unwrap();

    // Upstream already reflects the new values.
    let upstream = StubSource {
        tracks: vec![Track {
            name: "Oh Boy (Clean)".to_string(),
            artist: "Cam'ron".to_string(),
            playcount: 1,
            timestamp: Some(UTS),
            album: Some("Come Home With Me".to_string()),
            album_artist: None,
        }],
    };

    // The client must NOT be asked to edit again.
    let mut client = MockLastFmEditClient::new();
    client.expect_edit_scrobble_single().times(0);

    let editor = MirroredEditor::new(store.clone(), client);
    let outcomes = editor.resume_pending(&upstream).await.unwrap();
    assert_eq!(outcomes.len(), 1);
    assert!(matches!(outcomes[0].1, EditOutcome::AlreadyApplied { .. }));

    // Mirror converged: new id live, old id tombstoned, log Applied.
    let new_id = ScrobbleId::new(UTS, "Cam'ron", "Oh Boy (Clean)");
    assert!(store.get_scrobble(&new_id).await.unwrap().is_some());
    assert!(
        store
            .get_scrobble(&seeded_record().id)
            .await
            .unwrap()
            .unwrap()
            .deleted
    );
    let log = store.load_edit_log().await.unwrap();
    assert!(matches!(log[0].state, EditState::Applied { .. }));
}

#[tokio::test]
async fn apply_delete_tombstones_locally() {
    let store = seeded_store().await;
    let mut client = MockLastFmEditClient::new();
    client
        .expect_delete_scrobble()
        .times(1)
        .returning(|_, _, _| Ok(true));

    let editor = MirroredEditor::new(store.clone(), client);
    let outcome = editor.apply_delete(&seeded_record().id).await.unwrap();
    assert_eq!(outcome, EditOutcome::Applied { result_ids: vec![] });
    assert!(
        store
            .get_scrobble(&seeded_record().id)
            .await
            .unwrap()
            .unwrap()
            .deleted
    );
    assert!(store
        .scrobbles_in_range(0..u64::MAX)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn rate_limit_error_propagates_without_consuming_an_attempt() {
    let store = seeded_store().await;
    let mut client = MockLastFmEditClient::new();
    client
        .expect_get_scrobble_edit_variations()
        .returning(|_, _| Ok(vec![variation("The Diplomats")]));
    client
        .expect_edit_scrobble_single()
        .times(1)
        .returning(|_, _| Err(lastfm_edit::LastFmError::RateLimit { retry_after: 60 }));

    let editor = MirroredEditor::new(store.clone(), client);
    let edit = editor.prepare_edit(&seeded_record().id).await.unwrap();
    let mut edit = edit;
    edit.track_name = "Renamed".to_string();

    let err = editor.apply_edit(edit).await.unwrap_err();
    assert!(matches!(
        err,
        StoreError::LastFm(lastfm_edit::LastFmError::RateLimit { .. })
    ));
    // Queued but zero attempts consumed: ready for a later retry.
    let log = store.load_edit_log().await.unwrap();
    assert_eq!(
        log[0].state,
        EditState::Pending {
            attempts: 0,
            last_error: None
        }
    );
}
