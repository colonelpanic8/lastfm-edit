use super::*;
use scrobble_scrubber::identity::{
    Canonical, Proof, ReleaseGroup, ResolutionEvent, ResolutionEventKind, Rung, RESOLVER_VERSION,
};
use scrobble_scrubber::{ScrobbleKey, ScrubFeed, SdRule};
use scrobble_store::{Provenanced, RecordSource, ScrobbleRecord};

fn parse(toml: &str) -> std::result::Result<Vec<&'static str>, String> {
    let config: FileConfig = toml::from_str(toml).map_err(|e| e.message().to_string())?;
    Ok(config.correctors.0.iter().map(Corrector::kind).collect())
}

#[test]
fn correctors_parse_in_order_and_reject_unknown_or_repeated_kinds() {
    let both = "[[corrector]]\nkind = \"musicbrainz\"\nmode = \"review\"\n\
                    [[corrector]]\nkind = \"rules\"\n";
    let flipped = "[[corrector]]\nkind = \"rules\"\n\
                       [[corrector]]\nkind = \"musicbrainz\"\n";
    let unknown = "[[corrector]]\nkind = \"lastfm\"\n";
    let repeated = "[[corrector]]\nkind = \"rules\"\nmode = \"auto\"\n\
                        [[corrector]]\nkind = \"rules\"\nmode = \"review\"\n";
    let cases = [
        (both, Ok(vec!["musicbrainz", "rules"])),
        (flipped, Ok(vec!["rules", "musicbrainz"])),
        ("[scrubber]\ninterval = 60\n", Ok(vec!["rules"])),
        (
            unknown,
            Err("unknown variant `lastfm`, expected `rules` or `musicbrainz`".to_string()),
        ),
        (
            repeated,
            Err("corrector kind \"rules\" is listed more than once".to_string()),
        ),
    ];
    for (toml, expected) in cases {
        assert_eq!(parse(toml), expected, "{toml}");
    }
}

#[test]
fn musicbrainz_settings_parse_from_its_corrector() {
    let config: FileConfig = toml::from_str(
        "[[corrector]]\nkind = \"musicbrainz\"\nmode = \"review\"\ncontact = \"me@example.com\"\n\
             llm = \"claude-cli\"\nresearch = false\nresearch_budget = 5\n\
             moves = { compilations = \"review\", editions = \"auto\", singles = \"off\" }\n",
    )
    .unwrap();
    let settings = config.correctors.musicbrainz().unwrap();
    assert_eq!(
        (
            settings.mode,
            settings.contact.as_str(),
            settings.llm.as_deref(),
            settings.research,
            settings.research_budget,
            format!("{:?}", settings.moves),
        ),
        (
            Ceiling::Review,
            "me@example.com",
            Some("claude-cli"),
            Some(false),
            Some(5),
            "MovePolicy { compilations: Review, editions: Auto, singles: Off }".to_string(),
        )
    );
}

#[tokio::test]
async fn identity_plans_only_when_the_musicbrainz_corrector_is_listed() {
    let mut got = Vec::new();
    for with_musicbrainz in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let rules = "[[corrector]]\nkind = \"rules\"\n";
        let toml = if with_musicbrainz {
            let cache = dir.path().join("cache").display().to_string();
            format!("[[corrector]]\nkind = \"musicbrainz\"\ncache_dir = {cache:?}\n{rules}")
        } else {
            rules.to_string()
        };
        let config: FileConfig = toml::from_str(&toml).unwrap();
        let ctx = Context {
            store: Arc::new(FsStorage::open(dir.path().join("store")).unwrap()),
            state: Arc::new(FsScrubberState::open(dir.path().join("state")).unwrap()),
            username: "tester".into(),
            config,
        };
        let track = "Wild Horses - 2009 Re-Mastered Digital Version";
        ctx.store
            .append_scrobbles(&[ScrobbleRecord {
                id: ScrobbleId::new(100, "The Rolling Stones", track),
                uts: 100,
                artist: "The Rolling Stones".into(),
                track: track.into(),
                album: Some("Sticky Fingers".into()),
                album_artist: Provenanced::Unknown,
                source: RecordSource::Api,
                fetched_at: 1,
                deleted: false,
                v: 1,
            }])
            .await
            .unwrap();
        let sticky: ReleaseGroup = serde_json::from_value(serde_json::json!({
            "id": "sticky-fingers",
            "title": "Sticky Fingers",
            "primary": "Album",
            "secondary": [],
            "first_release": null,
            "credit": { "parts": [] },
        }))
        .unwrap();
        ctx.state
            .append_resolution_events(&[ResolutionEvent {
                key: ScrobbleKey {
                    artist: "The Rolling Stones".into(),
                    track: track.into(),
                    album: Some("Sticky Fingers".into()),
                },
                at: 1,
                resolver: RESOLVER_VERSION,
                kind: ResolutionEventKind::Resolved {
                    outcome: scrobble_scrubber::identity::Outcome {
                        master: Vec::new(),
                        title: "Wild Horses".into(),
                        scrobbled_group: Some(sticky.clone()),
                        canonical: Some(Canonical {
                            group: sticky,
                            rung: Rung::PlainAlbum,
                            single_first: false,
                        }),
                        proof: Proof::SameRecording,
                    },
                },
            }])
            .await
            .unwrap();
        ctx.state
            .save_rules(&[scrobble_scrubber::RewriteRule::new()
                .with_album_name(SdRule::new(r"^(.+) \(Deluxe\)$", "$1"))])
            .await
            .unwrap();

        let (planner, _identity) = build_planner(&ctx, Policy::default(), ScrubberEventBus::new())
            .await
            .unwrap();
        planner
            .plan(&ScrubFeed::StoreRange { range: None })
            .await
            .unwrap();
        let queue = ctx.state.load_queue().await.unwrap();
        got.push(
            queue
                .iter()
                .map(|intent| {
                    (
                        intent.provider.clone(),
                        intent.proposed.track_name.clone().unwrap(),
                    )
                })
                .collect::<Vec<_>>(),
        );
    }
    assert_eq!(
        got,
        vec![
            vec![(
                "musicbrainz_identity".to_string(),
                "Wild Horses".to_string()
            )],
            vec![],
        ]
    );
}
