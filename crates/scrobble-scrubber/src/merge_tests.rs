use super::*;
use EditField::{Album, AlbumArtist, Track};

const MB: &str = "musicbrainz_identity";
const RULES: &str = "rewrite_rules";

struct Scrobble {
    artist: &'static str,
    track: &'static str,
    album: &'static str,
}

const HORSES: Scrobble = Scrobble {
    artist: "The Rolling Stones",
    track: "Wild Horses - 2009 Re-Mastered Digital Version",
    album: "Sticky Fingers (Super Deluxe)",
};

const COCAINE: Scrobble = Scrobble {
    artist: "Eric Clapton",
    track: "Cocaine - 2012 Remaster",
    album: "The Cream of Clapton",
};

fn proposal(
    scrobble: &Scrobble,
    provider: &str,
    rank: usize,
    requires_confirmation: bool,
    sets: &[(EditField, &str)],
) -> Proposal {
    let mut edit = ScrobbleEdit {
        track_name_original: Some(scrobble.track.into()),
        album_name_original: Some(scrobble.album.into()),
        artist_name_original: scrobble.artist.into(),
        album_artist_name_original: None,
        track_name: Some(scrobble.track.into()),
        album_name: Some(scrobble.album.into()),
        artist_name: scrobble.artist.into(),
        album_artist_name: None,
        timestamp: Some(1_700_000_000),
        edit_all: true,
    };
    for &(field, value) in sets {
        field.set(&mut edit, Some(value));
    }
    Proposal {
        provider: provider.into(),
        rank,
        edit,
        requires_confirmation,
        rationale: Some(format!("{provider} says so")),
    }
}

#[derive(Debug, PartialEq)]
struct Summary {
    changes: Vec<(EditField, String)>,
    sources: Vec<(EditField, String)>,
    provider: String,
    requires_confirmation: bool,
    rationale: String,
}

fn owned(pairs: &[(EditField, &str)]) -> Vec<(EditField, String)> {
    pairs
        .iter()
        .map(|(field, value)| (*field, value.to_string()))
        .collect()
}

fn expect(
    changes: &[(EditField, &str)],
    sources: &[(EditField, &str)],
    provider: &str,
    requires_confirmation: bool,
    rationale: &str,
) -> Option<Summary> {
    Some(Summary {
        changes: owned(changes),
        sources: owned(sources),
        provider: provider.into(),
        requires_confirmation,
        rationale: rationale.into(),
    })
}

fn summarize(merged: Merged) -> Summary {
    Summary {
        changes: changes(&merged.edit)
            .into_iter()
            .map(|(field, value)| (field, value.to_string()))
            .collect(),
        sources: merged.sources.into_iter().collect(),
        provider: merged.provider,
        requires_confirmation: merged.requires_confirmation,
        rationale: merged.rationale.unwrap_or_default(),
    }
}

type Case = (
    &'static str,
    Vec<Proposal>,
    BTreeMap<EditField, usize>,
    Option<Summary>,
);

#[test]
fn merges_field_by_field_in_priority_order() {
    let cases: Vec<Case> = vec![
        (
            "the better-ranked title wins and the album comes along",
            vec![
                proposal(
                    &HORSES,
                    MB,
                    0,
                    false,
                    &[(Track, "Wild Horses"), (Album, "Sticky Fingers")],
                ),
                proposal(
                    &HORSES,
                    RULES,
                    1,
                    false,
                    &[(Track, "Wild Horses (Digital Version)")],
                ),
            ],
            BTreeMap::new(),
            expect(
                &[(Track, "Wild Horses"), (Album, "Sticky Fingers")],
                &[(Track, MB), (Album, MB)],
                MB,
                false,
                "musicbrainz_identity says so",
            ),
        ),
        (
            "a worse-ranked corrector fills a field the better one left alone",
            vec![
                proposal(&HORSES, RULES, 1, false, &[(Track, "Wild Horses")]),
                proposal(&HORSES, MB, 0, false, &[(Album, "Sticky Fingers")]),
            ],
            BTreeMap::new(),
            expect(
                &[(Track, "Wild Horses"), (Album, "Sticky Fingers")],
                &[(Track, RULES), (Album, MB)],
                MB,
                false,
                "musicbrainz_identity says so\nrewrite_rules says so",
            ),
        ),
        (
            "album and album artist come from one corrector",
            vec![
                proposal(&HORSES, RULES, 0, false, &[(Album, "Sticky Fingers")]),
                proposal(
                    &HORSES,
                    MB,
                    1,
                    false,
                    &[
                        (Album, "Sticky Fingers (Remastered)"),
                        (AlbumArtist, "The Rolling Stones"),
                    ],
                ),
            ],
            BTreeMap::new(),
            expect(
                &[(Album, "Sticky Fingers")],
                &[(Album, RULES)],
                RULES,
                false,
                "rewrite_rules says so",
            ),
        ),
        (
            "any contributor needing review sends the merge to review",
            vec![
                proposal(
                    &COCAINE,
                    MB,
                    0,
                    true,
                    &[(Album, "Slowhand"), (AlbumArtist, "Eric Clapton")],
                ),
                proposal(&COCAINE, RULES, 1, false, &[(Track, "Cocaine")]),
            ],
            BTreeMap::new(),
            expect(
                &[
                    (Track, "Cocaine"),
                    (Album, "Slowhand"),
                    (AlbumArtist, "Eric Clapton"),
                ],
                &[(Track, RULES), (Album, MB), (AlbumArtist, MB)],
                MB,
                true,
                "musicbrainz_identity says so\nrewrite_rules says so",
            ),
        ),
        (
            "a reviewed proposal that contributes nothing does not force review",
            vec![
                proposal(&COCAINE, MB, 0, true, &[]),
                proposal(&COCAINE, RULES, 1, false, &[(Track, "Cocaine")]),
            ],
            BTreeMap::new(),
            expect(
                &[(Track, "Cocaine")],
                &[(Track, RULES)],
                RULES,
                false,
                "rewrite_rules says so",
            ),
        ),
        (
            "nothing changes",
            vec![
                proposal(&COCAINE, MB, 0, false, &[]),
                proposal(&COCAINE, RULES, 1, false, &[]),
            ],
            BTreeMap::new(),
            None,
        ),
        (
            "a worse-ranked corrector may not change a field a better one owns",
            vec![proposal(
                &COCAINE,
                RULES,
                1,
                false,
                &[(Track, "Cocaine"), (Album, "Backless")],
            )],
            BTreeMap::from([(Album, 0)]),
            expect(
                &[(Track, "Cocaine")],
                &[(Track, RULES)],
                RULES,
                false,
                "rewrite_rules says so",
            ),
        ),
        (
            "the owner itself may change its field again",
            vec![proposal(&COCAINE, MB, 0, false, &[(Album, "Slowhand")])],
            BTreeMap::from([(Album, 0)]),
            expect(
                &[(Album, "Slowhand")],
                &[(Album, MB)],
                MB,
                false,
                "musicbrainz_identity says so",
            ),
        ),
        (
            "an owned-field drop that leaves nothing is no edit",
            vec![proposal(&COCAINE, RULES, 1, false, &[(Album, "Backless")])],
            BTreeMap::from([(Album, 0)]),
            None,
        ),
    ];
    for (name, proposals, owned, expected) in cases {
        let got = merge(&proposals, &owned).map(summarize);
        assert_eq!(got, expected, "{name}");
    }
}
