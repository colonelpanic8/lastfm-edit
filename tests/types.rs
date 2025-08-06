use lastfm_edit::types::{Album, Artist, ExactScrobbleEdit, ScrobbleEdit, Track};

#[test]
fn test_display_implementations() {
    // Test Artist Display
    let artist = Artist {
        name: "The Beatles".to_string(),
        playcount: 100,
        timestamp: Some(1234567890),
    };
    assert_eq!(format!("{artist}"), "The Beatles");

    // Test Album Display
    let album = Album {
        name: "Abbey Road".to_string(),
        artist: "The Beatles".to_string(),
        playcount: 50,
        timestamp: Some(1234567890),
    };
    assert_eq!(format!("{album}"), "The Beatles - Abbey Road");

    // Test Track Display with album
    let track_with_album = Track {
        name: "Come Together".to_string(),
        artist: "The Beatles".to_string(),
        playcount: 10,
        timestamp: Some(1234567890),
        album: Some("Abbey Road".to_string()),
        album_artist: None,
    };
    assert_eq!(
        format!("{track_with_album}"),
        "The Beatles - Come Together [Abbey Road]"
    );

    // Test Track Display without album
    let track_without_album = Track {
        name: "Yesterday".to_string(),
        artist: "The Beatles".to_string(),
        playcount: 15,
        timestamp: Some(1234567890),
        album: None,
        album_artist: None,
    };
    assert_eq!(format!("{track_without_album}"), "The Beatles - Yesterday");

    // Test ScrobbleEdit Display - no changes
    let no_changes_edit = ScrobbleEdit {
        track_name_original: Some("Yesterday".to_string()),
        album_name_original: Some("Help!".to_string()),
        artist_name_original: "The Beatles".to_string(),
        album_artist_name_original: None,
        track_name: Some("Yesterday".to_string()),
        album_name: Some("Help!".to_string()),
        artist_name: "The Beatles".to_string(),
        album_artist_name: None,
        timestamp: Some(1234567890),
        edit_all: false,
    };
    assert_eq!(format!("{no_changes_edit}"), "No changes");

    // Test ScrobbleEdit Display - artist change only
    let artist_edit = ScrobbleEdit {
        track_name_original: Some("Yesterday".to_string()),
        album_name_original: Some("Help!".to_string()),
        artist_name_original: "The Beatles".to_string(),
        album_artist_name_original: None,
        track_name: Some("Yesterday".to_string()),
        album_name: Some("Help!".to_string()),
        artist_name: "Beatles, The".to_string(),
        album_artist_name: None,
        timestamp: Some(1234567890),
        edit_all: false,
    };
    assert_eq!(
        format!("{artist_edit}"),
        "Artist: The Beatles → Beatles, The"
    );

    // Test ScrobbleEdit Display - multiple field changes
    let multi_edit = ScrobbleEdit {
        track_name_original: Some("Yesterday".to_string()),
        album_name_original: Some("Help!".to_string()),
        artist_name_original: "The Beatles".to_string(),
        album_artist_name_original: None,
        track_name: Some("Yesterday (Remastered)".to_string()),
        album_name: Some("Help! (Deluxe Edition)".to_string()),
        artist_name: "Beatles, The".to_string(),
        album_artist_name: None,
        timestamp: Some(1234567890),
        edit_all: false,
    };
    assert_eq!(format!("{multi_edit}"), "Artist: The Beatles → Beatles, The, Track: Yesterday → Yesterday (Remastered), Album: Help! → Help! (Deluxe Edition)");

    // Test ScrobbleEdit Display - with edit_all flag
    let edit_all = ScrobbleEdit {
        track_name_original: Some("Yesterday".to_string()),
        album_name_original: Some("Help!".to_string()),
        artist_name_original: "The Beatles".to_string(),
        album_artist_name_original: None,
        track_name: Some("Yesterday".to_string()),
        album_name: Some("Help!".to_string()),
        artist_name: "Beatles, The".to_string(),
        album_artist_name: None,
        timestamp: Some(1234567890),
        edit_all: true,
    };
    assert_eq!(
        format!("{edit_all}"),
        "Artist: The Beatles → Beatles, The (all instances)"
    );

    // Test ExactScrobbleEdit Display - no changes
    let exact_no_changes = ExactScrobbleEdit {
        track_name_original: "Yesterday".to_string(),
        album_name_original: "Help!".to_string(),
        artist_name_original: "The Beatles".to_string(),
        album_artist_name_original: "The Beatles".to_string(),
        track_name: "Yesterday".to_string(),
        album_name: "Help!".to_string(),
        artist_name: "The Beatles".to_string(),
        album_artist_name: "The Beatles".to_string(),
        timestamp: 1234567890,
        edit_all: false,
    };
    assert_eq!(format!("{exact_no_changes}"), "No changes");

    // Test ExactScrobbleEdit Display - artist change
    let exact_artist_change = ExactScrobbleEdit {
        track_name_original: "Yesterday".to_string(),
        album_name_original: "Help!".to_string(),
        artist_name_original: "The Beatles".to_string(),
        album_artist_name_original: "The Beatles".to_string(),
        track_name: "Yesterday".to_string(),
        album_name: "Help!".to_string(),
        artist_name: "Beatles, The".to_string(),
        album_artist_name: "The Beatles".to_string(),
        timestamp: 1234567890,
        edit_all: false,
    };
    assert_eq!(
        format!("{exact_artist_change}"),
        "Artist: The Beatles → Beatles, The"
    );

    // Test ExactScrobbleEdit Display - multiple changes with edit_all
    let exact_multi_changes = ExactScrobbleEdit {
        track_name_original: "Yesterday".to_string(),
        album_name_original: "Help!".to_string(),
        artist_name_original: "The Beatles".to_string(),
        album_artist_name_original: "The Beatles".to_string(),
        track_name: "Yesterday (Remastered)".to_string(),
        album_name: "Help! (Deluxe Edition)".to_string(),
        artist_name: "Beatles, The".to_string(),
        album_artist_name: "Beatles, The".to_string(),
        timestamp: 1234567890,
        edit_all: true,
    };
    assert_eq!(format!("{exact_multi_changes}"), "Artist: The Beatles → Beatles, The, Track: Yesterday → Yesterday (Remastered), Album: Help! → Help! (Deluxe Edition), Album Artist: The Beatles → Beatles, The (all instances)");
}
