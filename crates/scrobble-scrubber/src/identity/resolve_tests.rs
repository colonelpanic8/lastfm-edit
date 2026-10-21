use super::*;
use crate::identity::outcome::{derive_edit, Approval, Chosen, MovePolicy, Naming};
use crate::identity::testing::*;
use crate::subject::Subject;

fn key(artist: &str, track: &str, album: Option<&str>) -> ScrobbleKey {
    ScrobbleKey {
        artist: artist.into(),
        track: track.into(),
        album: album.map(str::to_string),
    }
}

async fn resolve(mb: &FixtureMb, key: &ScrobbleKey) -> Resolution {
    Resolver {
        mb,
        limits: ResolveLimits::default(),
    }
    .resolve(key)
    .await
    .unwrap()
}

fn plain(group: ReleaseGroup) -> Option<Canonical> {
    Some(Canonical {
        group,
        rung: Rung::PlainAlbum,
        single_first: false,
    })
}

#[tokio::test]
async fn compilation_track_resolves_to_studio_album_through_shared_isrc() {
    let mb = FixtureMb::default().cocaine();
    let resolution = resolve(
        &mb,
        &key("Eric Clapton", "Cocaine", Some("The Cream of Clapton")),
    )
    .await;
    assert_eq!(
        resolution,
        Resolution::Resolved(Outcome {
            master: vec![rid(COCAINE_CREAM), rid(COCAINE_STUDIO)],
            title: "Cocaine".into(),
            scrobbled_group: Some(cream_of_clapton()),
            canonical: plain(slowhand()),
            proof: Proof::SharedIsrc {
                isrc: "NLF057790024".into(),
                len_delta_ms: 2_107,
            },
        })
    );
}

#[tokio::test]
async fn plain_album_edition_takes_one_search_and_no_browse() {
    let mb = FixtureMb::default().cocaine();
    let resolution = resolve(
        &mb,
        &key(
            "Eric Clapton",
            "Cocaine",
            Some("Slowhand 35th Anniversary (Super Deluxe)"),
        ),
    )
    .await;
    assert_eq!(
        resolution,
        Resolution::Resolved(Outcome {
            master: vec![rid(COCAINE_51)],
            title: "Cocaine".into(),
            scrobbled_group: Some(slowhand()),
            canonical: plain(slowhand()),
            proof: Proof::SameRecording,
        })
    );
    assert_eq!(mb.calls(), vec!["search Cocaine | Eric Clapton | Slowhand"]);
}

#[tokio::test]
async fn remaster_suffix_resolves_to_the_plain_mb_title() {
    let mb = FixtureMb::default().wild_horses();
    let resolution = resolve(
        &mb,
        &key(
            "The Rolling Stones",
            "Wild Horses - 2009 Re-Mastered Digital Version",
            Some("Sticky Fingers"),
        ),
    )
    .await;
    assert_eq!(
        resolution,
        Resolution::Resolved(Outcome {
            master: vec![rid("5a6062a4-c790-4c29-84c3-080ea9e332ef")],
            title: "Wild Horses".into(),
            scrobbled_group: Some(sticky_fingers()),
            canonical: plain(sticky_fingers()),
            proof: Proof::SameRecording,
        })
    );
    assert_eq!(
        mb.calls(),
        vec!["search Wild Horses | The Rolling Stones | Sticky Fingers"]
    );
}

#[tokio::test]
async fn live_album_track_stays_on_the_live_album() {
    let mb = FixtureMb::default().four_on_six();
    let resolution = resolve(
        &mb,
        &key(
            "Wes Montgomery",
            "Four on Six",
            Some("Smokin' at the Half Note"),
        ),
    )
    .await;
    let Resolution::Resolved(outcome) = &resolution else {
        panic!("expected a resolution, got {resolution:?}");
    };
    assert_eq!(
        outcome,
        &Outcome {
            master: vec![rid(FOUR_ON_SIX_LIVE)],
            title: "Four on Six".into(),
            scrobbled_group: Some(smokin()),
            canonical: None,
            proof: Proof::SameRecording,
        }
    );
}

