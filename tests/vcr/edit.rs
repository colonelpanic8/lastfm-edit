use super::common;

use lastfm_edit::ScrobbleEdit;

#[test_log::test(tokio::test)]
async fn edit_album() {
    let client = common::create_lastfm_vcr_test_client("edit_album")
        .await
        .expect("Failed to setup VCR client");

    // Create an edit to change "Who's Next (Deluxe Edition)" to "Who's Next"
    let edit = ScrobbleEdit::for_album(
        "Tommy (Deluxe Edition - International Version)",
        "The Who",
        "The Who",
    )
    .with_album_name("Tommy") // New album name without (Deluxe Edition)
    .with_edit_all(true); // Edit all matching scrobbles

    // Execute the edit
    let response = client
        .edit_scrobble(&edit)
        .await
        .expect("Edit should succeed");

    // Check that we got some edits back
    assert!(
        !response.individual_results.is_empty(),
        "Should have found at least one scrobble to edit"
    );

    // Verify the edit details
    for result in &response.individual_results {
        let exact_edit = &result.exact_scrobble_edit;
        assert_eq!(exact_edit.album_name, "Tommy");
        assert_eq!(exact_edit.artist_name_original, "The Who");
        assert_eq!(exact_edit.artist_name, "The Who");
    }
}

#[test_log::test(tokio::test)]
async fn edit_track() {
    let client = common::create_lastfm_vcr_test_client("edit_track")
        .await
        .expect("Failed to setup VCR client");

    // Create an edit to fix a single track (example: fix a typo or incorrect track name)
    let edit = ScrobbleEdit::from_track_and_artist(
        "Won't Get Fooled Again - Original Album Version",
        "The Who",
    )
    .with_track_name("Won't Get Fooled Again");

    // Execute the edit
    let response = client
        .edit_scrobble(&edit)
        .await
        .expect("Edit should succeed");

    // Verify the edit was successful
    assert!(response.success(), "Edit should be successful");

    // Check that we got exactly one edit back (since edit_all is false)
    assert_eq!(
        response.individual_results.len(),
        3,
        "Should have found three scrobbles to edit"
    );

    // Verify the edit details
    let result = &response.individual_results[0];
    let exact_edit = &result.exact_scrobble_edit;
    assert_eq!(
        exact_edit.track_name_original,
        "Won't Get Fooled Again - Original Album Version"
    );
    assert_eq!(exact_edit.track_name, "Won't Get Fooled Again");
    assert_eq!(exact_edit.artist_name_original, "The Who");
    assert_eq!(exact_edit.artist_name, "The Who");
}
