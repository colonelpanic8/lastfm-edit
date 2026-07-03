use super::common;

#[test_log::test(tokio::test)]
async fn test_album_search_field_order() {
    let client = common::create_lastfm_vcr_test_client("album_search_field_swap")
        .await
        .expect("Failed to setup VCR client");

    // Search for a well-known album where we know the expected artist
    let mut album_search = client.search_albums("OK Computer");
    let albums = album_search
        .take(10)
        .await
        .expect("Album search should succeed");

    assert!(!albums.is_empty(), "Should find albums for 'OK Computer'");

    // Look for the Radiohead album specifically
    let radiohead_album = albums.iter().find(|album| {
        album.name.to_lowercase().contains("ok computer")
            && album.artist.to_lowercase().contains("radiohead")
    });

    assert!(
        radiohead_album.is_some(),
        "Should find 'OK Computer' by Radiohead in search results"
    );

    let album = radiohead_album.unwrap();

    // Verify the fields are correctly assigned
    println!(
        "Album found - Name: '{}', Artist: '{}'",
        album.name, album.artist
    );

    // The album name should contain "OK Computer" (not "Radiohead")
    assert!(
        album.name.to_lowercase().contains("ok computer"),
        "Album name field should contain 'OK Computer', but got: '{}'",
        album.name
    );

    // The artist name should contain "Radiohead" (not "OK Computer")
    assert!(
        album.artist.to_lowercase().contains("radiohead"),
        "Album artist field should contain 'Radiohead', but got: '{}'",
        album.artist
    );

    // Additional verification - the fields should NOT be swapped
    assert!(
        !album.name.to_lowercase().contains("radiohead"),
        "Album name should NOT contain 'Radiohead' (indicates fields are swapped)"
    );
    assert!(
        !album.artist.to_lowercase().contains("ok computer"),
        "Artist name should NOT contain 'OK Computer' (indicates fields are swapped)"
    );
}

#[test_log::test(tokio::test)]
#[ignore = "cassette album_search_beatles has not been recorded yet"]
async fn test_album_search_multiple_artists() {
    let client = common::create_lastfm_vcr_test_client("album_search_beatles")
        .await
        .expect("Failed to setup VCR client");

    // Search for an album that multiple artists might have covered
    let mut album_search = client.search_albums("Abbey Road");
    let albums = album_search
        .take(20)
        .await
        .expect("Album search should succeed");

    assert!(!albums.is_empty(), "Should find albums for 'Abbey Road'");

    // Find The Beatles' Abbey Road
    let beatles_album = albums.iter().find(|album| {
        album.name.to_lowercase().contains("abbey road")
            && album.artist.to_lowercase().contains("beatles")
    });

    if let Some(album) = beatles_album {
        println!(
            "Beatles album - Name: '{}', Artist: '{}'",
            album.name, album.artist
        );

        // Verify correct field assignment
        assert!(
            album.name.to_lowercase().contains("abbey road"),
            "Album name should be 'Abbey Road', not the artist name"
        );
        assert!(
            album.artist.to_lowercase().contains("beatles"),
            "Artist should be 'The Beatles', not the album name"
        );
    }

    // Check all results for consistency
    for album in &albums {
        println!("Album: '{}' by '{}'", album.name, album.artist);

        // Basic sanity check - if we searched for "Abbey Road",
        // any result with "Abbey Road" should have it in the name field, not artist
        if album.name.to_lowercase().contains("abbey road") {
            assert!(
                !album.artist.to_lowercase().contains("abbey road"),
                "Found 'Abbey Road' in artist field instead of album name field for: {:?}",
                album
            );
        }
    }
}
