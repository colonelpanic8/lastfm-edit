use super::*;
use crate::feed::ScrubFeed;
use crate::planner::Planner;
use crate::queue::IntentState;
use crate::state::MemoryScrubberState;
use crate::subject::ScrobbleKey;
use scrobble_store::{
    CoverageMap, MemoryStorage, Provenanced, RecordSource, ScrobbleId, ScrobbleRecord, Segment,
    Storage,
};

fn lennon(track: &str) -> ScrobbleKey {
    ScrobbleKey {
        artist: "John Lennon".into(),
        track: track.into(),
        album: Some("Plastic Ono Band".into()),
    }
}

fn settled(track: &str) -> ResolutionEvent {
    let group = ReleaseGroup {
        id: mb::ReleaseGroupId("8be83468-4917-390d-82b6-d890f058cd22".into()),
        title: "John Lennon/Plastic Ono Band".into(),
        primary: Some(mb::PrimaryType::Album),
        secondary: Vec::new(),
        first_release: None,
        credit: ArtistCredit { parts: Vec::new() },
    };
    ResolutionEvent {
        key: lennon(track),
        at: 1,
        resolver: RESOLVER_VERSION,
        kind: ResolutionEventKind::Resolved {
            outcome: Outcome {
                master: Vec::new(),
                title: track.into(),
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

#[tokio::test]
async fn a_naming_verdict_replans_the_held_retitle() {
    let store = Arc::new(MemoryStorage::new());
    store
        .append_scrobbles(&[ScrobbleRecord {
            id: ScrobbleId::new(1_600_000_000, "John Lennon", "Mother"),
            uts: 1_600_000_000,
            artist: "John Lennon".into(),
            track: "Mother".into(),
            album: Some("Plastic Ono Band".into()),
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
    let (mut writer, reader) = book::open(state.clone()).await.unwrap();
    let planner = Planner::new(store.clone(), state.clone()).with_provider(IdentityProvider::new(
        reader,
        MovePolicy::default(),
        RetitleJudge::Llm,
    ));
    let queue = || async {
        state
            .load_queue()
            .await
            .unwrap()
            .into_iter()
            .map(|intent| (intent.state, intent.proposed.album_name))
            .collect::<Vec<_>>()
    };

    writer
        .append(vec![settled("Mother"), settled("Love")])
        .await
        .unwrap();
    planner
        .plan(&ScrubFeed::Incremental { window: None })
        .await
        .unwrap();
    assert_eq!(queue().await, vec![], "held until the LLM answers");

    writer
        .append(vec![ResolutionEvent {
            key: lennon("Love"),
            at: 2,
            resolver: RESOLVER_VERSION,
            kind: ResolutionEventKind::NamingJudged {
                question: NamingQuestion::album(
                    "John Lennon",
                    "Plastic Ono Band",
                    "John Lennon/Plastic Ono Band",
                ),
                verdict: NamingVerdict::MusicBrainz,
                reason: "The official title.".into(),
                by: NamingJudge::Llm {
                    model: "scripted".into(),
                    prompt_version: 1,
                },
                raw: String::new(),
            },
        }])
        .await
        .unwrap();
    planner
        .plan(&ScrubFeed::Incremental { window: None })
        .await
        .unwrap();
    assert_eq!(
        queue().await,
        vec![(
            IntentState::Ready,
            Some("John Lennon/Plastic Ono Band".to_string())
        )]
    );
}
