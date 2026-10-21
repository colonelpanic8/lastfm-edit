use super::*;

fn date(year: u16, month: Option<u8>, day: Option<u8>) -> Option<PartialDate> {
    Some(PartialDate { year, month, day })
}

#[test]
fn parses_musicbrainz_dates() {
    let cases = [
        ("1977", date(1977, None, None)),
        ("1977-11", date(1977, Some(11), None)),
        ("1977-11-25", date(1977, Some(11), Some(25))),
        ("", None),
        ("????", None),
        ("1977-13", None),
        ("1977-11-25-01", None),
        ("77", None),
    ];
    for (input, expected) in cases {
        assert_eq!(PartialDate::parse(input), expected, "{input}");
    }
    assert!(PartialDate::parse("1977") < PartialDate::parse("1977-01-01"));
}

#[test]
fn credits_read_as_credited() {
    let credit = ArtistCredit {
        parts: vec![
            CreditPart {
                artist: ArtistId("wes-montgomery".to_string()),
                name: "Wes Montgomery".to_string(),
                joinphrase: " & ".to_string(),
            },
            CreditPart {
                artist: ArtistId("wynton-kelly-trio".to_string()),
                name: "Wynton Kelly Trio".to_string(),
                joinphrase: String::new(),
            },
        ],
    };
    assert_eq!(credit.display(), "Wes Montgomery & Wynton Kelly Trio");
    assert!(!credit.is_various_artists());

    let various = ArtistCredit {
        parts: vec![CreditPart {
            artist: ArtistId(VARIOUS_ARTISTS_ID.to_string()),
            name: "Various Artists".to_string(),
            joinphrase: String::new(),
        }],
    };
    assert!(various.is_various_artists());
}

#[test]
fn describes_release_groups() {
    let cream = ReleaseGroup {
        id: ReleaseGroupId("976d6745-0091-39f1-ae60-c142986485a5".to_string()),
        title: "The Cream of Clapton".to_string(),
        primary: Some(PrimaryType::Album),
        secondary: vec![SecondaryType::Compilation],
        first_release: PartialDate::parse("1995-03-13"),
        credit: ArtistCredit::default(),
    };
    assert_eq!(
        cream.describe(),
        "The Cream of Clapton (Album + Compilation, 1995)"
    );

    let untyped = ReleaseGroup {
        primary: None,
        secondary: vec![],
        first_release: None,
        ..cream
    };
    assert_eq!(untyped.describe(), "The Cream of Clapton");
}

#[test]
fn lucene_quotes_and_escapes_phrases() {
    let query = RecordingQuery {
        title: "Wild Horses".to_string(),
        artist: "The Rolling Stones".to_string(),
        release: Some("Sticky Fingers".to_string()),
    };
    assert_eq!(
        query.lucene(),
        r#"recording:"Wild Horses" AND artist:"The Rolling Stones" AND release:"Sticky Fingers""#
    );

    let quoted = RecordingQuery {
        title: r#"12" Mix \ Dub"#.to_string(),
        artist: "Bobby \"Blue\" Bland".to_string(),
        release: None,
    };
    assert_eq!(
        quoted.lucene(),
        r#"recording:"12\" Mix \\ Dub" AND artist:"Bobby \"Blue\" Bland""#
    );
}

#[test]
fn user_agent_requires_contact() {
    assert_eq!(
        UserAgent::new("scrobble-scrubber/0.2", "ivanmalison@gmail.com")
            .unwrap()
            .as_str(),
        "scrobble-scrubber/0.2 ( ivanmalison@gmail.com )"
    );
    assert!(UserAgent::new("scrobble-scrubber/0.2", " ").is_err());
}
