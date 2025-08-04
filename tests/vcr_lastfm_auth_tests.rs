mod common;

/// Test the login -> recent tracks flow using our test utilities
#[tokio::test]
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
#[tokio::test]
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
#[tokio::test]
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
