use super::*;
use crate::identity::adjudicate::{Case, CaseOption, OptionEvidence, OptionId};
use crate::identity::mb::{
    ArtistCredit, ArtistId, CreditPart, PrimaryType, RecordingId, ReleaseGroup, ReleaseGroupId,
    SecondaryType,
};
use crate::identity::{
    Canonical, Outcome, Proof, ResolutionEvent, ResolutionEventKind, Rung, Target, RESOLVER_VERSION,
};
use crate::subject::ScrobbleKey;
use uuid::Uuid;

fn credit(name: &str) -> ArtistCredit {
    ArtistCredit {
        parts: vec![CreditPart {
            artist: ArtistId(name.to_lowercase()),
            name: name.into(),
            joinphrase: String::new(),
        }],
    }
}

fn group(title: &str, artist: &str, secondary: Vec<SecondaryType>) -> ReleaseGroup {
    ReleaseGroup {
        id: ReleaseGroupId(title.to_lowercase()),
        title: title.into(),
        primary: Some(PrimaryType::Album),
        secondary,
        first_release: None,
        credit: credit(artist),
    }
}

fn outcome(title: &str, artist: &str, scrobbled: &ReleaseGroup, canonical: &str) -> Outcome {
    Outcome {
        master: vec![RecordingId(title.to_lowercase())],
        title: title.into(),
        scrobbled_group: Some(scrobbled.clone()),
        canonical: Some(Canonical {
            group: group(canonical, artist, Vec::new()),
            rung: Rung::PlainAlbum,
            single_first: false,
        }),
        proof: Proof::SameRecording,
    }
}

fn track(artist: &str, name: &str, album: &str) -> Track {
    Track {
        name: name.into(),
        artist: artist.into(),
        playcount: 1,
        timestamp: None,
        album: Some(album.into()),
        album_artist: None,
    }
}

fn key_of(track: &Track) -> ScrobbleKey {
    ScrobbleKey {
        artist: track.artist.clone(),
        track: track.name.clone(),
        album: track.album.clone(),
    }
}

fn event(track: &Track, kind: ResolutionEventKind) -> ResolutionEvent {
    ResolutionEvent {
        key: key_of(track),
        at: 1,
        resolver: RESOLVER_VERSION,
        kind,
    }
}

#[tokio::test]
async fn proposes_the_first_unrejected_plannable_outcome_per_track() {
    let horses = track(
        "The Rolling Stones",
        "Wild Horses - 2009 Re-Mastered Digital Version",
        "Sticky Fingers",
    );
    let four = track("Wes Montgomery", "Four on Six", "Smokin' at the Half Note");
    let cocaine = track("Eric Clapton", "Cocaine", "The Cream of Clapton");
    let unresolved = track(
        "Eric Clapton",
        "Layla",
        "Layla and Other Assorted Love Songs",
    );

    let sticky = group("Sticky Fingers", "The Rolling Stones", Vec::new());
    let half_note = group(
        "Smokin' at the Half Note",
        "Wes Montgomery",
        vec![SecondaryType::Live],
    );
    let cream = group(
        "The Cream of Clapton",
        "Eric Clapton",
        vec![SecondaryType::Compilation],
    );
    let options = ["Slowhand", "Backless"]
        .into_iter()
        .zip(['A', 'B'])
        .map(|(canonical, id)| CaseOption {
            id: OptionId(id),
            outcome: outcome("Cocaine", "Eric Clapton", &cream, canonical),
            evidence: OptionEvidence::default(),
        })
        .collect();
    let reader = ResolutionReader::from_events(vec![
        event(
            &horses,
            ResolutionEventKind::Resolved {
                outcome: outcome(
                    "Wild Horses",
                    "The Rolling Stones",
                    &sticky,
                    "Sticky Fingers",
                ),
            },
        ),
        event(
            &four,
            ResolutionEventKind::Resolved {
                outcome: outcome(
                    "Four on Six",
                    "Wes Montgomery",
                    &half_note,
                    "The Incredible Jazz Guitar of Wes Montgomery",
                ),
            },
        ),
        event(
            &cocaine,
            ResolutionEventKind::Ambiguous {
                case: Case {
                    key: key_of(&cocaine),
                    plays: 12,
                    siblings: Vec::new(),
                    options,
                },
                awaiting_llm: false,
            },
        ),
        event(
            &cocaine,
            ResolutionEventKind::HumanRejected {
                intent: Uuid::from_u128(1),
                target: Target {
                    title: "Cocaine".into(),
                    album: Some("Slowhand".into()),
                    album_artist: Some("Eric Clapton".into()),
                },
            },
        ),
    ]);

    let results = IdentityProvider::new(reader, MovePolicy::default(), RetitleJudge::Llm)
        .analyze_tracks(&[horses, four, cocaine, unresolved], None, None)
        .await
        .unwrap();
    let got: Vec<_> = results
        .iter()
        .map(|(idx, suggestions)| {
            let [suggestion] = suggestions.as_slice() else {
                panic!("one suggestion per track");
            };
            let crate::provider::ScrubActionSuggestion::Edit(edit) = &suggestion.suggestion else {
                panic!("an edit");
            };
            assert_eq!(suggestion.provider_name, IDENTITY_PROVIDER);
            assert!(suggestion.rationale.is_some());
            (
                *idx,
                edit.track_name.clone().unwrap(),
                edit.album_name.clone().unwrap(),
                suggestion.requires_confirmation,
            )
        })
        .collect();
    assert_eq!(
        got,
        vec![
            (0, "Wild Horses".into(), "Sticky Fingers".into(), false),
            (2, "Cocaine".into(), "Backless".into(), true),
        ]
    );
}
