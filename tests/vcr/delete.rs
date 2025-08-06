use super::common;

#[test_log::test(tokio::test)]
async fn delete_scrobble_after_search() {
    let client = common::create_lastfm_vcr_test_client("delete_scrobble_after_search")
        .await
        .expect("Failed to setup VCR client");

    // First, search for the fake track to find its timestamp
    let found_track = client
        .find_recent_scrobble_for_track("Fake Track", "Fake Artist", 10)
        .await
        .expect("Failed to search for track")
        .expect("Fake Track by Fake Artist should exist in recent scrobbles");

    // Verify we found the correct track
    assert_eq!(found_track.artist, "Fake Artist");
    assert_eq!(found_track.name, "Fake Track");

    let timestamp = found_track
        .timestamp
        .expect("Track should have a timestamp for deletion");

    // Now delete the scrobble using the timestamp
    let delete_success = client
        .delete_scrobble("Fake Artist", "Fake Track", timestamp)
        .await
        .expect("Delete operation should not fail");

    assert!(
        delete_success,
        "Delete operation should succeed for the found scrobble"
    );
}
