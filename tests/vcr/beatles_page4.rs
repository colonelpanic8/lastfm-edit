use super::common;

/// Test getting exactly page 4 of Beatles tracks - should return 50 tracks
#[test_log::test(tokio::test)]
async fn beatles_page4_tracks() {
    let client = common::create_lastfm_vcr_test_client("beatles_page4")
        .await
        .expect("Failed to setup VCR client");

    let page = client
        .get_artist_tracks_page("The Beatles", 4)
        .await
        .expect("Failed to get Beatles page 4 tracks");

    log::debug!(
        "Page 4 Beatles tracks: {} tracks, has_next_page: {}, total_pages: {:?}",
        page.tracks.len(),
        page.has_next_page,
        page.total_pages
    );

    // Print all track names for debugging
    for (i, track) in page.tracks.iter().enumerate() {
        log::info!(
            "  {}. {} (played {} times)",
            i + 1,
            track.name,
            track.playcount
        );
    }

    // Note: We expected 50 tracks but Last.fm only returns 48 for page 4
    // This could be due to:
    // 1. Some tracks being filtered out by Last.fm
    // 2. Deduplication happening on Last.fm's side
    // 3. Changes in the user's library
    assert_eq!(
        page.tracks.len(),
        50,
        "Page 4 should have exactly 50 tracks, but found {}. Missing tracks: 'Love Me Do [single version]' and 'Some Like It Hot!'",
        page.tracks.len()
    );

    assert_eq!(page.page_number, 4, "Page number should be 4");
}
