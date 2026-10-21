use super::*;
use crate::identity::adjudicate::{CaseOption, OptionEvidence, OptionId};
use crate::identity::canonical::Rung;
use crate::identity::mb::{ArtistCredit, PrimaryType, ReleaseGroup, SecondaryType};
use crate::identity::Canonical;
use crate::identity::RESOLVER_VERSION;
use crate::state::MemoryScrubberState;

fn key(artist: &str, track: &str, album: &str) -> ScrobbleKey {
    ScrobbleKey {
        artist: artist.into(),
        track: track.into(),
        album: Some(album.into()),
    }
}

fn cocaine() -> ScrobbleKey {
    key("Eric Clapton", "Cocaine", "The Cream of Clapton")
}

fn group(id: &str, title: &str, secondary: Vec<SecondaryType>) -> ReleaseGroup {
    ReleaseGroup {
        id: ReleaseGroupId(id.into()),
        title: title.into(),
        primary: Some(PrimaryType::Album),
        secondary,
        first_release: None,
        credit: ArtistCredit::default(),
    }
}

/// Cocaine scrobbled on The Cream of Clapton, proposed onto `canonical`.
fn outcome(recording: &str, canonical: &str) -> Outcome {
    Outcome {
        master: vec![RecordingId(recording.into())],
        title: "Cocaine".into(),
        scrobbled_group: Some(group(
            "cream",
            "The Cream of Clapton",
            vec![SecondaryType::Compilation],
        )),
        canonical: Some(Canonical {
            group: group(canonical, canonical, Vec::new()),
            rung: Rung::PlainAlbum,
            single_first: false,
        }),
        proof: Proof::SameRecording,
    }
}

fn case(options: &[(char, &str)]) -> Case {
    Case {
        key: cocaine(),
        plays: 12,
        siblings: Vec::new(),
        options: options
            .iter()
            .map(|(id, canonical)| CaseOption {
                id: OptionId(*id),
                outcome: outcome(id.to_string().as_str(), canonical),
                evidence: OptionEvidence::default(),
            })
            .collect(),
    }
}

fn verdict(choice: Choice) -> Verdict {
    Verdict {
        choice,
        confidence: Confidence::High,
        reason: String::new(),
        invalid: false,
    }
}

fn target(album: &str) -> Target {
    Target {
        title: "Cocaine".into(),
        album: Some(album.into()),
        album_artist: Some("Eric Clapton".into()),
    }
}

fn event(at: u64, resolver: u32, kind: ResolutionEventKind) -> ResolutionEvent {
    ResolutionEvent {
        key: cocaine(),
        at,
        resolver,
        kind,
    }
}

fn resolved(at: u64, resolver: u32) -> ResolutionEvent {
    event(
        at,
        resolver,
        ResolutionEventKind::Resolved {
            outcome: outcome("40ae4c06", "Slowhand"),
        },
    )
}

fn not_found(at: u64, resolver: u32) -> ResolutionEvent {
    event(
        at,
        resolver,
        ResolutionEventKind::NotFound {
            queries: Vec::new(),
        },
    )
}

fn rejected(at: u64, resolver: u32, intent: u128, album: &str) -> ResolutionEvent {
    event(
        at,
        resolver,
        ResolutionEventKind::HumanRejected {
            intent: Uuid::from_u128(intent),
            target: target(album),
        },
    )
}

fn entry(status: KeyStatus, rejected: Vec<Target>, seq: u64, at: u64) -> KeyEntry {
    KeyEntry {
        status,
        rejected,
        seq,
        at,
        research: None,
    }
}

