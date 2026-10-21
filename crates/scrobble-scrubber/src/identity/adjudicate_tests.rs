use super::*;
use crate::identity::canonical::{Canonical, Rung};
use crate::identity::llm::scripted::ScriptedBackend;
use crate::identity::mb::{
    ArtistCredit, ArtistId, CreditPart, PartialDate, PrimaryType, RecordingId, ReleaseGroup,
    ReleaseGroupId, SecondaryType,
};

const SCROBBLED_REC: &str = "aca688be-742b-4c44-96b9-2278bdc483c3";
const STUDIO_REC: &str = "40ae4c06-3799-413b-b879-101dec39ac44";

fn key(track: &str, album: Option<&str>) -> ScrobbleKey {
    ScrobbleKey {
        artist: "Eric Clapton".into(),
        track: track.into(),
        album: album.map(Into::into),
    }
}

fn clapton() -> ArtistCredit {
    ArtistCredit {
        parts: vec![CreditPart {
            artist: ArtistId("618b6900-0618-4f1e-b835-bccb17f84294".into()),
            name: "Eric Clapton".into(),
            joinphrase: String::new(),
        }],
    }
}

fn group(
    id: &str,
    title: &str,
    primary: PrimaryType,
    secondary: Vec<SecondaryType>,
    year: u16,
) -> ReleaseGroup {
    ReleaseGroup {
        id: ReleaseGroupId(id.into()),
        title: title.into(),
        primary: Some(primary),
        secondary,
        first_release: Some(PartialDate {
            year,
            month: None,
            day: None,
        }),
        credit: clapton(),
    }
}

fn cream_of_clapton() -> ReleaseGroup {
    group(
        "976d6745-0091-39f1-ae60-c142986485a5",
        "The Cream of Clapton",
        PrimaryType::Album,
        vec![SecondaryType::Compilation],
        1995,
    )
}

fn cocaine_case() -> Case {
    let slowhand = Outcome {
        master: vec![
            RecordingId(SCROBBLED_REC.into()),
            RecordingId(STUDIO_REC.into()),
        ],
        title: "Cocaine".into(),
        scrobbled_group: Some(cream_of_clapton()),
        canonical: Some(Canonical {
            group: group(
                "02c2b0c7-065d-38b4-8ac0-3391839f2418",
                "Slowhand",
                PrimaryType::Album,
                vec![],
                1977,
            ),
            rung: Rung::PlainAlbum,
            single_first: false,
        }),
        proof: Proof::SharedIsrc {
            isrc: "NLF057790024".into(),
            len_delta_ms: 2107,
        },
    };
    let slowhand_evidence = OptionEvidence {
        recordings: vec![
            (
                SCROBBLED_REC.into(),
                "Cocaine".into(),
                String::new(),
                Some(218_747),
            ),
            (
                STUDIO_REC.into(),
                "Cocaine".into(),
                "original studio mix".into(),
                Some(216_640),
            ),
        ],
        isrcs: vec![
            "NLF057790024".into(),
            "USRY20517111".into(),
            "GBA077700020".into(),
        ],
        work_attributes: vec![],
        scrobbled_group: Some("The Cream of Clapton (Album + Compilation, 1995)".into()),
        canonical_group: Some("Slowhand (Album, 1977)".into()),
    };
    let live = Outcome {
        master: vec![RecordingId("426a6973-9583-4761-8cd1-efab138a98f5".into())],
        title: "Cocaine".into(),
        scrobbled_group: Some(cream_of_clapton()),
        canonical: None,
        proof: Proof::SameWork,
    };
    let live_evidence = OptionEvidence {
        recordings: vec![(
            "426a6973-9583-4761-8cd1-efab138a98f5".into(),
            "Cocaine".into(),
            String::new(),
            Some(532_000),
        )],
        isrcs: vec![],
        work_attributes: vec!["live".into()],
        scrobbled_group: Some("The Cream of Clapton (Album + Compilation, 1995)".into()),
        canonical_group: None,
    };
    Case::new(
        key("Cocaine", Some("The Cream of Clapton")),
        37,
        vec![
            key("Cocaine", Some("Slowhand")),
            key("Cocaine", Some("Slowhand 35th Anniversary (Super Deluxe)")),
        ],
        vec![(slowhand, slowhand_evidence), (live, live_evidence)],
    )
}

fn precedents() -> Vec<Precedent> {
    vec![
        Precedent {
            key: key("Lay Down Sally", Some("The Cream of Clapton")),
            proposed_album: Some("Slowhand".into()),
            proposed_title: None,
            accepted: true,
        },
        Precedent {
            key: key("Wonderful Tonight", Some("Just One Night")),
            proposed_album: Some("Slowhand".into()),
            proposed_title: Some("Wonderful Tonight".into()),
            accepted: false,
        },
    ]
}

