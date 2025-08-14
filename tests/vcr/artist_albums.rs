use super::common;
use std::collections::HashSet;

#[test_log::test(tokio::test)]
async fn artist_albums_beatles() {
    let client = common::create_lastfm_vcr_test_client("artist_albums_beatles")
        .await
        .expect("Failed to setup VCR client");

    let mut artist_albums = client.artist_albums("The Beatles");
    let mut unique_album_names = HashSet::new();

    while let Some(album) = artist_albums
        .next()
        .await
        .expect("Failed to get next album")
    {
        unique_album_names.insert(album.name.clone());
    }

    assert_eq!(
        unique_album_names.len(),
        38,
        "Expected 38 unique albums but got {}",
        unique_album_names.len()
    );
}
