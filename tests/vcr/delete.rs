use super::common;
use lastfm_edit::delete_manifest::{execute_delete_targets, DeleteTarget};
use std::time::Duration;

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

#[test_log::test(tokio::test)]
async fn delete_single_scrobble_from_manifest() {
    let client = common::create_lastfm_vcr_test_client("delete_scrobble_after_search")
        .await
        .expect("Failed to setup VCR client");

    let found_track = client
        .find_recent_scrobble_for_track("Fake Track", "Fake Artist", 10)
        .await
        .expect("Failed to search for track")
        .expect("Fake Track by Fake Artist should exist in recent scrobbles");

    let timestamp = found_track
        .timestamp
        .expect("Track should have a timestamp for deletion");

    let targets = vec![DeleteTarget {
        offset: Some(0),
        artist: "Fake Artist".to_string(),
        track: "Fake Track".to_string(),
        album: found_track.album,
        timestamp,
    }];

    let mut attempts = Vec::new();
    let summary = execute_delete_targets(
        client.as_ref(),
        &targets,
        Duration::ZERO,
        |index, target, result| {
            attempts.push((index, target.clone(), result.clone()));
        },
    )
    .await
    .expect("Manifest delete execution should not fail");

    assert_eq!(summary.total_found, 1);
    assert_eq!(summary.successful_deletions, 1);
    assert_eq!(summary.failed_deletions, 0);
    assert_eq!(attempts.len(), 1);
    assert!(attempts[0].2.success());
}
