use lastfm_edit::LastFmApiClient;

use super::common;

#[test_log::test(tokio::test)]
async fn test_api_recent_tracks_page() {
    let client = common::create_lastfm_api_vcr_test_client("api_recent_tracks")
        .await
        .expect("Failed to setup API VCR client");

    let page = client
        .api_get_recent_tracks_page(1)
        .await
        .expect("API recent tracks page should succeed");

    assert!(!page.tracks.is_empty(), "Should have tracks on page 1");
    assert_eq!(page.page_number, 1);

    for track in &page.tracks {
        assert!(!track.name.is_empty(), "Track name should not be empty");
        assert!(!track.artist.is_empty(), "Track artist should not be empty");
        assert!(
            track.timestamp.is_some(),
            "API tracks should have timestamps"
        );
        assert_eq!(track.playcount, 1, "API recent tracks have playcount 1");
    }
}

#[test_log::test(tokio::test)]
async fn test_api_recent_tracks_iterator() {
    let client = common::create_lastfm_api_vcr_test_client("api_recent_tracks_iterator")
        .await
        .expect("Failed to setup API VCR client");

    let mut iter = client.recent_tracks();
    let tracks = iter
        .take(10)
        .await
        .expect("Should get tracks from iterator");

    assert!(!tracks.is_empty(), "Should have at least some tracks");
    for track in &tracks {
        assert!(!track.name.is_empty());
        assert!(track.timestamp.is_some());
    }
}
