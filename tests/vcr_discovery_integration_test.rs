mod common;

#[tokio::test]
async fn login_and_discover_queen_scrobbles() {
    let client = common::create_lastfm_vcr_test_client("login_and_discover_queen_scrobbles")
        .await
        .expect("Failed to setup VCR client");

    // Test discover_scrobbles for Queen
    let edit = lastfm_edit::ScrobbleEdit::for_artist("Queen", "Queen");
    let mut discovery_iterator = client.discover_scrobbles(edit);

    // Collect a few results to verify it works
    // lets just count the total number of tracks here, and then just assert that the count is correct.
    let mut count = 0;
    while let Some(_exact_edit) = discovery_iterator
        .next()
        .await
        .expect("Failed to get next scrobble")
    {
        count += 1;
        if count >= 5 {
            // Just get first 5 for testing
            break;
        }
    }

    assert!(count > 0, "Should have found at least one Queen scrobble");
}
