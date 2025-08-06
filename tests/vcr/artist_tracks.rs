use super::common;
use std::collections::HashSet;

#[test_log::test(tokio::test)]
async fn artist_tracks() {
    let client = common::create_lastfm_vcr_test_client("artist_tracks")
        .await
        .expect("Failed to setup VCR client");

    let mut artist_tracks = client.artist_tracks("The Beatles");
    let mut unique_track_names = HashSet::new();
    let mut _total_count = 0;

    while let Some(track) = artist_tracks
        .next()
        .await
        .expect("Failed to get next track")
    {
        unique_track_names.insert(track.name.clone());
        _total_count += 1;
    }

    // Verify we got the expected number of tracks
    assert_eq!(
        unique_track_names.len(),
        204,
        "Should have exactly 204 unique track names, but found {}",
        unique_track_names.len()
    );
}

#[test_log::test(tokio::test)]
async fn artist_tracks_direct() {
    let client = common::create_lastfm_vcr_test_client("artist_tracks_direct")
        .await
        .expect("Failed to setup VCR client");

    let mut artist_tracks = client.artist_tracks_direct("The Beatles");
    let mut unique_track_names = HashSet::new();
    let mut _total_count = 0;

    while let Some(track) = artist_tracks
        .next()
        .await
        .expect("Failed to get next track")
    {
        // Track loaded successfully
        unique_track_names.insert(track.name.clone());
        _total_count += 1;
    }

    // Verify we got the expected number of tracks
    assert_eq!(
        unique_track_names.len(),
        204,
        "Should have exactly 192 unique track names, but found {}",
        unique_track_names.len()
    );
}
