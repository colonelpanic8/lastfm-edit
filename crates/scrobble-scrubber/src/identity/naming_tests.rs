use super::*;
use crate::identity::llm::scripted::ScriptedBackend;
use crate::identity::llm::RawChoice;
use crate::identity::testing::{credit, group};

#[tokio::test]
async fn answers_outside_keep_and_musicbrainz_are_unsure() {
    let case = NamingCase {
        question: NamingQuestion::album(
            "John Lennon",
            "Plastic Ono Band",
            "John Lennon/Plastic Ono Band",
        ),
        group: Some(group(
            "8be83468-4917-390d-82b6-d890f058cd22",
            "John Lennon/Plastic Ono Band",
            super::super::mb::PrimaryType::Album,
            &[],
            1970,
            credit(&[("4d5447d7", "John Lennon", "")]),
        )),
        albums: vec!["Plastic Ono Band".into()],
        tracks: vec!["Mother".into()],
        plays: 5,
    };
    let cases = [
        ("MusicBrainz", NamingVerdict::MusicBrainz),
        (" keep ", NamingVerdict::Keep),
        ("unsure", NamingVerdict::Unsure),
        ("John Lennon/Plastic Ono Band", NamingVerdict::Unsure),
        ("", NamingVerdict::Unsure),
    ];
    for (choice, expected) in cases {
        let backend = ScriptedBackend::default();
        backend.answers.lock().unwrap().push(RawChoice {
            choice: choice.into(),
            reason: format!("  {}  ", "é".repeat(400)),
            ..RawChoice::default()
        });
        let answer = judge(&backend, &case).await.unwrap();
        assert_eq!(
            (answer.verdict, answer.reason, answer.by),
            (expected, "é".repeat(300), "scripted".to_string()),
            "{choice:?}"
        );
    }
}

#[test]
fn edition_fixes_that_change_more_than_the_suffix_are_retitles() {
    let question = |artist: &str, album: &str, title: &str| {
        EditionFix::Retitle(NamingQuestion::album(artist, album, title))
    };
    let cases = [
        (
            "Béla Fleck and the Flecktones",
            "Bela Fleck And The Flecktones",
            "Béla Fleck and the Flecktones",
            question(
                "Béla Fleck and the Flecktones",
                "Bela Fleck And The Flecktones",
                "Béla Fleck and the Flecktones",
            ),
        ),
        (
            "John Lennon",
            "Plastic Ono Band (Remastered)",
            "John Lennon/Plastic Ono Band",
            question(
                "John Lennon",
                "Plastic Ono Band",
                "John Lennon/Plastic Ono Band",
            ),
        ),
        (
            "Jeff Buckley",
            "Grace (Legacy Edition)",
            "Grace",
            EditionFix::SuffixOnly,
        ),
        (
            "Eric Clapton",
            "Slowhand 35th Anniversary (Super Deluxe)",
            "Slowhand",
            EditionFix::SuffixOnly,
        ),
        (
            "The Who",
            "Who's Next (Deluxe Edition)",
            "Who\u{2019}s Next",
            EditionFix::SuffixOnly,
        ),
    ];
    for (artist, album, title, expected) in cases {
        assert_eq!(
            classify_edition_fix(artist, album, title),
            expected,
            "{album}"
        );
    }
}
