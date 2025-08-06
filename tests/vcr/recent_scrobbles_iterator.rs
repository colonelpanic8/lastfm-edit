use super::common;

#[test_log::test(tokio::test)]
async fn test_recent_scrobbles_iterator() {
    let client = common::create_lastfm_vcr_test_client("recent_scrobbles_iterator")
        .await
        .expect("Failed to setup VCR client");

    // Test getting recent tracks page (different from recent_tracks iterator)
    let recent_tracks_page = client
        .get_recent_tracks_page(2)
        .await
        .expect("Getting recent tracks page should succeed");

    assert!(
        !recent_tracks_page.tracks.is_empty(),
        "Should have some recent tracks"
    );

    // Verify each track has required data
    for track in &recent_tracks_page.tracks {
        assert!(!track.name.is_empty(), "Track name should not be empty");
        assert!(!track.artist.is_empty(), "Artist name should not be empty");
        // Note: album can be empty for some tracks, so we don't assert it
    }

    // Test that we get results from the page
    assert!(
        recent_tracks_page.tracks.len() >= 10,
        "Should get multiple tracks from page 2"
    );

    // Verify page metadata
    assert_eq!(recent_tracks_page.page_number, 2, "Page number should be 2");
}

#[test_log::test(tokio::test)]
async fn test_recent_tracks_from_page() {
    let client = common::create_lastfm_vcr_test_client("recent_tracks_from_page")
        .await
        .expect("Failed to setup VCR client");

    // Test starting from a specific page
    let mut tracks_iterator = client.recent_tracks_from_page(2);
    let tracks = tracks_iterator
        .take(5)
        .await
        .expect("Should get tracks from page 2");

    assert!(
        !tracks.is_empty(),
        "Should get tracks even starting from page 2"
    );

    for track in &tracks {
        assert!(!track.name.is_empty(), "Track name should not be empty");
        assert!(!track.artist.is_empty(), "Artist name should not be empty");
    }
}

#[test_log::test(tokio::test)]
async fn test_recent_tracks_limited_collection() {
    let client = common::create_lastfm_vcr_test_client("recent_tracks_limited_collection")
        .await
        .expect("Failed to setup VCR client");

    // Test collecting a reasonable number of tracks (not all)
    let mut tracks_iterator = client.recent_tracks();
    let tracks = tracks_iterator
        .take(50)
        .await
        .expect("take(50) should succeed");

    assert!(!tracks.is_empty(), "Should collect some tracks");
    assert!(
        tracks.len() >= 20,
        "Should collect a reasonable number of tracks"
    );

    // Verify tracks are in chronological order (newest first)
    for i in 1..tracks.len() {
        if let (Some(prev_ts), Some(curr_ts)) = (tracks[i - 1].timestamp, tracks[i].timestamp) {
            assert!(
                prev_ts >= curr_ts,
                "Tracks should be in descending chronological order"
            );
        }
    }
}