#[test]
fn fold_takes_the_last_current_status_and_keeps_every_rejection() {
    let old = RESOLVER_VERSION - 1;
    let now = RESOLVER_VERSION;
    let settled = KeyStatus::Settled {
        outcome: outcome("40ae4c06", "Slowhand"),
    };
    let cases = [
        (
            "older version only",
            vec![resolved(10, old)],
            entry(KeyStatus::Stale, vec![], 1, 10),
        ),
        (
            "older version after a current one is ignored",
            vec![resolved(10, now), not_found(20, old)],
            entry(settled.clone(), vec![], 2, 10),
        ),
        (
            "last current status wins",
            vec![not_found(10, now), resolved(20, now)],
            entry(settled.clone(), vec![], 2, 20),
        ),
        (
            "deferral replaces a settlement",
            vec![
                resolved(10, now),
                event(
                    20,
                    now,
                    ResolutionEventKind::Deferred {
                        error: "503".into(),
                        retry_after: 99,
                    },
                ),
            ],
            entry(KeyStatus::Deferred { retry_after: 99 }, vec![], 2, 20),
        ),
        (
            "rejections alone are stale",
            vec![rejected(10, now, 1, "Slowhand")],
            entry(KeyStatus::Stale, vec![target("Slowhand")], 1, 10),
        ),
        (
            "rejections count across versions, once per target",
            vec![
                rejected(10, old, 1, "Slowhand"),
                resolved(20, now),
                rejected(30, now, 2, "Slowhand"),
                rejected(40, now, 3, "Backless"),
            ],
            entry(settled, vec![target("Slowhand"), target("Backless")], 4, 20),
        ),
    ];
    for (name, events, expected) in cases {
        let snapshot = fold(events);
        assert_eq!(snapshot.keys.get(&cocaine()), Some(&expected), "{name}");
    }

    let snapshot = fold(vec![
        rejected(10, old, 1, "Slowhand"),
        rejected(20, now, 2, "Backless"),
    ]);
    assert_eq!(
        snapshot.consumed_rejections,
        HashSet::from([Uuid::from_u128(1), Uuid::from_u128(2)])
    );
}

#[test]
fn verdicts_attach_only_to_the_open_case_they_were_given_for() {
    let open = case(&[('A', "Slowhand"), ('B', "Backless")]);
    let other = case(&[('A', "Slowhand"), ('B', "461 Ocean Boulevard")]);
    let adjudicated = |at, case: &Case, choice| {
        event(
            at,
            RESOLVER_VERSION,
            ResolutionEventKind::Adjudicated {
                case_hash: case_hash(case),
                verdict: verdict(choice),
                by: "model".into(),
                prompt_version: 1,
                raw: String::new(),
            },
        )
    };
    let snapshot = fold(vec![
        event(
            10,
            RESOLVER_VERSION,
            ResolutionEventKind::Ambiguous {
                case: open.clone(),
                awaiting_llm: true,
            },
        ),
        adjudicated(20, &other, Choice::Keep),
        adjudicated(30, &open, Choice::Option(OptionId('B'))),
    ]);
    assert_eq!(
        snapshot.keys[&cocaine()].status,
        KeyStatus::Open {
            case: open,
            awaiting_llm: true,
            verdict: Some(verdict(Choice::Option(OptionId('B')))),
        }
    );
    assert_eq!(snapshot.adjudication_times, vec![20, 30]);
}

#[test]
fn plannable_orders_outcomes_by_status_and_verdict() {
    let open = |awaiting_llm, choice: Option<Choice>| KeyStatus::Open {
        case: case(&[('A', "Slowhand"), ('B', "Backless"), ('C', "Unplugged")]),
        awaiting_llm,
        verdict: choice.map(verdict),
    };
    let cases = [
        (
            "settled",
            KeyStatus::Settled {
                outcome: outcome("40ae4c06", "Slowhand"),
            },
            vec![("Slowhand", Chosen::Deterministic)],
        ),
        (
            "settled by research",
            KeyStatus::Settled {
                outcome: Outcome {
                    proof: Proof::Researched {
                        by: "claude:opus".into(),
                    },
                    ..outcome("d31d4528", "Slowhand")
                },
            },
            vec![("Slowhand", Chosen::Adjudicated)],
        ),
        (
            "llm picked B",
            open(true, Some(Choice::Option(OptionId('B')))),
            vec![
                ("Backless", Chosen::Adjudicated),
                ("Slowhand", Chosen::Adjudicated),
                ("Unplugged", Chosen::Adjudicated),
            ],
        ),
        ("llm said keep", open(true, Some(Choice::Keep)), vec![]),
        (
            "llm unsure",
            open(true, Some(Choice::Unsure)),
            vec![
                ("Slowhand", Chosen::Adjudicated),
                ("Backless", Chosen::Adjudicated),
                ("Unplugged", Chosen::Adjudicated),
            ],
        ),
        (
            "no llm",
            open(false, None),
            vec![
                ("Slowhand", Chosen::Adjudicated),
                ("Backless", Chosen::Adjudicated),
                ("Unplugged", Chosen::Adjudicated),
            ],
        ),
        ("awaiting the llm", open(true, None), vec![]),
        ("not found", KeyStatus::NotFound, vec![]),
        ("deferred", KeyStatus::Deferred { retry_after: 9 }, vec![]),
        ("stale", KeyStatus::Stale, vec![]),
    ];
    for (name, status, expected) in cases {
        let entry = entry(status, vec![], 1, 1);
        let got: Vec<_> = entry
            .plannable()
            .into_iter()
            .map(|(outcome, chosen)| {
                let canonical = outcome.canonical.as_ref().expect("canonical");
                (canonical.group.title.as_str(), chosen)
            })
            .collect();
        assert_eq!(got, expected, "{name}");
    }
}