#[tokio::test]
async fn candidates_implying_different_edits_are_ambiguous() {
    let mb = FixtureMb::default().wild_horses_on_honk();
    let resolution = resolve(&mb, &key("The Rolling Stones", "Wild Horses", Some("Honk"))).await;
    assert_eq!(
        resolution,
        Resolution::Ambiguous(vec![
            (
                Outcome {
                    master: vec![rid(WILD_HORSES_2009)],
                    title: "Wild Horses".into(),
                    scrobbled_group: Some(honk()),
                    canonical: plain(sticky_fingers()),
                    proof: Proof::SameRecording,
                },
                OptionEvidence {
                    recordings: vec![(
                        WILD_HORSES_2009.into(),
                        "Wild Horses".into(),
                        "2009 mix".into(),
                        Some(343_000)
                    )],
                    isrcs: vec!["GBUM70909466".into()],
                    work_attributes: vec![],
                    scrobbled_group: Some("Honk (Album + Compilation, 2019)".into()),
                    canonical_group: Some("Sticky Fingers (Album, 1971)".into()),
                },
            ),
            (
                Outcome {
                    master: vec![rid(WILD_HORSES_COMP)],
                    title: "Wild Horses".into(),
                    scrobbled_group: Some(honk()),
                    canonical: None,
                    proof: Proof::SameRecording,
                },
                OptionEvidence {
                    recordings: vec![(
                        WILD_HORSES_COMP.into(),
                        "Wild Horses".into(),
                        "".into(),
                        Some(342_000)
                    )],
                    isrcs: vec![],
                    work_attributes: vec![],
                    scrobbled_group: Some("Honk (Album + Compilation, 2019)".into()),
                    canonical_group: None,
                },
            ),
        ])
    );
}

#[tokio::test]
async fn isrc_siblings_must_share_the_title_and_an_artist() {
    let ocean_boulevard = group(
        "461-ocean-boulevard",
        "461 Ocean Boulevard",
        PrimaryType::Album,
        &[],
        1974,
        clapton(),
    );
    let jj_cale = || credit(&[("jj-cale", "J.J. Cale", "")]);
    let mb = FixtureMb::default()
        .cocaine()
        .with_isrc(
            "NLF057790024",
            &[
                COCAINE_CREAM,
                COCAINE_STUDIO,
                "i-shot-the-sheriff",
                "cale-cocaine",
            ],
        )
        .with_recording(detail(
            "i-shot-the-sheriff",
            "I Shot the Sheriff",
            "",
            218_000,
            clapton(),
            &["NLF057790024"],
            vec![],
        ))
        .with_recording(detail(
            "cale-cocaine",
            "Cocaine",
            "",
            219_000,
            jj_cale(),
            &["NLF057790024"],
            vec![],
        ))
        .with_releases("i-shot-the-sheriff", vec![rel(&ocean_boulevard)])
        .with_releases(
            "cale-cocaine",
            vec![rel(&group(
                "troubadour",
                "Troubadour",
                PrimaryType::Album,
                &[],
                1976,
                jj_cale(),
            ))],
        );
    let resolution = resolve(
        &mb,
        &key("Eric Clapton", "Cocaine", Some("The Cream of Clapton")),
    )
    .await;
    assert_eq!(
        resolution,
        Resolution::Resolved(Outcome {
            master: vec![rid(COCAINE_CREAM), rid(COCAINE_STUDIO)],
            title: "Cocaine".into(),
            scrobbled_group: Some(cream_of_clapton()),
            canonical: plain(slowhand()),
            proof: Proof::SharedIsrc {
                isrc: "NLF057790024".into(),
                len_delta_ms: 2_107,
            },
        })
    );
}

