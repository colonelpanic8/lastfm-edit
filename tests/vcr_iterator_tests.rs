mod common;

/// Test the login -> recent tracks flow using our test utilities
#[test_log::test(tokio::test)]
async fn login_and_get_recent_tracks() {
    let client = common::create_lastfm_vcr_test_client("login_and_get_recent_tracks")
        .await
        .expect("Failed to setup VCR client");

    // Test getting recent tracks (first 10)
    let mut recent_tracks = client.recent_tracks();
    let mut count = 0;

    while let Some(_track) = recent_tracks
        .next()
        .await
        .expect("Failed to get next track")
    {
        count += 1;
        if count >= 10 {
            break;
        }
    }

    assert!(count > 0, "Should have found at least one recent track");
}

/// Test the login -> artist search flow using our test utilities
#[test_log::test(tokio::test)]
async fn login_and_search_artist() {
    let client = common::create_lastfm_vcr_test_client("login_and_search_artist")
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

/// Test login -> album tracks flow
#[test_log::test(tokio::test)]
async fn login_and_get_album_tracks() {
    let client = common::create_lastfm_vcr_test_client("login_and_get_album_tracks")
        .await
        .expect("Failed to setup VCR client");

    // Get tracks from Pink Floyd's Dark Side of the Moon
    let mut album_tracks = client.album_tracks("Pink Floyd", "The Dark Side of the Moon");
    let mut count = 0;

    while let Some(_track) = album_tracks.next().await.expect("Failed to get next track") {
        count += 1;
        if count >= 10 {
            break;
        }
    }

    assert!(
        count > 0,
        "Should have found at least one track from the album"
    );
}

#[test_log::test(tokio::test)]
async fn vcr_login_test() {
    // Create VCR client that records login interaction for this test
    common::create_lastfm_vcr_test_client_with_login_recording("vcr_login_test")
        .await
        .expect("Client creation should succeed");
}

#[test_log::test(tokio::test)]
async fn vcr_recent_tracks_discovery() {
    let client = common::create_lastfm_vcr_test_client("vcr_recent_tracks_discovery")
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
