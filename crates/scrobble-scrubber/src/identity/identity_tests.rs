use super::*;
use crate::state::MemoryScrubberState;
use crate::subject::ScrobbleKey;
use scrobble_store::{
    MemoryStorage, Provenanced, RecordSource, ScrobbleId, ScrobbleRecord, Storage,
};

#[derive(Debug)]
struct Offline;

#[async_trait::async_trait]
impl http_client::HttpClient for Offline {
    async fn send(
        &self,
        _: http_client::Request,
    ) -> Result<http_client::Response, http_client::Error> {
        Err(http_client::Error::from_str(
            http_client::http_types::StatusCode::ServiceUnavailable,
            "offline",
        ))
    }
}

fn key(track: &str) -> ScrobbleKey {
    ScrobbleKey {
        artist: "Eric Clapton".into(),
        track: track.into(),
        album: Some("Slowhand".into()),
    }
}

fn not_found(track: &str) -> ResolutionEvent {
    ResolutionEvent {
        key: key(track),
        at: 1,
        resolver: RESOLVER_VERSION,
        kind: ResolutionEventKind::NotFound {
            queries: Vec::new(),
        },
    }
}

#[tokio::test]
async fn the_lane_starts_from_the_log_as_it_is_when_locked() {
    let state = Arc::new(MemoryScrubberState::new());
    state
        .append_resolution_events(&[not_found("Cocaine")])
        .await
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut identity = Identity::open(IdentityConfig {
        state: state.clone(),
        cache_dir: dir.path().to_path_buf(),
        http: Arc::new(Offline),
        user_agent: UserAgent::new("scrobble-scrubber-test/0.2", "ivanmalison@gmail.com").unwrap(),
        llm: None,
        moves: MovePolicy::default(),
    })
    .await
    .unwrap();
    let reader = identity.reader();
    state
        .append_resolution_events(&[not_found("Wonderful Tonight")])
        .await
        .unwrap();
    let store = Arc::new(MemoryStorage::new());
    store
        .append_scrobbles(&[ScrobbleRecord {
            id: ScrobbleId::new(1_600_000_000, "Eric Clapton", "Lay Down Sally"),
            uts: 1_600_000_000,
            artist: "Eric Clapton".into(),
            track: "Lay Down Sally".into(),
            album: Some("Slowhand".into()),
            album_artist: Provenanced::Unknown,
            source: RecordSource::Api,
            fetched_at: 1,
            deleted: false,
            v: 1,
        }])
        .await
        .unwrap();

    let mut lane = identity.lane(store).await.unwrap();

    assert_eq!(reader.changes_since(1), (vec![key("Wonderful Tonight")], 2));
    let report = lane.run_round(1).await.unwrap();
    assert_eq!(report.deferred, 1);
    assert_eq!(
        reader.changes_since(2),
        (vec![key("Lay Down Sally")], 3),
        "the lane's append continues from the head on disk"
    );
    let (_, reopened) = book::open(state).await.unwrap();
    assert_eq!(reopened.get(&key("Lay Down Sally")).map(|e| e.seq), Some(3));
}