#[tokio::test]
async fn isrc_siblings_must_be_the_same_version() {
    let beatles = || credit(&[("b10bbbfc-cf9e-42e0-be17-e2c3e1d2600d", "The Beatles", "")]);
    let one = group(
        "one",
        "1",
        PrimaryType::Album,
        &[SecondaryType::Compilation],
        2015,
        beatles(),
    );
    let magical_mystery_tour = group(
        "magical-mystery-tour",
        "Magical Mystery Tour",
        PrimaryType::Album,
        &[],
        1967,
        beatles(),
    );
    let remix = "strawberry-fields-2015-remix";
    let original = "strawberry-fields-original";
    // (how MB names the remix: title, disambiguation)
    let cases = [
        ("Strawberry Fields Forever (2015 Remix)", ""),
        ("Strawberry Fields Forever", "2015 remix"),
    ];
    for (title, disambiguation) in cases {
        let mb = FixtureMb::default()
            .with_search(
                "Strawberry Fields Forever",
                "The Beatles",
                Some("1"),
                vec![hit(
                    remix,
                    100,
                    title,
                    disambiguation,
                    247_000,
                    beatles(),
                    vec![rel(&one)],
                )],
            )
            .with_recording(detail(
                remix,
                title,
                disambiguation,
                247_000,
                beatles(),
                &["GBUM71505902"],
                vec![],
            ))
            .with_recording(detail(
                original,
                "Strawberry Fields Forever",
                "",
                248_000,
                beatles(),
                &["GBUM71505902"],
                vec![],
            ))
            .with_isrc("GBUM71505902", &[remix, original])
            .with_releases(remix, vec![rel(&one)])
            .with_releases(original, vec![rel(&magical_mystery_tour)]);

        let resolution = resolve(
            &mb,
            &key(
                "The Beatles",
                "Strawberry Fields Forever - 2015 Remix",
                Some("1"),
            ),
        )
        .await;

        assert_eq!(
            resolution,
            Resolution::Resolved(Outcome {
                master: vec![rid(remix)],
                title: title.into(),
                scrobbled_group: Some(one.clone()),
                canonical: None,
                proof: Proof::SameRecording,
            }),
            "{title} ({disambiguation})"
        );
    }
}

#[tokio::test]
async fn a_clean_scrobble_matches_only_the_clean_recording() {
    let outkast = || credit(&[("73fdb566-a9b1-494c-9f32-51768ec9fd56", "OutKast", "")]);
    let speakerboxxx = group(
        "speakerboxxx",
        "Speakerboxxx/The Love Below",
        PrimaryType::Album,
        &[],
        2003,
        outkast(),
    );
    let explicit = "hey-ya-explicit";
    let clean = "hey-ya-clean";
    let take = |id: &str, disambiguation: &str| {
        hit(
            id,
            100,
            "Hey Ya!",
            disambiguation,
            235_000,
            outkast(),
            vec![rel(&speakerboxxx)],
        )
    };
    let mb = FixtureMb::default().with_search(
        "Hey Ya!",
        "OutKast",
        Some("Speakerboxxx/The Love Below"),
        vec![take(explicit, ""), take(clean, "clean")],
    );
    let scrobble = key(
        "OutKast",
        "Hey Ya! (Clean)",
        Some("Speakerboxxx/The Love Below"),
    );

    let resolution = resolve(&mb, &scrobble).await;

    let expected = Outcome {
        master: vec![rid(clean)],
        title: "Hey Ya!".into(),
        scrobbled_group: Some(speakerboxxx.clone()),
        canonical: plain(speakerboxxx),
        proof: Proof::SameRecording,
    };
    assert_eq!(resolution, Resolution::Resolved(expected.clone()));
    let subject = Subject {
        artist: scrobble.artist,
        track: scrobble.track,
        album: scrobble.album,
        album_artist: None,
    };
    assert_eq!(
        derive_edit(
            &subject,
            &expected,
            Chosen::Deterministic,
            &[],
            &MovePolicy::default(),
            Naming::reviewer(),
        )
        .proposal,
        None,
        "\"(Clean)\" names the version and is never stripped"
    );
}

#[tokio::test]
async fn a_truncated_search_never_proves_the_match() {
    let mut mb = FixtureMb::default().cocaine();
    mb.truncated_searches.insert(query(
        "Cocaine",
        "Eric Clapton",
        Some("The Cream of Clapton"),
    ));
    let scrobble = key("Eric Clapton", "Cocaine", Some("The Cream of Clapton"));

    let resolution = resolve(&mb, &scrobble).await;

    let expected = Outcome {
        master: vec![rid(COCAINE_CREAM), rid(COCAINE_STUDIO)],
        title: "Cocaine".into(),
        scrobbled_group: Some(cream_of_clapton()),
        canonical: plain(slowhand()),
        proof: Proof::TitleOnly,
    };
    assert_eq!(resolution, Resolution::Resolved(expected.clone()));
    let subject = Subject {
        artist: scrobble.artist,
        track: scrobble.track,
        album: scrobble.album,
        album_artist: None,
    };
    let proposal = derive_edit(
        &subject,
        &expected,
        Chosen::Deterministic,
        &[],
        &MovePolicy::default(),
        Naming::reviewer(),
    )
    .proposal
    .expect("the move is still proposed");
    assert_eq!(
        (proposal.target.album.as_deref(), proposal.approval),
        (Some("Slowhand"), Approval::Review)
    );
}

