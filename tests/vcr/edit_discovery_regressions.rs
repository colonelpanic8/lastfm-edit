use super::common;

use lastfm_edit::ScrobbleEdit;

#[test_log::test(tokio::test)]
async fn discover_album_tracks_continues_past_track_pages_without_chartlists() {
    let client = common::create_lastfm_vcr_test_client("discover_album_missing_chartlist")
        .await
        .expect("Failed to setup VCR client");

    // This album surfaced a Last.fm page-shape issue while scrubbing album
    // remaster metadata: one of the track pages returned no chartlist table.
    // Discovery should skip that track page and continue, not fail the whole
    // album batch.
    let edit = ScrobbleEdit::for_album(
        "Time Out - HD Digital Remastered 2009",
        "Dave Brubeck",
        "Dave Brubeck",
    )
    .with_album_name("Time Out")
    .with_edit_all(true);

    let mut discovery_iterator = client.discover_scrobbles(edit);
    let mut discovered_edits = Vec::new();

    while let Some(discovered_edit) = discovery_iterator
        .next()
        .await
        .expect("Discovery should not fail when one track page has no chartlist")
    {
        discovered_edits.push(discovered_edit);
    }

    assert!(
        !discovered_edits.is_empty(),
        "Should discover at least one editable scrobble from the album"
    );

    assert!(
        discovered_edits
            .iter()
            .any(|edit| edit.artist_name_original == "Dave Brubeck Quartet"),
        "Should use the track-row artist when discovering editable scrobbles"
    );

    for edit in &discovered_edits {
        assert_eq!(
            edit.album_name_original,
            "Time Out - HD Digital Remastered 2009"
        );
        assert_eq!(edit.album_name, "Time Out");
    }
}
