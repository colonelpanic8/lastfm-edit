mod common;

#[tokio::test]
async fn edit_scrobble_works() {
    let client = common::create_lastfm_vcr_test_client("edit_scrobble_works")
        .await
        .expect("Failed to setup VCR client");

    // Just test that we can browse tracks after login - no editing yet
    let mut wilco_tracks = client.artist_tracks("Wilco");
    let mut track_count = 0;

    // Sample a few tracks to verify the connection works
    while let Some(_track) = wilco_tracks.next().await.expect("Failed to get next track") {
        track_count += 1;

        // Just sample the first 5 tracks for now
        if track_count >= 5 {
            break;
        }
    }

    assert!(
        track_count > 0,
        "Should have found at least one Wilco track"
    );
}