#[test]
fn single_suffix_only_matches_a_single_or_ep_group() {
    let prince = || credit(&[("prince", "Prince", "")]);
    let album = group(
        "purple-rain-album",
        "Purple Rain",
        PrimaryType::Album,
        &[],
        1984,
        prince(),
    );
    let single = group(
        "purple-rain-single",
        "Purple Rain",
        PrimaryType::Single,
        &[],
        1984,
        prince(),
    );
    let title =
        |releases: &[ReleaseRef], album: &str| scrobbled_group(releases, album).map(|g| g.id.0);
    let both = [rel(&album), rel(&single)];
    assert_eq!(title(&[rel(&album)], "Purple Rain - Single"), None);
    assert_eq!(
        title(&both, "Purple Rain - Single"),
        Some("purple-rain-single".into())
    );
    assert_eq!(
        title(&both, "Purple Rain (Deluxe Edition)"),
        Some("purple-rain-album".into())
    );
    assert_eq!(
        title(&both, "Purple Rain"),
        Some("purple-rain-album".into())
    );
}

#[tokio::test]
async fn compilation_track_without_isrc_link_finds_the_studio_take_by_work() {
    let mut mb = FixtureMb::default().cocaine();
    mb.recordings
        .get_mut(&rid(COCAINE_CREAM))
        .unwrap()
        .isrcs
        .clear();
    let resolution = resolve(
        &mb,
        &key("Eric Clapton", "Cocaine", Some("The Cream of Clapton")),
    )
    .await;
    assert_eq!(
        resolution,
        Resolution::Resolved(Outcome {
            master: vec![rid(COCAINE_CREAM), rid(COCAINE_STUDIO)],
            title: "Cocaine".into(),
            scrobbled_group: Some(cream_of_clapton()),
            canonical: plain(slowhand()),
            proof: Proof::SameWork,
        })
    );
}

#[tokio::test]
async fn truncated_browse_offers_staying_put() {
    let mut mb = FixtureMb::default().cocaine();
    mb.releases
        .get_mut(&rid(COCAINE_STUDIO))
        .unwrap()
        .push(rel(&cream_of_clapton()));
    let limits = ResolveLimits {
        max_releases: 3,
        ..ResolveLimits::default()
    };
    let resolution = Resolver { mb: &mb, limits }
        .resolve(&key(
            "Eric Clapton",
            "Cocaine",
            Some("The Cream of Clapton"),
        ))
        .await
        .unwrap();
    let Resolution::Ambiguous(options) = resolution else {
        panic!("expected ambiguity, got {resolution:?}");
    };
    let canonicals: Vec<_> = options
        .iter()
        .map(|(outcome, _)| outcome.canonical.as_ref().map(|c| c.group.title.as_str()))
        .collect();
    assert_eq!(canonicals, vec![None, Some("Slowhand")]);
}

#[tokio::test]
async fn untitled_album_falls_back_to_title_only() {
    let mb = FixtureMb::default().with_search(
        "Cocaine",
        "Eric Clapton",
        None,
        vec![hit(
            COCAINE_STUDIO,
            100,
            "Cocaine",
            "original studio mix",
            216_640,
            clapton(),
            vec![],
        )],
    );
    let resolution = resolve(&mb, &key("Eric Clapton", "Cocaine", None)).await;
    assert_eq!(
        resolution,
        Resolution::Resolved(Outcome {
            master: vec![rid(COCAINE_STUDIO)],
            title: "Cocaine".into(),
            scrobbled_group: None,
            canonical: None,
            proof: Proof::TitleOnly,
        })
    );
}

