use super::*;
use crate::feed::ScrubFeed;
use crate::planner::Planner;
use crate::provider::RewriteRulesScrubActionProvider;
use crate::queue::IntentState;
use crate::state::MemoryScrubberState;
use crate::subject::ScrobbleKey;
use scrobble_store::{
    CoverageMap, MemoryStorage, Provenanced, RecordSource, ScrobbleId, ScrobbleRecord, Segment,
    Storage,
};

const SCROBBLED: &str = "(What's The Story) Morning Glory? (Deluxe Edition) [Remastered]";
const FULL: &str = "(What's The Story) Morning Glory?";
const UNGATED: &str = "(What's The Story) Morning Glory? (Deluxe Edition)";

fn wonderwall() -> ScrobbleKey {
    ScrobbleKey {
        artist: "Oasis".into(),
        track: "Wonderwall".into(),
        album: Some(SCROBBLED.into()),
    }
}

async fn library() -> (Arc<MemoryStorage>, Arc<MemoryScrubberState>) {
    let store = Arc::new(MemoryStorage::new());
    store
        .append_scrobbles(&[ScrobbleRecord {
            id: ScrobbleId::new(1_600_000_000, "Oasis", "Wonderwall"),
            uts: 1_600_000_000,
            artist: "Oasis".into(),
            track: "Wonderwall".into(),
            album: Some(SCROBBLED.into()),
            album_artist: Provenanced::Unknown,
            source: RecordSource::Api,
            fetched_at: 1,
            deleted: false,
            v: 1,
        }])
        .await
        .unwrap();
    let mut sync = CoverageMap::new();
    sync.insert(Segment::new(0, u64::MAX, 1));
    store.save_coverage(&sync).await.unwrap();
    let state = Arc::new(MemoryScrubberState::new());
    state
        .save_rules(&crate::rewrite::load_comprehensive_default_rules())
        .await
        .unwrap();
    (store, state)
}

fn planner(
    store: &Arc<MemoryStorage>,
    state: &Arc<MemoryScrubberState>,
    identity: Option<ResolutionReader>,
) -> Planner {
    let mut rules = RewriteRulesScrubActionProvider::from_rules(Vec::new());
    if let Some(reader) = identity {
        rules = rules.with_identity(reader);
    }
    Planner::new(store.clone(), state.clone()).with_provider(rules)
}

async fn queue(state: &MemoryScrubberState) -> Vec<(IntentState, String)> {
    state
        .load_queue()
        .await
        .unwrap()
        .into_iter()
        .map(|intent| (intent.state, intent.proposed.album_name.unwrap()))
        .collect()
}

/// The key settled on a recording proven to be on `album`.
fn settled_on(album: &str) -> ResolutionEvent {
    let group = mb::ReleaseGroup {
        id: mb::ReleaseGroupId(album.to_lowercase()),
        title: album.into(),
        primary: Some(mb::PrimaryType::Album),
        secondary: Vec::new(),
        first_release: None,
        credit: ArtistCredit { parts: Vec::new() },
    };
    ResolutionEvent {
        key: wonderwall(),
        at: 2,
        resolver: RESOLVER_VERSION,
        kind: ResolutionEventKind::Resolved {
            outcome: Outcome {
                master: Vec::new(),
                title: "Wonderwall".into(),
                scrobbled_group: Some(group.clone()),
                canonical: Some(Canonical {
                    group,
                    rung: Rung::PlainAlbum,
                    single_first: false,
                }),
                proof: Proof::SameRecording,
            },
        },
    }
}

fn deferred() -> ResolutionEvent {
    ResolutionEvent {
        key: wonderwall(),
        at: 1,
        resolver: RESOLVER_VERSION,
        kind: ResolutionEventKind::Deferred {
            error: "rate limited".into(),
            retry_after: 60,
        },
    }
}

#[tokio::test]
async fn an_unresolved_key_queues_the_full_gated_edit_for_review() {
    let (store, state) = library().await;
    let (_writer, reader) = book::open(state.clone()).await.unwrap();

    planner(&store, &state, Some(reader))
        .plan(&ScrubFeed::Incremental { window: None })
        .await
        .unwrap();

    assert_eq!(
        queue(&state).await,
        vec![(IntentState::AwaitingApproval, FULL.to_string())]
    );
}

#[tokio::test]
async fn replanning_an_unresolved_key_keeps_its_awaiting_gated_edit() {
    let (store, state) = library().await;
    planner(&store, &state, None)
        .plan(&ScrubFeed::Incremental { window: None })
        .await
        .unwrap();
    let (mut writer, reader) = book::open(state.clone()).await.unwrap();
    writer.append(vec![deferred()]).await.unwrap();

    let identity = planner(&store, &state, Some(reader));
    let drained = identity
        .plan(&ScrubFeed::Incremental { window: None })
        .await
        .unwrap();
    let rescan = identity
        .plan(&ScrubFeed::StoreRange { range: None })
        .await
        .unwrap();

    assert_eq!((drained.superseded, rescan.superseded), (0, 0), "no churn");
    assert_eq!(
        queue(&state).await,
        vec![(IntentState::AwaitingApproval, FULL.to_string())]
    );
}

#[tokio::test]
async fn settling_the_key_replans_its_gated_edit() {
    let superseded = || IntentState::Abandoned {
        reason: crate::planner::SUPERSEDED.into(),
    };
    let mut got = Vec::new();
    for with_musicbrainz in [false, true] {
        for album in [FULL, "Be Here Now"] {
            let (store, state) = library().await;
            let (mut writer, reader) = book::open(state.clone()).await.unwrap();
            let mut planner = planner(&store, &state, Some(reader.clone()));
            if with_musicbrainz {
                planner = planner.with_provider(IdentityProvider::new(
                    reader,
                    MovePolicy::default(),
                    RetitleJudge::Llm,
                ));
            }
            planner
                .plan(&ScrubFeed::Incremental { window: None })
                .await
                .unwrap();

            writer.append(vec![settled_on(album)]).await.unwrap();
            planner
                .plan(&ScrubFeed::Incremental { window: None })
                .await
                .unwrap();
            got.push((with_musicbrainz, queue(&state).await));
        }
    }

    let confirmed = vec![
        (superseded(), FULL.to_string()),
        (IntentState::Ready, FULL.to_string()),
    ];
    let contradicted = vec![
        (superseded(), FULL.to_string()),
        (IntentState::Ready, UNGATED.to_string()),
    ];
    assert_eq!(
        got,
        vec![
            (false, confirmed.clone()),
            (false, contradicted.clone()),
            (true, confirmed),
            (true, contradicted),
        ]
    );
}
