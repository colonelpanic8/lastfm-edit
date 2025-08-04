use super::common;

/// Test artist search using our test utilities
#[test_log::test(tokio::test)]
async fn search_artist() {
    let client = common::create_lastfm_vcr_test_client("search_artist")
        .await
        .expect("Failed to setup VCR client");

    // Search for Beatles tracks
    let mut search_tracks = client.search_tracks("Beatles");
    let mut count = 0;

    while let Some(_track) = search_tracks
        .next()
        .await
        .expect("Failed to get next track")
    {
        count += 1;
        if count >= 5 {
            break;
        }
    }

    assert!(count > 0, "Should have found at least one Beatles track");
}