#[tokio::test]
async fn changes_since_reports_each_changed_key_once_and_survives_reopen() {
    let state = Arc::new(MemoryScrubberState::new());
    let (mut writer, reader) = open(state.clone()).await.unwrap();
    assert_eq!(reader.changes_since(0), (vec![], 0));

    let horses = key("The Rolling Stones", "Wild Horses", "Sticky Fingers");
    let four = key("Wes Montgomery", "Four on Six", "Smokin' at the Half Note");
    let on = |key: &ScrobbleKey, at| ResolutionEvent {
        key: key.clone(),
        ..not_found(at, RESOLVER_VERSION)
    };
    writer
        .append(vec![on(&horses, 1), on(&four, 2), on(&horses, 3)])
        .await
        .unwrap();
    writer.append(vec![on(&four, 4)]).await.unwrap();

    assert_eq!(
        reader.changes_since(0),
        (vec![horses.clone(), four.clone()], 4)
    );
    assert_eq!(
        reader.changes_since(2),
        (vec![horses.clone(), four.clone()], 4)
    );
    assert_eq!(reader.changes_since(3), (vec![four.clone()], 4));
    assert_eq!(reader.changes_since(4), (vec![], 4));
    assert_eq!(
        reader.changes_since(9),
        (vec![], 9),
        "a cursor past the head belongs to another process's appends"
    );
    assert_eq!(
        reader.status_counts(),
        StatusCounts {
            not_found: 2,
            ..StatusCounts::default()
        }
    );

    let (_, reopened) = open(state).await.unwrap();
    assert_eq!(reopened.changes_since(3), (vec![four], 4));
    assert_eq!(reopened.get(&horses).map(|entry| entry.seq), Some(3));
}

fn researched(at: u64, resolver: u32, outcome: Option<Outcome>) -> ResolutionEvent {
    event(
        at,
        resolver,
        ResolutionEventKind::Researched {
            outcome,
            recording: Some(RecordingId("40ae4c06-3799-413b-b879-101dec39ac44".into())),
            release_group: None,
            confidence: Confidence::Medium,
            reason: "the Slowhand studio take".into(),
            by: "claude:opus".into(),
            raw: String::new(),
        },
    )
}

#[test]
fn research_settles_the_key_only_when_musicbrainz_confirmed_it() {
    let found = Outcome {
        proof: Proof::Researched {
            by: "claude:opus".into(),
        },
        ..outcome("40ae4c06", "Slowhand")
    };
    let note = |at, verified| ResearchNote {
        at,
        by: "claude:opus".into(),
        confidence: Confidence::Medium,
        reason: "the Slowhand studio take".into(),
        recording: Some(RecordingId("40ae4c06-3799-413b-b879-101dec39ac44".into())),
        release_group: None,
        verified,
    };
    let now = RESOLVER_VERSION;
    let cases = [
        (
            "confirmed",
            vec![not_found(10, now), researched(20, now, Some(found.clone()))],
            KeyEntry {
                research: Some(note(20, true)),
                ..entry(
                    KeyStatus::Settled {
                        outcome: found.clone(),
                    },
                    vec![],
                    2,
                    20,
                )
            },
        ),
        (
            "not confirmed",
            vec![not_found(10, now), researched(20, now, None)],
            KeyEntry {
                research: Some(note(20, false)),
                ..entry(KeyStatus::NotFound, vec![], 2, 10)
            },
        ),
        (
            "older resolver version",
            vec![not_found(10, now), researched(20, now - 1, Some(found))],
            entry(KeyStatus::NotFound, vec![], 2, 10),
        ),
    ];
    for (name, events, expected) in cases {
        let snapshot = fold(events);
        assert_eq!(snapshot.keys.get(&cocaine()), Some(&expected), "{name}");
        assert_eq!(snapshot.research_times, vec![20], "{name}");
        assert_eq!(snapshot.changes.last(), Some(&(2, cocaine())), "{name}");
    }
}

fn lennon(track: &str, album: &str) -> ScrobbleKey {
    key("John Lennon", track, album)
}