#[tokio::test]
async fn empty_searches_walk_the_whole_ladder() {
    let mb = FixtureMb::default();
    let resolution = resolve(
        &mb,
        &key(
            "The Rolling Stones",
            "Wild Horses - 2009 Re-Mastered Digital Version",
            Some("Sticky Fingers (Deluxe Edition)"),
        ),
    )
    .await;
    assert_eq!(
        mb.calls(),
        vec![
            "search Wild Horses | The Rolling Stones | Sticky Fingers",
            "search Wild Horses | The Rolling Stones",
            "search Wild Horses - 2009 Re-Mastered Digital Version | The Rolling Stones",
        ]
    );
    assert_eq!(
        resolution,
        Resolution::NotFound {
            queries: vec![
                query("Wild Horses", "The Rolling Stones", Some("Sticky Fingers")).lucene(),
                query("Wild Horses", "The Rolling Stones", None).lucene(),
                query(
                    "Wild Horses - 2009 Re-Mastered Digital Version",
                    "The Rolling Stones",
                    None
                )
                .lucene(),
            ]
        }
    );
}

#[tokio::test]
async fn credited_remix_only_matches_the_same_remixer() {
    let jack_u = || credit(&[("0eb2a6ef-5aa5-4b8f-8d2e-4dc9d0d3e3b1", "Jack Ü", "")]);
    let remix = |id: &str, title: &str| hit(id, 100, title, "", 200_000, jack_u(), vec![]);
    let mb = FixtureMb::default().with_search(
        "To Ü",
        "Jack Ü",
        None,
        vec![
            remix(
                "11111111-1111-1111-1111-111111111111",
                "To Ü (Manikin remix)",
            ),
            remix(
                "22222222-2222-2222-2222-222222222222",
                "To Ü (Oliver remix)",
            ),
        ],
    );

    let resolution = resolve(&mb, &key("Jack Ü", "To Ü - Oliver Remix", None)).await;

    let Resolution::Resolved(outcome) = resolution else {
        panic!("expected a single outcome, got {resolution:?}");
    };
    assert_eq!(outcome.title, "To Ü (Oliver remix)");
    assert_eq!(
        outcome.master,
        vec![rid("22222222-2222-2222-2222-222222222222")]
    );
}

#[tokio::test]
async fn typographic_title_variants_are_one_outcome() {
    let dylan = || credit(&[("72c536dc-4b47-4f12-8d8a-1d4b8e3a5d2b", "Bob Dylan", "")]);
    let freewheelin = group(
        "c0cbd0b3-5b7e-3d1b-a8e3-1a5f4b6f1d0e",
        "The Freewheelin’ Bob Dylan",
        PrimaryType::Album,
        &[],
        1963,
        dylan(),
    );
    let take = |id: &str, title: &str| {
        hit(
            id,
            100,
            title,
            "",
            220_000,
            dylan(),
            vec![rel(&freewheelin)],
        )
    };
    let mb = FixtureMb::default().with_search(
        "Don't Think Twice, It's All Right",
        "Bob Dylan",
        Some("The Freewheelin' Bob Dylan"),
        vec![
            take(
                "33333333-3333-3333-3333-333333333333",
                "Don't Think Twice, It's All Right",
            ),
            take(
                "44444444-4444-4444-4444-444444444444",
                "Don’t Think Twice, It’s All Right",
            ),
        ],
    );

    let resolution = resolve(
        &mb,
        &key(
            "Bob Dylan",
            "Don't Think Twice, It's All Right",
            Some("The Freewheelin' Bob Dylan"),
        ),
    )
    .await;

    assert!(
        matches!(resolution, Resolution::Resolved(_)),
        "expected one outcome, got {resolution:?}"
    );
}

fn waltz_for_debby() -> ScrobbleKey {
    key("Bill Evans", "Waltz for Debby", Some("Waltz For Debby"))
}

#[tokio::test]
async fn a_trio_credit_matches_its_leader() {
    let mb = FixtureMb::default().waltz_for_debby().with_search(
        "Waltz for Debby",
        "Bill Evans",
        Some("Waltz For Debby"),
        vec![hit(
            WALTZ_FOR_DEBBY_TAKE_2,
            100,
            "Waltz for Debby",
            "take 2 - master",
            420_000,
            evans_trio_with_lafaro_and_motian(),
            vec![rel(&waltz_for_debby_live())],
        )],
    );

    let resolution = resolve(&mb, &waltz_for_debby()).await;

    let Resolution::Resolved(outcome) = resolution else {
        panic!("expected a resolution, got {resolution:?}");
    };
    assert_eq!(outcome.scrobbled_group, Some(waltz_for_debby_live()));
}

