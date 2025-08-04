#[path = "../common/mod.rs"] mod common;

/// Test getting album tracks
#[test_log::test(tokio::test)]
async fn get_album_tracks() {
    let client = common::create_lastfm_vcr_test_client("get_album_tracks")
        .await
        .expect("Failed to setup VCR client");

    // Get all tracks from Pink Floyd's Dark Side of the Moon
    let mut album_tracks = client.album_tracks("Pink Floyd", "The Dark Side of the Moon");
    let mut count = 0;

    while let Some(_track) = album_tracks.next().await.expect("Failed to get next track") {
        count += 1;
    }

    println!("Total album tracks found: {}", count);
    assert!(
        count > 0,
        "Should have found at least one track from the album"
    );
}