#[test]
fn renders_the_cocaine_case() {
    let expected = r#"Scrobble
  artist: "Eric Clapton"
  title: "Cocaine"
  album: "The Cream of Clapton"
  plays: 37
Same title under the user's other albums:
  "Cocaine" on "Slowhand"
  "Cocaine" on "Slowhand 35th Anniversary (Super Deluxe)"

Option A
  target: title "Cocaine", album "Slowhand"
  proof: different recording entries sharing ISRC NLF057790024, lengths 2.1 s apart
  scrobbled release group: The Cream of Clapton (Album + Compilation, 1995)
  canonical release group: Slowhand (Album, 1977)
  recordings:
    aca688be-742b-4c44-96b9-2278bdc483c3 "Cocaine" 3:38
    40ae4c06-3799-413b-b879-101dec39ac44 "Cocaine" (original studio mix) 3:36
  ISRCs: NLF057790024, USRY20517111, GBA077700020
  work attributes: none

Option B
  target: title "Cocaine", album unchanged
  proof: same work, different recording, no shared ISRC
  scrobbled release group: The Cream of Clapton (Album + Compilation, 1995)
  recordings:
    426a6973-9583-4761-8cd1-efab138a98f5 "Cocaine" 8:52
  ISRCs: none
  work attributes: live

Earlier human decisions on this artist:
  "Lay Down Sally" on "The Cream of Clapton", proposed album "Slowhand": accepted
  "Wonderful Tonight" on "Just One Night", proposed title "Wonderful Tonight", album "Slowhand": rejected

Answer with one of: A, B, keep, unsure."#;
    assert_eq!(render_case(&cocaine_case(), &precedents()), expected);
}

#[test]
fn case_hash_ignores_plays_but_not_evidence() {
    let case = cocaine_case();
    let more_plays = Case {
        plays: 120,
        ..case.clone()
    };
    let mut other_isrc = case.clone();
    other_isrc.options[0].evidence.isrcs.pop();

    assert_eq!(case_hash(&more_plays), case_hash(&case));
    assert_ne!(case_hash(&other_isrc), case_hash(&case));
    assert_eq!(case_hash(&case).0.len(), 64);
}

#[test]
fn validate_coerces_hostile_answers() {
    let cases = [
        (
            "A",
            "high",
            Choice::Option(OptionId('A')),
            Confidence::High,
            false,
        ),
        (
            " b ",
            "Medium",
            Choice::Option(OptionId('B')),
            Confidence::Medium,
            false,
        ),
        ("KEEP", "low", Choice::Keep, Confidence::Low, false),
        ("unsure", "certain", Choice::Unsure, Confidence::Low, false),
        ("C", "high", Choice::Unsure, Confidence::High, true),
        ("option A", "high", Choice::Unsure, Confidence::High, true),
        ("", "", Choice::Unsure, Confidence::Low, true),
        (
            "The studio version on Slowhand is the right one.",
            "high",
            Choice::Unsure,
            Confidence::High,
            true,
        ),
    ];
    let case = cocaine_case();
    for (choice, confidence, expected_choice, expected_confidence, invalid) in cases {
        let raw = RawChoice {
            choice: choice.into(),
            confidence: confidence.into(),
            reason: "  Shared ISRC.  ".into(),
            text: String::new(),
        };
        assert_eq!(
            validate(&raw, &case),
            Verdict {
                choice: expected_choice,
                confidence: expected_confidence,
                reason: "Shared ISRC.".into(),
                invalid,
            },
            "choice {choice:?}, confidence {confidence:?}"
        );
    }
}

#[test]
fn validate_caps_the_reason() {
    let raw = RawChoice {
        choice: "A".into(),
        reason: "é".repeat(400),
        ..RawChoice::default()
    };
    assert_eq!(validate(&raw, &cocaine_case()).reason, "é".repeat(300));
}

#[tokio::test]
async fn adjudicate_asks_with_the_allowed_set_and_audits_the_answer() {
    let text = format!(
        r#"{{"choice":"a","confidence":"high","reason":"Shared ISRC."}} {}"#,
        "ü".repeat(2100)
    );
    let backend = ScriptedBackend::default();
    backend.answers.lock().unwrap().push(RawChoice {
        choice: "a".into(),
        confidence: "high".into(),
        reason: "Shared ISRC.".into(),
        text: text.clone(),
    });
    let backend = std::sync::Arc::new(backend);
    let adjudicator = Adjudicator::new(Box::new(backend.clone()), 200);
    let case = cocaine_case();

    let adjudication = adjudicator.adjudicate(&case, &precedents()).await.unwrap();

    assert_eq!(adjudication.by, "scripted");
    assert_eq!(adjudication.case_hash, case_hash(&case));
    assert_eq!(
        adjudication.verdict,
        Verdict {
            choice: Choice::Option(OptionId('A')),
            confidence: Confidence::High,
            reason: "Shared ISRC.".into(),
            invalid: false,
        }
    );
    assert_eq!(adjudication.raw.len(), 4095);
    assert!(text.starts_with(&adjudication.raw));
    let requests = backend.requests.lock().unwrap();
    assert_eq!(
        requests.as_slice(),
        [ChoiceRequest {
            system: SYSTEM_PROMPT.into(),
            user: render_case(&case, &precedents()),
            allowed: vec!["A".into(), "B".into(), "keep".into(), "unsure".into()],
        }]
    );
}