/// Settled on John Lennon/Plastic Ono Band, the release group `album` names.
fn on_plastic_ono_band(track: &str, album: &str) -> ResolutionEvent {
    let group = group("8be83468", "John Lennon/Plastic Ono Band", Vec::new());
    ResolutionEvent {
        key: lennon(track, album),
        at: 10,
        resolver: RESOLVER_VERSION,
        kind: ResolutionEventKind::Resolved {
            outcome: Outcome {
                master: vec![RecordingId(track.into())],
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

fn plastic_ono_band() -> NamingQuestion {
    NamingQuestion::album(
        "John Lennon",
        "Plastic Ono Band",
        "John Lennon/Plastic Ono Band",
    )
}

fn judged(at: u64, verdict: NamingVerdict, by: NamingJudge) -> ResolutionEvent {
    ResolutionEvent {
        key: lennon("Mother", "Plastic Ono Band"),
        at,
        resolver: RESOLVER_VERSION,
        kind: ResolutionEventKind::NamingJudged {
            question: plastic_ono_band(),
            verdict,
            reason: "the album is released as Plastic Ono Band".into(),
            by,
            raw: String::new(),
        },
    }
}

#[test]
fn a_naming_verdict_changes_every_key_on_the_album() {
    let llm = NamingJudge::Llm {
        model: "claude:opus".into(),
        prompt_version: 1,
    };
    let human = NamingJudge::Human {
        intent: Uuid::from_u128(7),
    };
    let reader = ResolutionReader::from_events(vec![
        on_plastic_ono_band("Mother", "Plastic Ono Band"),
        on_plastic_ono_band("Love", "Plastic Ono Band"),
        on_plastic_ono_band("Mother", "Plastic Ono Band (Remastered)"),
        on_plastic_ono_band("Love", "John Lennon/Plastic Ono Band"),
        resolved(10, RESOLVER_VERSION),
        judged(20, NamingVerdict::MusicBrainz, llm),
        judged(30, NamingVerdict::Keep, human.clone()),
    ]);

    assert_eq!(
        reader.changes_since(5),
        (
            vec![
                lennon("Mother", "Plastic Ono Band"),
                lennon("Love", "Plastic Ono Band"),
                lennon("Mother", "Plastic Ono Band (Remastered)"),
            ],
            7
        )
    );
    let snapshot = reader.snapshot();
    assert_eq!(
        snapshot.naming.get(&plastic_ono_band()),
        Some(&NamingJudgement {
            verdict: NamingVerdict::Keep,
            reason: "the album is released as Plastic Ono Band".into(),
            by: human,
            at: 30,
        })
    );
    assert_eq!(snapshot.adjudication_times, vec![20]);
}

#[test]
fn only_proven_settlements_confirm_rewrite_rules() {
    let isrc = |len_delta_ms| Proof::SharedIsrc {
        isrc: "NLF057790024".into(),
        len_delta_ms,
    };
    let cases = [
        ("same recording", Proof::SameRecording, Some(true)),
        ("shared ISRC, 2.1 s apart", isrc(2_107), Some(true)),
        ("shared ISRC, 5 s apart", isrc(5_000), Some(true)),
        ("shared ISRC, 5.001 s apart", isrc(5_001), None),
        ("same work", Proof::SameWork, None),
        ("title only", Proof::TitleOnly, None),
        (
            "researched",
            Proof::Researched {
                by: "claude:opus".into(),
            },
            None,
        ),
    ];
    for (name, proof, expected) in cases {
        let settled = Outcome {
            proof,
            ..outcome("40ae4c06", "Slowhand")
        };
        let reader = ResolutionReader::from_events(vec![event(
            20,
            RESOLVER_VERSION,
            ResolutionEventKind::Resolved { outcome: settled },
        )]);
        assert_eq!(
            reader.settled_agrees(&cocaine(), Some("Cocaine"), Some("Slowhand")),
            expected,
            "{name}"
        );
    }

    let researched_outcome = Outcome {
        proof: Proof::Researched {
            by: "claude:opus".into(),
        },
        ..outcome("40ae4c06", "Slowhand")
    };
    let reader = ResolutionReader::from_events(vec![researched(
        20,
        RESOLVER_VERSION,
        Some(researched_outcome),
    )]);
    assert_eq!(
        reader.settled_agrees(&cocaine(), Some("Cocaine"), Some("Slowhand")),
        None,
        "settled by a research answer"
    );
}

impl ResolutionReader {
    pub(crate) fn from_events(events: Vec<ResolutionEvent>) -> Self {
        Self {
            snapshot: Arc::new(RwLock::new(Arc::new(fold(events)))),
        }
    }
}
