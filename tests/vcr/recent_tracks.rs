use super::common;

/// Test getting recent tracks using our test utilities
#[test_log::test(tokio::test)]
async fn get_recent_tracks() {
    let client = common::create_lastfm_vcr_test_client("recent_tracks")
        .await
        .expect("Failed to setup VCR client");

    // Test getting all recent tracks
    let mut recent_tracks = client.recent_tracks();
    let mut count = 0;

    while let Some(_track) = recent_tracks
        .next()
        .await
        .expect("Failed to get next track")
    {
        count += 1;
        if count > 60 {
            break;
        }
    }
}
