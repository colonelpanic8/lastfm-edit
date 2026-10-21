use super::*;
use crate::identity::adjudicate::Confidence;
use crate::identity::book::fold;
use crate::identity::canonical::{Canonical, Rung};
use crate::identity::llm::scripted::ScriptedBackend;
use crate::identity::llm::ChoiceRequest;
use crate::identity::llm::{RawChoice, RawResearch};
use crate::identity::mb::{PrimaryType, ReleaseGroup, SecondaryType};
use crate::identity::mb::{RecordingId, ReleaseGroupId};
use crate::identity::naming::{NAMING_PROMPT, TRACK_NAMING_PROMPT};
use crate::identity::outcome::{MoveMode, Outcome};
use crate::identity::testing::{credit, group, FixtureMb, WALTZ_FOR_DEBBY_TAKE_2};
use crate::queue::{QueueEvent, QueueEventKind};
use crate::rewrite::create_no_op_edit;
use crate::state::MemoryScrubberState;
use crate::subject::Subject;
use scrobble_store::{MemoryStorage, Provenanced, RecordSource, ScrobbleId, ScrobbleRecord};
use uuid::Uuid;

struct Harness {
    lane: IdentityLane,
    state: Arc<MemoryScrubberState>,
}

/// `(artist, track, album, plays)` per key.
async fn harness(
    mb: FixtureMb,
    library: &[(&str, &str, Option<&str>, u32)],
    adjudicator: Option<Adjudicator>,
) -> Harness {
    let store = Arc::new(MemoryStorage::new());
    let mut records = Vec::new();
    for (artist, track, album, plays) in library {
        for _ in 0..*plays {
            let uts = 1_600_000_000 + records.len() as u64;
            records.push(ScrobbleRecord {
                id: ScrobbleId::new(uts, artist, track),
                uts,
                artist: artist.to_string(),
                track: track.to_string(),
                album: album.map(str::to_string),
                album_artist: Provenanced::Unknown,
                source: RecordSource::Api,
                fetched_at: 1,
                deleted: false,
                v: 1,
            });
        }
    }
    store.append_scrobbles(&records).await.unwrap();
    let state = Arc::new(MemoryScrubberState::new());
    let (writer, _) = crate::identity::book::open(state.clone()).await.unwrap();
    let lane = IdentityLane::new(store, state.clone(), writer, Arc::new(mb), adjudicator);
    Harness { lane, state }
}

/// `(track, album, event)` per logged event.
async fn log(state: &MemoryScrubberState) -> Vec<(String, Option<String>, &'static str)> {
    state
        .load_resolution_events()
        .await
        .unwrap()
        .into_iter()
        .map(|e| (e.key.track, e.key.album, event_name(&e.kind)))
        .collect()
}

fn event_name(kind: &ResolutionEventKind) -> &'static str {
    match kind {
        ResolutionEventKind::Ambiguous { .. } => "ambiguous",
        other => status_label(other),
    }
}

fn row(track: &str, album: &str, event: &'static str) -> (String, Option<String>, &'static str) {
    (track.into(), Some(album.into()), event)
}

#[tokio::test]
async fn round_resolves_each_key_once_trigger_first_and_then_rests() {
    let mb = FixtureMb::default().cocaine().wild_horses().four_on_six();
    let mut h = harness(
        mb,
        &[
            (
                "Wes Montgomery",
                "Four on Six",
                Some("Smokin' at the Half Note"),
                5,
            ),
            ("Eric Clapton", "Cocaine", Some("The Cream of Clapton"), 3),
            (
                "Eric Clapton",
                "Cocaine",
                Some("Slowhand 35th Anniversary (Super Deluxe)"),
                1,
            ),
            (
                "The Rolling Stones",
                "Wild Horses - 2009 Re-Mastered Digital Version",
                Some("Sticky Fingers"),
                1,
            ),
        ],
        None,
    )
    .await;

    let first = h.lane.run_round(10).await.unwrap();
    assert_eq!(
        first,
        LaneReport {
            keys_attempted: 4,
            resolved: 4,
            mb_requests: 11,
            ..LaneReport::default()
        }
    );
    let expected = vec![
        row("Cocaine", "The Cream of Clapton", "resolved"),
        row(
            "Cocaine",
            "Slowhand 35th Anniversary (Super Deluxe)",
            "resolved",
        ),
        row(
            "Wild Horses - 2009 Re-Mastered Digital Version",
            "Sticky Fingers",
            "resolved",
        ),
        row("Four on Six", "Smokin' at the Half Note", "resolved"),
    ];
    assert_eq!(log(&h.state).await, expected);

    let second = h.lane.run_round(10).await.unwrap();
    assert_eq!(second, LaneReport::default());
    assert_eq!(log(&h.state).await, expected);
}

