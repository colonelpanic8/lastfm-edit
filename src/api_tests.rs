use super::*;

#[test]
fn test_parse_api_recent_tracks() {
    let json = r##"{
            "recenttracks": {
                "track": [
                    {
                        "name": "Test Track",
                        "artist": {"#text": "Test Artist"},
                        "album": {"#text": "Test Album"},
                        "date": {"uts": "1700000000"}
                    },
                    {
                        "name": "Now Playing",
                        "artist": {"#text": "Some Artist"},
                        "album": {"#text": "Some Album"},
                        "@attr": {"nowplaying": "true"}
                    }
                ],
                "@attr": {
                    "page": "1",
                    "totalPages": "5"
                }
            }
        }"##;

    let page = parse_api_recent_tracks_response(json).unwrap();
    assert_eq!(page.tracks.len(), 1);
    assert_eq!(page.tracks[0].name, "Test Track");
    assert_eq!(page.tracks[0].artist, "Test Artist");
    assert_eq!(page.tracks[0].album.as_deref(), Some("Test Album"));
    // The API provides no album-artist information; it must not be fabricated.
    assert_eq!(page.tracks[0].album_artist, None);
    assert_eq!(page.tracks[0].timestamp, Some(1700000000));
    assert_eq!(page.tracks[0].playcount, 1);
    assert_eq!(page.page_number, 1);
    assert!(page.has_next_page);
    assert_eq!(page.total_pages, Some(5));
}

#[test]
fn test_parse_single_track_as_object() {
    // The API returns a bare object (not a one-element array) for single-track pages.
    let json = r##"{
            "recenttracks": {
                "track": {
                    "name": "Solo",
                    "artist": {"#text": "Artist"},
                    "album": {"#text": "Album"},
                    "date": {"uts": "1700000000"}
                },
                "@attr": {"page": "1", "totalPages": "1"}
            }
        }"##;
    let page = parse_api_recent_tracks_response(json).unwrap();
    assert_eq!(page.tracks.len(), 1);
    assert_eq!(page.tracks[0].name, "Solo");
}

#[test]
fn test_parse_empty_page_without_track_field() {
    let json = r##"{
            "recenttracks": {
                "@attr": {"page": "1", "totalPages": "0"}
            }
        }"##;
    let page = parse_api_recent_tracks_response(json).unwrap();
    assert!(page.tracks.is_empty());
}

#[test]
fn test_api_error_body_is_surfaced() {
    let json = r##"{"error":6,"message":"User not found"}"##;
    let err = parse_api_recent_tracks_response(json).unwrap_err();
    assert!(err.to_string().contains("User not found"), "{err}");
}

#[test]
fn test_parse_api_last_page() {
    let json = r##"{
            "recenttracks": {
                "track": [
                    {
                        "name": "Track",
                        "artist": {"#text": "Artist"},
                        "album": {"#text": "Album"},
                        "date": {"uts": "1700000000"}
                    }
                ],
                "@attr": {
                    "page": "3",
                    "totalPages": "3"
                }
            }
        }"##;

    let page = parse_api_recent_tracks_response(json).unwrap();
    assert!(!page.has_next_page);
    assert_eq!(page.page_number, 3);
}

#[test]
fn test_build_recent_tracks_url_without_range() {
    let url = build_recent_tracks_url("someuser", "apikey123", 2, None, None);
    assert_eq!(
            url,
            "https://ws.audioscrobbler.com/2.0/?method=user.getrecenttracks&user=someuser&api_key=apikey123&format=json&page=2&limit=200"
        );
    assert!(!url.contains("&from="));
    assert!(!url.contains("&to="));
}

#[test]
fn test_build_recent_tracks_url_with_from_and_to() {
    let url = build_recent_tracks_url(
        "someuser",
        "apikey123",
        1,
        Some(1700000000),
        Some(1700086400),
    );
    assert_eq!(
            url,
            "https://ws.audioscrobbler.com/2.0/?method=user.getrecenttracks&user=someuser&api_key=apikey123&format=json&page=1&limit=200&from=1700000000&to=1700086400"
        );
}

#[test]
fn test_build_recent_tracks_url_with_only_from() {
    let url = build_recent_tracks_url("someuser", "apikey123", 1, Some(1700000000), None);
    assert!(url.ends_with("&limit=200&from=1700000000"));
    assert!(!url.contains("&to="));
}

#[test]
fn test_build_recent_tracks_url_with_only_to() {
    let url = build_recent_tracks_url("someuser", "apikey123", 1, None, Some(1700086400));
    assert!(url.ends_with("&limit=200&to=1700086400"));
    assert!(!url.contains("&from="));
}

#[test]
fn test_build_recent_tracks_url_encodes_username() {
    let url = build_recent_tracks_url("some user&x", "key", 1, None, None);
    assert!(url.contains("user=some%20user%26x"));
}
