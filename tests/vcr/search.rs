use super::common;

/// Test artist search using our test utilities
#[test_log::test(tokio::test)]
async fn search_iterator() {
    let client = common::create_lastfm_vcr_test_client("search")
        .await
        .expect("Failed to setup VCR client");

    let mut search_tracks = client.search_tracks("moon");
    let mut count = 0;

    while let Some(_track) = search_tracks
        .next()
        .await
        .expect("Failed to get next track")
    {
        count += 1;
    }

    assert!(count == 122);
}