#[tokio::test]
async fn round_stops_at_its_key_budget() {
    let mb = FixtureMb::default().cocaine().four_on_six();
    let mut h = harness(
        mb,
        &[
            (
                "Wes Montgomery",
                "Four on Six",
                Some("Smokin' at the Half Note"),
                5,
            ),
            ("Eric Clapton", "Cocaine", Some("The Cream of Clapton"), 3),
        ],
        None,
    )
    .await;
    let report = h.lane.run_round(1).await.unwrap();
    assert_eq!(
        (report.keys_attempted, report.ended),
        (1, LaneEnded::BudgetExhausted)
    );
    let report = h.lane.run_round(1).await.unwrap();
    assert_eq!(
        (report.keys_attempted, report.ended),
        (1, LaneEnded::WorklistEmpty)
    );
}

#[tokio::test]
async fn keys_with_an_open_intent_resolve_before_the_rest() {
    let mb = FixtureMb::default().wild_horses().four_on_six();
    let mut h = harness(
        mb,
        &[
            (
                "The Rolling Stones",
                "Wild Horses - 2009 Re-Mastered Digital Version",
                Some("Sticky Fingers"),
                9,
            ),
            (
                "Wes Montgomery",
                "Four on Six",
                Some("Smokin' at the Half Note"),
                1,
            ),
        ],
        None,
    )
    .await;
    let subject = Subject {
        artist: "Wes Montgomery".into(),
        track: "Four on Six".into(),
        album: Some("Smokin' at the Half Note".into()),
        album_artist: None,
    };
    let mut edit = create_no_op_edit(&subject.representative_track(0, None));
    edit.album_name = Some("Smokin' at the Half Note (Live)".into());
    h.state
        .append_queue_events(&[QueueEvent {
            id: Uuid::new_v4(),
            at: 1,
            kind: QueueEventKind::Created {
                subject,
                proposed: Box::new(edit),
                provider: "rewrite_rules".into(),
                requires_approval: true,
                rationale: None,
                sources: Default::default(),
            },
        }])
        .await
        .unwrap();

    h.lane.run_round(1).await.unwrap();
    assert_eq!(
        log(&h.state).await,
        vec![row("Four on Six", "Smokin' at the Half Note", "resolved")]
    );
}

#[tokio::test]
async fn human_rejection_of_an_identity_intent_is_recorded_once() {
    let mut h = harness(FixtureMb::default(), &[], None).await;
    let subject = Subject {
        artist: "Eric Clapton".into(),
        track: "Cocaine".into(),
        album: Some("The Cream of Clapton".into()),
        album_artist: None,
    };
    let mut edit = create_no_op_edit(&subject.representative_track(0, None));
    edit.album_name = Some("Slowhand".into());
    edit.album_artist_name = Some("Eric Clapton".into());
    let id = Uuid::new_v4();
    h.state
        .append_queue_events(&[
            QueueEvent {
                id,
                at: 1,
                kind: QueueEventKind::Created {
                    subject: subject.clone(),
                    proposed: Box::new(edit),
                    provider: IDENTITY_PROVIDER.into(),
                    requires_approval: true,
                    rationale: None,
                    sources: Default::default(),
                },
            },
            QueueEvent {
                id,
                at: 2,
                kind: QueueEventKind::Rejected {
                    dismiss_subject: false,
                },
            },
        ])
        .await
        .unwrap();

    assert_eq!(h.lane.run_round(10).await.unwrap().rejections_recorded, 1);
    assert_eq!(h.lane.run_round(10).await.unwrap().rejections_recorded, 0);
    let events = h.state.load_resolution_events().await.unwrap();
    let kinds: Vec<_> = events.into_iter().map(|e| (e.key, e.kind)).collect();
    assert_eq!(
        kinds,
        vec![(
            subject.key(),
            ResolutionEventKind::HumanRejected {
                intent: id,
                target: Target {
                    title: "Cocaine".into(),
                    album: Some("Slowhand".into()),
                    album_artist: Some("Eric Clapton".into()),
                },
            },
        )]
    );
}