#[test]
fn ensemble_credits_match_only_their_leader_by_whole_words() {
    let one = |name: &str| credit(&[("id", name, "")]);
    let cases = [
        ("Bill Evans", "Bill Evans Trio", true),
        ("Dave Brubeck", "The Dave Brubeck Quartet", true),
        ("Bob Marley", "Bob Marley & The Wailers", true),
        ("Count Basie", "Count Basie Orchestra", true),
        ("Steve Miller", "Steve Miller Bandit", false),
        ("Bill", "Bill Evans Trio", false),
        ("Bill Evans", "Bill Evans Quintet Revisited", true),
        ("Bill Evans", "Tony Bennett & Bill Evans", false),
    ];
    for (artist, credited, expected) in cases {
        assert_eq!(
            artist_matches(artist, &one(credited)),
            expected,
            "{artist} vs {credited}"
        );
    }
}

async fn verify(mb: &FixtureMb, recording: &str, release_group: Option<&str>) -> Option<Outcome> {
    Resolver {
        mb,
        limits: ResolveLimits::default(),
    }
    .verify_research(
        &waltz_for_debby(),
        &rid(recording),
        release_group.map(|id| ReleaseGroupId(id.into())).as_ref(),
        "claude:opus",
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn verified_research_settles_waltz_for_debby_on_the_live_album_without_an_edit() {
    let mb = FixtureMb::default().waltz_for_debby();

    let outcome = verify(
        &mb,
        WALTZ_FOR_DEBBY_TAKE_2,
        Some("392e7fcf-5ae9-37f6-b8c9-57901cfbee1f"),
    )
    .await;

    let expected = Outcome {
        master: vec![rid(WALTZ_FOR_DEBBY_TAKE_2)],
        title: "Waltz for Debby".into(),
        scrobbled_group: Some(waltz_for_debby_live()),
        canonical: None,
        proof: Proof::Researched {
            by: "claude:opus".into(),
        },
    };
    assert_eq!(outcome, Some(expected.clone()));
    let subject = Subject {
        artist: "Bill Evans".into(),
        track: "Waltz for Debby".into(),
        album: Some("Waltz For Debby".into()),
        album_artist: None,
    };
    assert_eq!(
        derive_edit(
            &subject,
            &expected,
            Chosen::Adjudicated,
            &[],
            &MovePolicy::default(),
            Naming::reviewer(),
        )
        .proposal,
        None
    );
}

#[tokio::test]
async fn research_answers_that_are_not_this_scrobble_fail_verification() {
    let my_foolish_heart = "8a3a8a4e-2c39-4d2f-9f0b-3e3c0c7c1a11";
    let zetterlund = "0f4e2b8b-6a4c-4b8e-8f6a-2a3b8d6c9e21";
    let mb = FixtureMb::default()
        .waltz_for_debby()
        .with_recording(detail(
            my_foolish_heart,
            "My Foolish Heart",
            "",
            296_000,
            evans_trio_with_lafaro_and_motian(),
            &[],
            vec![],
        ))
        .with_releases(my_foolish_heart, vec![rel(&waltz_for_debby_live())])
        .with_recording(detail(
            zetterlund,
            "Waltz for Debby",
            "",
            165_000,
            credit(&[("monica-zetterlund", "Monica Zetterlund", "")]),
            &[],
            vec![],
        ));
    let cases = [
        (
            "unknown to MusicBrainz",
            "1d0b6fbe-3f4e-4e43-9b52-0d3a2c1e5f77",
        ),
        ("another song on the album", my_foolish_heart),
        ("another artist's recording", zetterlund),
    ];
    for (name, recording) in cases {
        assert_eq!(
            verify(&mb, recording, Some("392e7fcf-5ae9-37f6-b8c9-57901cfbee1f")).await,
            None,
            "{name}"
        );
    }
}
