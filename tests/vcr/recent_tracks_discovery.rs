use super::common;

#[test_log::test(tokio::test)]
async fn recent_tracks_discovery() {
    let client = common::create_lastfm_vcr_test_client("recent_tracks_discovery")
        .await
        .expect("Failed to setup VCR client");

    // Create an iterator for recent tracks and count them all
    let mut track_stream = client.recent_tracks();
    let mut total_tracks_checked = 0;

    while let Some(_track) = track_stream.next().await.expect("Failed to get next track") {
        total_tracks_checked += 1;
    }

    // Assert we found the expected number of tracks
    assert_eq!(
        total_tracks_checked, 200,
        "Should have found exactly 200 tracks"
    );
}