#[tokio::test]
async fn three_rate_limited_keys_in_a_row_defer_the_round() {
    let mb = FixtureMb {
        rate_limited: true,
        ..FixtureMb::default()
    };
    let mut h = harness(
        mb,
        &[
            ("A", "One", Some("X"), 5),
            ("A", "Two", Some("X"), 4),
            ("A", "Three", Some("X"), 3),
            ("A", "Four", Some("X"), 2),
        ],
        None,
    )
    .await;
    let report = h.lane.run_round(10).await.unwrap();
    assert_eq!(
        report,
        LaneReport {
            keys_attempted: 3,
            deferred: 3,
            mb_requests: 3,
            ended: LaneEnded::Deferred,
            ..LaneReport::default()
        }
    );
    assert_eq!(
        log(&h.state).await,
        vec![
            row("One", "X", "deferred"),
            row("Two", "X", "deferred"),
            row("Three", "X", "deferred"),
        ]
    );
    let entry = h.lane.reader().get(&ScrobbleKey {
        artist: "A".into(),
        track: "One".into(),
        album: Some("X".into()),
    });
    let Some(KeyStatus::Deferred { retry_after }) = entry.map(|e| e.status) else {
        panic!("expected a deferred key");
    };
    assert!(retry_after >= unix_now() + 29);
}

#[tokio::test]
async fn ambiguous_cases_are_adjudicated_within_the_daily_budget() {
    let backend = Arc::new(ScriptedBackend::default());
    backend.answers.lock().unwrap().push(RawChoice {
        choice: "A".into(),
        confidence: "high".into(),
        reason: "the 2009 mix is the Sticky Fingers master".into(),
        text: r#"{"choice":"A"}"#.into(),
    });
    let adjudicator = Adjudicator::new(Box::new(backend.clone()), 1);
    let mut h = harness(
        FixtureMb::default().wild_horses_on_honk(),
        &[
            ("The Rolling Stones", "Wild Horses", Some("Honk"), 2),
            (
                "The Rolling Stones",
                "Wild Horses",
                Some("Honk (Deluxe)"),
                1,
            ),
        ],
        Some(adjudicator),
    )
    .await;

    let report = h.lane.run_round(10).await.unwrap();
    assert_eq!(
        (report.keys_attempted, report.ambiguous, report.adjudicated),
        (2, 2, 1)
    );
    assert_eq!(backend.requests.lock().unwrap().len(), 1);

    let events = h.state.load_resolution_events().await.unwrap();
    let summary: Vec<_> = events
        .iter()
        .map(|e| match &e.kind {
            ResolutionEventKind::Ambiguous { case, awaiting_llm } => (
                e.key.album.clone(),
                format!(
                    "ambiguous awaiting_llm={awaiting_llm} siblings={}",
                    case.siblings.len()
                ),
            ),
            ResolutionEventKind::Adjudicated { verdict, by, .. } => (
                e.key.album.clone(),
                format!("adjudicated {:?} by {by}", verdict.choice),
            ),
            other => (e.key.album.clone(), status_label(other).to_string()),
        })
        .collect();
    assert_eq!(
        summary,
        vec![
            (
                Some("Honk".into()),
                "ambiguous awaiting_llm=false siblings=1".into()
            ),
            (
                Some("Honk".into()),
                "adjudicated Option(OptionId('A')) by scripted".into()
            ),
            (
                Some("Honk (Deluxe)".into()),
                "ambiguous awaiting_llm=true siblings=1".into()
            ),
        ]
    );
    let entry = h.lane.reader().get(&events[0].key).unwrap();
    let KeyStatus::Open { verdict, .. } = entry.status else {
        panic!("expected an open case");
    };
    assert_eq!(
        verdict,
        Some(Verdict {
            choice: Choice::Option(crate::identity::OptionId('A')),
            confidence: Confidence::High,
            reason: "the 2009 mix is the Sticky Fingers master".into(),
            invalid: false,
        })
    );

    let report = h.lane.run_round(10).await.unwrap();
    assert_eq!(report.keys_attempted, 0);
    assert_eq!(backend.requests.lock().unwrap().len(), 1);
}

/// Settled on `group`, the release group the key's album names.
fn settled_on(key: ScrobbleKey, group: ReleaseGroup) -> ResolutionEvent {
    ResolutionEvent {
        at: unix_now(),
        resolver: RESOLVER_VERSION,
        kind: ResolutionEventKind::Resolved {
            outcome: Outcome {
                master: vec![RecordingId(key.track.clone())],
                title: key.track.clone(),
                scrobbled_group: Some(group.clone()),
                canonical: Some(Canonical {
                    group,
                    rung: Rung::PlainAlbum,
                    single_first: false,
                }),
                proof: Proof::SameRecording,
            },
        },
        key,
    }
}

fn lennon(track: &str) -> ScrobbleKey {
    ScrobbleKey {
        artist: "John Lennon".into(),
        track: track.into(),
        album: Some("Plastic Ono Band".into()),
    }
}

