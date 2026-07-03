use lastfm_edit::LastFmApiClient;

use super::common;

#[test_log::test(tokio::test)]
async fn test_api_recent_tracks_page() {
    let client = common::create_lastfm_api_vcr_test_client("api_recent_tracks")
        .await
        .expect("Failed to setup API VCR client");

    let page = client
        .api_get_recent_tracks_page(1)
        .await
        .expect("API recent tracks page should succeed");

    assert!(!page.tracks.is_empty(), "Should have tracks on page 1");
    assert_eq!(page.page_number, 1);

    for track in &page.tracks {
        assert!(!track.name.is_empty(), "Track name should not be empty");
        assert!(!track.artist.is_empty(), "Track artist should not be empty");
        assert!(
            track.timestamp.is_some(),
            "API tracks should have timestamps"
        );
        assert_eq!(track.playcount, 1, "API recent tracks have playcount 1");
    }
}

#[test_log::test(tokio::test)]
async fn test_api_recent_tracks_iterator() {
    let client = common::create_lastfm_api_vcr_test_client("api_recent_tracks_iterator")
        .await
        .expect("Failed to setup API VCR client");

    let mut iter = client.recent_tracks();
    let tracks = iter
        .take(10)
        .await
        .expect("Should get tracks from iterator");

    assert!(!tracks.is_empty(), "Should have at least some tracks");
    for track in &tracks {
        assert!(!track.name.is_empty());
        assert!(track.timestamp.is_some());
    }
}

/// Records/replays a windowed fetch and pins down the API's `from`/`to` edge semantics.
///
/// The window is derived from timestamps observed on an unwindowed first page, so the
/// cassette stays self-consistent when re-recorded.
#[test_log::test(tokio::test)]
async fn test_api_recent_tracks_in_range() {
    let client = common::create_lastfm_api_vcr_test_client("api_recent_tracks_in_range")
        .await
        .expect("Failed to setup API VCR client");

    let page = client
        .api_get_recent_tracks_page(1)
        .await
        .expect("unwindowed page should succeed");
    let timestamps: Vec<u64> = page.tracks.iter().filter_map(|t| t.timestamp).collect();
    assert!(timestamps.len() >= 10, "need a reasonably full page");

    // Newest first: pick an interior window [from_ts, to_ts] a few tracks in.
    let to_ts = timestamps[2];
    let from_ts = timestamps[8];
    assert!(from_ts < to_ts);

    let ranged = client
        .api_get_recent_tracks_page_in_range(1, Some(from_ts), Some(to_ts))
        .await
        .expect("windowed page should succeed");

    let ranged_ts: Vec<u64> = ranged.tracks.iter().filter_map(|t| t.timestamp).collect();
    assert!(!ranged_ts.is_empty(), "window should contain tracks");
    for ts in &ranged_ts {
        assert!(
            *ts >= from_ts && *ts < to_ts,
            "track at {ts} outside half-open window [{from_ts}, {to_ts})"
        );
    }

    // Edge semantics observed against the live service on 2026-07-03 (contrary to the API
    // docs' "strictly after"/"strictly before" wording): `from` is INCLUSIVE and `to` is
    // EXCLUSIVE — a native half-open [from, to) window. If a re-recording trips these
    // assertions, last.fm changed behavior and ApiSource's window math must be revisited.
    assert!(
        ranged_ts.contains(&from_ts),
        "`from` edge should be included (inclusive from)"
    );
    assert!(
        !ranged_ts.contains(&to_ts),
        "`to` edge should be excluded (exclusive to)"
    );
}
