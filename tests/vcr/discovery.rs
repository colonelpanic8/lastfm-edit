use super::common;
use lastfm_edit::ScrobbleEdit;

/// Test artist tracks discovery (Case 4: neither track nor album specified)
#[test_log::test(tokio::test)]
async fn discover_artist_tracks() {
    let client = common::create_lastfm_vcr_test_client("discover_artist_tracks")
        .await
        .expect("Failed to setup VCR client");

    // Create an edit for all tracks by an artist (no specific track or album specified)
    let edit = ScrobbleEdit::for_artist("At the Drive-In", "At the Drive-In");

    // Use the discovery iterator to find tracks by the artist
    let mut discovery_iterator = client.discover_scrobbles(edit);

    // Collect all discoveries to test the functionality
    let mut discovered_edits = Vec::new();

    while let Some(discovered_edit) = discovery_iterator.next().await.unwrap() {
        discovered_edits.push(discovered_edit);
    }

    // Verify we found some tracks by the artist
    assert!(
        !discovered_edits.is_empty(),
        "Should have found at least one track by At the Drive-In"
    );

    // Verify all discovered edits are for the correct artist
    for edit in &discovered_edits {
        assert_eq!(
            edit.artist_name_original, "At the Drive-In",
            "All discovered tracks should be by At the Drive-In"
        );
        assert_eq!(
            edit.artist_name, "At the Drive-In",
            "Artist name should remain unchanged"
        );
        // Verify we have actual track names
        assert!(
            !edit.track_name_original.is_empty(),
            "Should have non-empty track name: {edit:?}"
        );
    }

    log::debug!(
        "Discovered {} tracks by At the Drive-In:",
        discovered_edits.len()
    );
    for edit in &discovered_edits {
        log::debug!(
            "  - '{}' from '{}'",
            edit.track_name_original,
            edit.album_name_original
        );
    }
}

#[test_log::test(tokio::test)]
async fn discover_exact_match() {
    let client = common::create_lastfm_vcr_test_client("discover_exact_match")
        .await
        .expect("Failed to setup VCR client");

    // Create an edit for a specific track and album combination
    let edit = ScrobbleEdit::new(
        Some("One Armed Scissor".to_string()), // original track name to search for
        Some("Relationship of Command".to_string()), // original album name to search for
        "At the Drive-In".to_string(),         // original artist name
        Some("At the Drive-In".to_string()),   // original album artist name
        None,                                  // new track name (same as original for discovery)
        None,                                  // new album name (same as original for discovery)
        "At the Drive-In".to_string(),         // new artist name (same as original for discovery)
        None, // new album artist name (same as original for discovery)
        None, // timestamp (client will find)
        true, // edit_all
    );

    // Use the discovery iterator
    let mut discovery_iterator = client.discover_scrobbles(edit);

    // Collect all discoveries (should be a small number for exact match)
    let mut discovered_edits = Vec::new();
    while let Some(discovered_edit) = discovery_iterator.next().await.unwrap() {
        discovered_edits.push(discovered_edit);
    }

    // Verify we found the exact match
    assert!(
        !discovered_edits.is_empty(),
        "Should have found exact match for One Armed Scissor from Relationship of Command"
    );

    // Verify all matches are for the correct track and album
    for edit in &discovered_edits {
        assert_eq!(edit.artist_name_original, "At the Drive-In");
        assert_eq!(edit.track_name_original, "One Armed Scissor");
        assert_eq!(edit.album_name_original, "Relationship of Command");
    }

    log::debug!(
        "Found {} exact matches for One Armed Scissor from Relationship of Command",
        discovered_edits.len()
    );
}

/// Test album tracks discovery (Case 3: album specified, track not specified)
#[test_log::test(tokio::test)]
async fn discover_album_tracks() {
    let client = common::create_lastfm_vcr_test_client("discover_album_tracks")
        .await
        .expect("Failed to setup VCR client");

    // Create an edit for all tracks from a specific album
    let edit = ScrobbleEdit::for_album("In/Casino/Out", "At the Drive-In", "At the Drive-In");

    // Use the discovery iterator
    let mut discovery_iterator = client.discover_scrobbles(edit);

    // Collect all discoveries from the album
    let mut discovered_edits = Vec::new();
    while let Some(discovered_edit) = discovery_iterator.next().await.unwrap() {
        discovered_edits.push(discovered_edit);
    }

    // Verify we found tracks from the album
    assert!(
        !discovered_edits.is_empty(),
        "Should have found tracks from In/Casino/Out album"
    );

    // Verify all tracks are from the correct album and artist
    for edit in &discovered_edits {
        assert_eq!(edit.artist_name_original, "At the Drive-In");
        assert_eq!(edit.album_name_original, "In/Casino/Out");
        assert!(!edit.track_name_original.is_empty());
    }

    log::debug!(
        "Found {} tracks from In/Casino/Out album:",
        discovered_edits.len()
    );
    for edit in &discovered_edits {
        log::debug!("  - '{}'", edit.track_name_original);
    }
}

/// Test track variations discovery (Case 2: track specified, album not specified)
#[test_log::test(tokio::test)]
async fn discover_track_variations() {
    let client = common::create_lastfm_vcr_test_client("discover_track_variations")
        .await
        .expect("Failed to setup VCR client");

    // Create an edit for a specific track across all albums
    let edit = ScrobbleEdit::from_track_and_artist("One Armed Scissor", "At the Drive-In");

    // Use the discovery iterator
    let mut discovery_iterator = client.discover_scrobbles(edit);

    // Collect all track variations
    let mut discovered_edits = Vec::new();
    while let Some(discovered_edit) = discovery_iterator.next().await.unwrap() {
        discovered_edits.push(discovered_edit);
    }

    // Verify we found track variations
    assert!(
        !discovered_edits.is_empty(),
        "Should have found variations of One Armed Scissor"
    );

    // Verify all variations are for the correct track and artist
    for edit in &discovered_edits {
        assert_eq!(edit.artist_name_original, "At the Drive-In");
        assert_eq!(edit.track_name_original, "One Armed Scissor");
        // Album names may vary (different releases, compilations, etc.)
    }

    log::debug!(
        "Found {} variations of One Armed Scissor:",
        discovered_edits.len()
    );
    for edit in &discovered_edits {
        log::debug!("  - from album '{}'", edit.album_name_original);
    }
}