fn sinister_minister() -> ScrobbleKey {
    ScrobbleKey {
        artist: "Béla Fleck and the Flecktones".into(),
        track: "The Sinister Minister".into(),
        album: Some("Bela Fleck And The Flecktones".into()),
    }
}

fn plastic_ono_band() -> ReleaseGroup {
    group(
        "8be83468-4917-390d-82b6-d890f058cd22",
        "John Lennon/Plastic Ono Band",
        PrimaryType::Album,
        &[],
        1970,
        credit(&[("4d5447d7-c61c-4120-ba1b-d7f471d385b9", "John Lennon", "")]),
    )
}

fn flecktones() -> ReleaseGroup {
    group(
        "35a0a631-c711-3818-b726-b0d63b2bf602",
        "Béla Fleck and the Flecktones",
        PrimaryType::Album,
        &[],
        1990,
        credit(&[(
            "830dce6f-c9ff-4ebe-944b-2ea2ab22fcdf",
            "Béla Fleck and the Flecktones",
            "",
        )]),
    )
}

#[tokio::test]
async fn naming_questions_are_asked_once_per_album_within_the_daily_budget() {
    let backend = Arc::new(ScriptedBackend::default());
    backend.answers.lock().unwrap().push(RawChoice {
        choice: "keep".into(),
        confidence: "high".into(),
        reason: "Released and streamed as Plastic Ono Band.".into(),
        text: r#"{"choice":"keep"}"#.into(),
    });
    let mut h = harness(
        FixtureMb::default(),
        &[
            ("John Lennon", "Mother", Some("Plastic Ono Band"), 5),
            ("John Lennon", "Love", Some("Plastic Ono Band"), 3),
            (
                "Béla Fleck and the Flecktones",
                "The Sinister Minister",
                Some("Bela Fleck And The Flecktones"),
                6,
            ),
        ],
        Some(Adjudicator::new(Box::new(backend.clone()), 1)),
    )
    .await;
    h.lane
        .writer
        .append(vec![
            settled_on(lennon("Mother"), plastic_ono_band()),
            settled_on(lennon("Love"), plastic_ono_band()),
            settled_on(sinister_minister(), flecktones()),
        ])
        .await
        .unwrap();

    let report = h.lane.run_round(10).await.unwrap();

    assert_eq!(
        (report.keys_attempted, report.naming_judged),
        (0, 1),
        "{report:?}"
    );
    assert_eq!(
            backend.requests.lock().unwrap().as_slice(),
            [ChoiceRequest {
                system: NAMING_PROMPT.into(),
                user: r#"Album
  artist: "John Lennon"
  the user's title: "Plastic Ono Band"
  MusicBrainz title: "John Lennon/Plastic Ono Band"
  MusicBrainz release group: John Lennon/Plastic Ono Band (Album, 1970), credited to "John Lennon"
  the user's album strings: "Plastic Ono Band"
  tracks played: "Mother", "Love"
  plays: 8

Answer keep to use "Plastic Ono Band", musicbrainz to use "John Lennon/Plastic Ono Band", or unsure."#
                    .into(),
                allowed: vec!["keep".into(), "musicbrainz".into(), "unsure".into()],
            }]
        );
    let reader = h.lane.reader();
    assert_eq!(
        reader.changes_since(3),
        (vec![lennon("Mother"), lennon("Love")], 4)
    );
    let question = NamingQuestion::album(
        "John Lennon",
        "Plastic Ono Band",
        "John Lennon/Plastic Ono Band",
    );
    let judgement = reader.snapshot().naming.get(&question).cloned().unwrap();
    assert_eq!(
        (judgement.verdict, judgement.reason, judgement.by),
        (
            NamingVerdict::Keep,
            "Released and streamed as Plastic Ono Band.".to_string(),
            NamingJudge::Llm {
                model: "scripted".into(),
                prompt_version: 1,
            },
        )
    );

    let report = h.lane.run_round(10).await.unwrap();
    assert_eq!(report.naming_judged, 0, "the daily budget is spent");
    assert_eq!(backend.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn no_album_naming_question_is_asked_when_edition_fixes_are_off() {
    let backend = Arc::new(ScriptedBackend::default());
    let h = harness(
        FixtureMb::default(),
        &[("John Lennon", "Mother", Some("Plastic Ono Band"), 5)],
        Some(Adjudicator::new(Box::new(backend.clone()), 10)),
    )
    .await;
    let mut lane = h.lane.with_moves(MovePolicy {
        editions: MoveMode::Off,
        ..MovePolicy::default()
    });
    lane.writer
        .append(vec![settled_on(lennon("Mother"), plastic_ono_band())])
        .await
        .unwrap();

    let report = lane.run_round(10).await.unwrap();

    assert_eq!(report.naming_judged, 0);
    assert_eq!(backend.requests.lock().unwrap().len(), 0);
}

#[tokio::test]
async fn rejecting_a_retitle_keeps_the_title_for_the_whole_album() {
    let mut h = harness(
        FixtureMb::default(),
        &[
            ("John Lennon", "Mother", Some("Plastic Ono Band"), 5),
            ("John Lennon", "Love", Some("Plastic Ono Band"), 3),
        ],
        None,
    )
    .await;
    let question = NamingQuestion::album(
        "John Lennon",
        "Plastic Ono Band",
        "John Lennon/Plastic Ono Band",
    );
    h.lane
        .writer
        .append(vec![
            settled_on(lennon("Mother"), plastic_ono_band()),
            settled_on(lennon("Love"), plastic_ono_band()),
            ResolutionEvent {
                key: lennon("Mother"),
                at: unix_now(),
                resolver: RESOLVER_VERSION,
                kind: ResolutionEventKind::NamingJudged {
                    question: question.clone(),
                    verdict: NamingVerdict::MusicBrainz,
                    reason: "The official title.".into(),
                    by: NamingJudge::Llm {
                        model: "scripted".into(),
                        prompt_version: 1,
                    },
                    raw: String::new(),
                },
            },
        ])
        .await
        .unwrap();
    let provider = crate::identity::IdentityProvider::new(
        h.lane.reader(),
        crate::identity::MovePolicy::default(),
        crate::identity::RetitleJudge::Llm,
    );
    let love = lastfm_edit::Track {
        name: "Love".into(),
        artist: "John Lennon".into(),
        playcount: 3,
        timestamp: None,
        album: Some("Plastic Ono Band".into()),
        album_artist: None,
    };
    let proposed_albums = || async {
        use crate::provider::{ScrubActionProvider, ScrubActionSuggestion};
        provider
            .analyze_tracks(std::slice::from_ref(&love), None, None)
            .await
            .unwrap()
            .into_iter()
            .flat_map(|(_, suggestions)| suggestions)
            .filter_map(|suggestion| match suggestion.suggestion {
                ScrubActionSuggestion::Edit(edit) => edit.album_name,
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        proposed_albums().await,
        vec!["John Lennon/Plastic Ono Band".to_string()]
    );

    let subject = Subject {
        artist: "John Lennon".into(),
        track: "Mother".into(),
        album: Some("Plastic Ono Band".into()),
        album_artist: None,
    };
    let mut edit = create_no_op_edit(&subject.representative_track(0, None));
    edit.album_name = Some("John Lennon/Plastic Ono Band".into());
    let id = Uuid::new_v4();
    h.state
        .append_queue_events(&[
            QueueEvent {
                id,
                at: 1,
                kind: QueueEventKind::Created {
                    subject,
                    proposed: Box::new(edit),
                    provider: IDENTITY_PROVIDER.into(),
                    requires_approval: false,
                    rationale: None,
                    sources: Default::default(),
                },
            },
            QueueEvent {
                id,
                at: 2,
                kind: QueueEventKind::Rejected {
                    dismiss_subject: false,
                },
            },
        ])
        .await
        .unwrap();

    h.lane.run_round(10).await.unwrap();

    assert_eq!(
        log(&h.state).await[3..],
        [
            row("Mother", "Plastic Ono Band", "human_rejected"),
            row("Mother", "Plastic Ono Band", "naming_judged"),
        ]
    );
    let judgement = h.lane.reader().snapshot().naming[&question].clone();
    assert_eq!(
        (judgement.verdict, judgement.by),
        (NamingVerdict::Keep, NamingJudge::Human { intent: id })
    );
    assert_eq!(proposed_albums().await, Vec::<String>::new());
}

fn mack_the_knife() -> ScrobbleKey {
    ScrobbleKey {
        artist: "Ella Fitzgerald".into(),
        track: "Mack the Knife [Live]".into(),
        album: Some("Ella in Berlin: Mack the Knife".into()),
    }
}

/// Two live recordings; the LLM picked "Mack the Knife", which drops the live marker.
fn picked_mack_the_knife() -> Vec<ResolutionEvent> {
    let berlin = group(
        "2e8d0d4c-1b0b-3a0e-8d55-0e4a5c2b4c7b",
        "Ella in Berlin: Mack the Knife",
        PrimaryType::Album,
        &[SecondaryType::Live],
        1960,
        credit(&[(
            "54799c0e-eb45-4eea-996d-c4d71a63c499",
            "Ella Fitzgerald",
            "",
        )]),
    );
    let recording = |title: &str| Outcome {
        master: vec![RecordingId(title.into())],
        title: title.into(),
        scrobbled_group: Some(berlin.clone()),
        canonical: Some(Canonical {
            group: berlin.clone(),
            rung: Rung::PlainAlbum,
            single_first: false,
        }),
        proof: Proof::SameRecording,
    };
    let case = Case::new(
        mack_the_knife(),
        4,
        Vec::new(),
        vec![
            (recording("Mack the Knife"), Default::default()),
            (
                recording("Mack the Knife (live, 1960-02-13: Deutschlandhalle, Berlin)"),
                Default::default(),
            ),
        ],
    );
    let event = |kind| ResolutionEvent {
        key: mack_the_knife(),
        at: unix_now(),
        resolver: RESOLVER_VERSION,
        kind,
    };
    vec![
        event(ResolutionEventKind::Adjudicated {
            case_hash: crate::identity::adjudicate::case_hash(&case),
            verdict: Verdict {
                choice: Choice::Option(crate::identity::adjudicate::OptionId('A')),
                confidence: Confidence::High,
                reason: "The Berlin recording.".into(),
                invalid: false,
            },
            by: "scripted".into(),
            prompt_version: PROMPT_VERSION,
            raw: String::new(),
        }),
        event(ResolutionEventKind::Ambiguous {
            case,
            awaiting_llm: false,
        }),
    ]
    .into_iter()
    .rev()
    .collect()
}

#[tokio::test]
async fn the_picked_options_track_retitle_is_asked_once() {
    let backend = Arc::new(ScriptedBackend::default());
    backend.answers.lock().unwrap().push(RawChoice {
        choice: "keep".into(),
        confidence: "high".into(),
        reason: "MusicBrainz drops the live marker.".into(),
        text: r#"{"choice":"keep"}"#.into(),
    });
    let mut h = harness(
        FixtureMb::default(),
        &[(
            "Ella Fitzgerald",
            "Mack the Knife [Live]",
            Some("Ella in Berlin: Mack the Knife"),
            4,
        )],
        Some(Adjudicator::new(Box::new(backend.clone()), 10)),
    )
    .await;
    h.lane.writer.append(picked_mack_the_knife()).await.unwrap();

    let report = h.lane.run_round(10).await.unwrap();

    assert_eq!(report.naming_judged, 1, "{report:?}");
    assert_eq!(
            backend.requests.lock().unwrap().as_slice(),
            [ChoiceRequest {
                system: TRACK_NAMING_PROMPT.into(),
                user: r#"Track
  artist: "Ella Fitzgerald"
  the user's title: "Mack the Knife [Live]"
  MusicBrainz title: "Mack the Knife"
  MusicBrainz release group: Ella in Berlin: Mack the Knife (Album + Live, 1960), credited to "Ella Fitzgerald"
  albums the user played it on: "Ella in Berlin: Mack the Knife"
  plays: 4

Answer keep to use "Mack the Knife [Live]", musicbrainz to use "Mack the Knife", or unsure."#
                    .into(),
                allowed: vec!["keep".into(), "musicbrainz".into(), "unsure".into()],
            }]
        );
    let reader = h.lane.reader();
    assert_eq!(reader.changes_since(2), (vec![mack_the_knife()], 3));
    let question =
        NamingQuestion::track("Ella Fitzgerald", "Mack the Knife [Live]", "Mack the Knife");
    let judgement = reader.snapshot().naming[&question].clone();
    assert_eq!(
        (judgement.verdict, judgement.reason),
        (
            NamingVerdict::Keep,
            "MusicBrainz drops the live marker.".to_string()
        )
    );

    h.lane.run_round(10).await.unwrap();
    assert_eq!(backend.requests.lock().unwrap().len(), 1, "answered once");
}

#[tokio::test]
async fn rejecting_a_track_retitle_keeps_the_users_title() {
    let mut h = harness(
        FixtureMb::default(),
        &[(
            "Ella Fitzgerald",
            "Mack the Knife [Live]",
            Some("Ella in Berlin: Mack the Knife"),
            4,
        )],
        None,
    )
    .await;
    h.lane.writer.append(picked_mack_the_knife()).await.unwrap();
    let subject = Subject {
        artist: "Ella Fitzgerald".into(),
        track: "Mack the Knife [Live]".into(),
        album: Some("Ella in Berlin: Mack the Knife".into()),
        album_artist: None,
    };
    let mut edit = create_no_op_edit(&subject.representative_track(0, None));
    edit.track_name = Some("Mack the Knife".into());
    let id = Uuid::new_v4();
    h.state
        .append_queue_events(&[
            QueueEvent {
                id,
                at: 1,
                kind: QueueEventKind::Created {
                    subject,
                    proposed: Box::new(edit),
                    provider: IDENTITY_PROVIDER.into(),
                    requires_approval: true,
                    rationale: None,
                    sources: Default::default(),
                },
            },
            QueueEvent {
                id,
                at: 2,
                kind: QueueEventKind::Rejected {
                    dismiss_subject: false,
                },
            },
        ])
        .await
        .unwrap();

    h.lane.run_round(10).await.unwrap();

    assert_eq!(
        log(&h.state).await[2..],
        [
            row(
                "Mack the Knife [Live]",
                "Ella in Berlin: Mack the Knife",
                "human_rejected"
            ),
            row(
                "Mack the Knife [Live]",
                "Ella in Berlin: Mack the Knife",
                "naming_judged"
            ),
        ]
    );
    let question =
        NamingQuestion::track("Ella Fitzgerald", "Mack the Knife [Live]", "Mack the Knife");
    let judgement = h.lane.reader().snapshot().naming[&question].clone();
    assert_eq!(
        (judgement.verdict, judgement.by),
        (NamingVerdict::Keep, NamingJudge::Human { intent: id })
    );
}

fn research_answer(recording: &str, release_group: &str) -> RawResearch {
    RawResearch {
        recording: Some(RecordingId(recording.into())),
        release_group: Some(ReleaseGroupId(release_group.into())),
        confidence: "medium".into(),
        reason: "take 2 is the master take".into(),
        text: String::new(),
    }
}

fn researching(
    mut h: Harness,
    answers: Vec<RawResearch>,
    daily_budget: u32,
) -> (Harness, Arc<ScriptedBackend>) {
    let backend = Arc::new(ScriptedBackend::default());
    *backend.research_answers.lock().unwrap() = answers;
    h.lane = h.lane.with_researcher(Some(Researcher::new(
        Box::new(backend.clone()),
        daily_budget,
    )));
    (h, backend)
}

#[tokio::test]
async fn research_takes_not_found_keys_most_played_first_within_the_daily_budget() {
    let h = harness(
        FixtureMb::default().waltz_for_debby(),
        &[
            ("A", "One", Some("X"), 5),
            ("Bill Evans", "Waltz for Debby", Some("Waltz For Debby"), 9),
            ("A", "Two", Some("X"), 3),
        ],
        None,
    )
    .await;
    let (mut h, backend) = researching(
        h,
        vec![
            research_answer(
                WALTZ_FOR_DEBBY_TAKE_2,
                "392e7fcf-5ae9-37f6-b8c9-57901cfbee1f",
            ),
            research_answer(
                "1d0b6fbe-3f4e-4e43-9b52-0d3a2c1e5f77",
                "392e7fcf-5ae9-37f6-b8c9-57901cfbee1f",
            ),
        ],
        2,
    );

    let report = h.lane.run_round(10).await.unwrap();

    assert_eq!((report.not_found, report.researched), (3, 2), "{report:?}");
    assert_eq!(
        log(&h.state).await,
        vec![
            row("Waltz for Debby", "Waltz For Debby", "not_found"),
            row("One", "X", "not_found"),
            row("Two", "X", "not_found"),
            row("Waltz for Debby", "Waltz For Debby", "researched"),
            row("One", "X", "research_unverified"),
        ]
    );
    let evans = h
        .lane
        .reader()
        .get(&ScrobbleKey {
            artist: "Bill Evans".into(),
            track: "Waltz for Debby".into(),
            album: Some("Waltz For Debby".into()),
        })
        .unwrap();
    let KeyStatus::Settled { outcome } = evans.status else {
        panic!("expected a settled key, got {:?}", evans.status);
    };
    assert_eq!(
        outcome.proof,
        Proof::Researched {
            by: "scripted".into()
        }
    );

    let report = h.lane.run_round(10).await.unwrap();
    assert_eq!(report.researched, 0);
    assert_eq!(backend.research_requests.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn research_failure_ends_research_for_the_round_but_not_resolution() {
    let h = harness(
        FixtureMb::default().cocaine(),
        &[
            ("A", "One", Some("X"), 5),
            ("A", "Two", Some("X"), 4),
            ("Eric Clapton", "Cocaine", Some("The Cream of Clapton"), 3),
        ],
        None,
    )
    .await;
    let (mut h, backend) = researching(h, Vec::new(), 20);

    let report = h.lane.run_round(10).await.unwrap();

    assert_eq!(
        (report.resolved, report.not_found, report.researched),
        (1, 2, 0)
    );
    assert_eq!(backend.research_requests.lock().unwrap().len(), 1);

    backend
        .research_answers
        .lock()
        .unwrap()
        .push(research_answer(
            "1d0b6fbe-3f4e-4e43-9b52-0d3a2c1e5f77",
            "392e7fcf-5ae9-37f6-b8c9-57901cfbee1f",
        ));
    let report = h.lane.run_round(10).await.unwrap();
    assert_eq!((report.keys_attempted, report.researched), (0, 1));
    assert_eq!(backend.research_requests.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn research_is_recorded_even_when_musicbrainz_cannot_verify_it() {
    let mb = FixtureMb {
        rate_limited: true,
        ..FixtureMb::default()
    };
    let h = harness(
        mb,
        &[("A", "One", Some("X"), 5), ("A", "Two", Some("X"), 3)],
        None,
    )
    .await;
    let (mut h, backend) = researching(
        h,
        vec![
            research_answer(
                "1d0b6fbe-3f4e-4e43-9b52-0d3a2c1e5f77",
                "392e7fcf-5ae9-37f6-b8c9-57901cfbee1f",
            ),
            research_answer(
                "8a3a8a4e-2c39-4d2f-9f0b-3e3c0c7c1a11",
                "392e7fcf-5ae9-37f6-b8c9-57901cfbee1f",
            ),
        ],
        20,
    );
    let key = |track: &str| ScrobbleKey {
        artist: "A".into(),
        track: track.into(),
        album: Some("X".into()),
    };
    let not_found = |track: &str| ResolutionEvent {
        key: key(track),
        at: unix_now(),
        resolver: RESOLVER_VERSION,
        kind: ResolutionEventKind::NotFound {
            queries: Vec::new(),
        },
    };
    h.lane
        .writer
        .append(vec![not_found("One"), not_found("Two")])
        .await
        .unwrap();

    let report = h.lane.run_round(10).await.unwrap();

    assert_eq!((report.keys_attempted, report.researched), (0, 1));
    let note = h.lane.reader().get(&key("One")).unwrap().research.unwrap();
    assert_eq!(
        (note.recording, note.confidence, note.verified),
        (
            Some(RecordingId("1d0b6fbe-3f4e-4e43-9b52-0d3a2c1e5f77".into())),
            Confidence::Medium,
            false
        )
    );
    assert_eq!(h.lane.reader().snapshot().research_times.len(), 1);

    let report = h.lane.run_round(10).await.unwrap();

    assert_eq!(report.researched, 1);
    assert_eq!(
        log(&h.state).await,
        vec![
            row("One", "X", "not_found"),
            row("Two", "X", "not_found"),
            row("One", "X", "research_unverified"),
            row("Two", "X", "research_unverified"),
        ]
    );
    assert_eq!(backend.research_requests.lock().unwrap().len(), 2);
}

#[test]
fn research_candidates_skip_keys_researched_in_the_last_90_days() {
    let now = 200 * DAY_SECS;
    let key = |track: &str| ScrobbleKey {
        artist: "A".into(),
        track: track.into(),
        album: Some("X".into()),
    };
    let event = |track: &str, at: u64, kind: ResolutionEventKind| ResolutionEvent {
        key: key(track),
        at,
        resolver: RESOLVER_VERSION,
        kind,
    };
    let not_found = || ResolutionEventKind::NotFound {
        queries: Vec::new(),
    };
    let unconfirmed = || ResolutionEventKind::Researched {
        outcome: None,
        recording: None,
        release_group: None,
        confidence: Confidence::Low,
        reason: String::new(),
        by: "scripted".into(),
        raw: String::new(),
    };
    let snapshot = fold(vec![
        event("Recent", DAY_SECS, not_found()),
        event("Recent", now - 89 * DAY_SECS, unconfirmed()),
        event("Old", DAY_SECS, not_found()),
        event("Old", now - 91 * DAY_SECS, unconfirmed()),
        event("Never", DAY_SECS, not_found()),
    ]);
    let library = Library {
        plays: BTreeMap::from([(key("Recent"), 9), (key("Old"), 2), (key("Never"), 4)]),
    };

    let tracks: Vec<String> = research_cases(&library, &snapshot, now)
        .into_iter()
        .map(|case| case.key.track)
        .collect();

    assert_eq!(tracks, vec!["Never", "Old"]);
}
