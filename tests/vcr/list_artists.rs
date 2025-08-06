use super::common;

#[test_log::test(tokio::test)]
async fn test_list_artists() {
    let client = common::create_lastfm_vcr_test_client("list_artists")
        .await
        .expect("Failed to setup VCR client");

    // Test getting artists from the user's library
    let mut artists_iterator = client.artists();
    let artists = artists_iterator
        .take(20)
        .await
        .expect("Artists list should succeed");

    assert!(
        !artists.is_empty(),
        "Should find some artists in user's library"
    );

    // Verify we got actual artist results
    for artist in &artists {
        assert!(!artist.name.is_empty(), "Artist name should not be empty");
        assert!(
            artist.playcount > 0,
            "Artist should have positive play count"
        );
    }

    // Artists should be sorted by play count (descending)
    for i in 1..artists.len() {
        assert!(
            artists[i - 1].playcount >= artists[i].playcount,
            "Artists should be sorted by play count in descending order"
        );
    }
}

#[test_log::test(tokio::test)]
async fn test_artists_pagination() {
    let client = common::create_lastfm_vcr_test_client("artists_pagination")
        .await
        .expect("Failed to setup VCR client");

    // Test that we can get multiple pages of artists
    let mut artists_iterator = client.artists();
    let first_batch = artists_iterator
        .take(5)
        .await
        .expect("First batch should succeed");
    let second_batch = artists_iterator
        .take(5)
        .await
        .expect("Second batch should succeed");

    assert_eq!(
        first_batch.len(),
        5,
        "Should get exactly 5 artists in first batch"
    );
    assert!(
        !second_batch.is_empty(),
        "Should get more artists in second batch"
    );

    // Artists should be different between batches
    let first_names: std::collections::HashSet<_> = first_batch.iter().map(|a| &a.name).collect();
    let second_names: std::collections::HashSet<_> = second_batch.iter().map(|a| &a.name).collect();

    let intersection = first_names.intersection(&second_names).count();
    assert!(
        intersection < first_batch.len(),
        "Batches should contain mostly different artists"
    );
}
