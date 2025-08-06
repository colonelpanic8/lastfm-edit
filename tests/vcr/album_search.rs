use super::common;

#[test_log::test(tokio::test)]
async fn test_album_search() {
    let client = common::create_lastfm_vcr_test_client("album_search")
        .await
        .expect("Failed to setup VCR client");

    // Test searching for albums
    let mut album_search = client.search_albums("Abbey Road");
    let albums = album_search
        .take(5)
        .await
        .expect("Album search should succeed");

    assert!(
        !albums.is_empty(),
        "Should find some albums for 'Abbey Road'"
    );

    // Verify we got actual album results
    for album in &albums {
        assert!(!album.name.is_empty(), "Album name should not be empty");
        assert!(!album.artist.is_empty(), "Album artist should not be empty");
    }

    // Check that at least one result contains "Abbey Road" (case-insensitive)
    let found_abbey_road = albums
        .iter()
        .any(|album| album.name.to_lowercase().contains("abbey road"));
    assert!(
        found_abbey_road,
        "Should find an album containing 'Abbey Road'"
    );
}

#[test_log::test(tokio::test)]
async fn test_album_search_empty_query() {
    let client = common::create_lastfm_vcr_test_client("album_search_empty")
        .await
        .expect("Failed to setup VCR client");

    // Test searching with empty query
    let mut album_search = client.search_albums("");
    let albums = album_search
        .take(5)
        .await
        .expect("Empty album search should succeed");

    // Should return some results even with empty query
    assert!(!albums.is_empty(), "Empty search should return some albums");
}
