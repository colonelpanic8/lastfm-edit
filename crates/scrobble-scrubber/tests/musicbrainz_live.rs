//! Live MusicBrainz tests ported from the original scrobble-scrubber
//! (`lib/tests/musicbrainz_confirmation.rs` and `lib/tests/compilation_to_canonical.rs`).
//!
//! Every test in the old suites hit the real MusicBrainz API (the old repo gated them
//! behind a `SCROBBLE_SCRUBBER_SKIP_LIVE_MB_TESTS` env var). This crate's policy is NO
//! network in tests, so a representative subset is ported here as `#[ignore]`d tests;
//! run them explicitly with:
//!
//! ```sh
//! cargo test -p scrobble-scrubber --features musicbrainz -- --ignored
//! ```
//!
//! The remaining old cases (nirvana_nevermind, drake/RHCP/Gaga/Weeknd deluxe cases,
//! beatles_anthology_tests.rs, rejected_musicbrainz_cases.rs,
//! compilation_provider_precise_tests.rs) were the same shape — live-network
//! album-confirmation checks — and were dropped rather than duplicated.
#![cfg(feature = "musicbrainz")]

use lastfm_edit::Track;
use scrobble_scrubber::musicbrainz::CompilationToCanonicalProvider;
use scrobble_scrubber::provider::{
    RewriteRulesScrubActionProvider, ScrubActionProvider, ScrubActionSuggestion,
};
use scrobble_scrubber::rewrite::{RewriteRule, SdRule};
use std::collections::HashMap;

/// Test data for a track that should or should not be renamed
struct TrackTestCase {
    track_name: String,
    should_be_renamed: bool,
    expected_album: Option<String>,
}

/// Helper function to test MusicBrainz confirmation rules on albums
async fn check_mb_confirmation_rule(
    rule: RewriteRule,
    artist: &str,
    original_album: &str,
    test_cases: Vec<TrackTestCase>,
) {
    let provider = RewriteRulesScrubActionProvider::from_rules(vec![rule]);

    // Build tracks from test cases
    let tracks: Vec<Track> = test_cases
        .iter()
        .enumerate()
        .map(|(idx, tc)| Track {
            name: tc.track_name.clone(),
            artist: artist.to_string(),
            album: Some(original_album.to_string()),
            album_artist: None,
            playcount: 1,
            timestamp: Some(1_600_000_000 + idx as u64 * 100),
        })
        .collect();

    let results = provider
        .analyze_tracks(&tracks, None, None)
        .await
        .expect("analysis should succeed");

    // Convert results to a map for easier assertions
    let mut map = HashMap::new();
    for (idx, suggestions) in results {
        map.insert(idx, suggestions);
    }

    // Check each test case
    for (idx, tc) in test_cases.iter().enumerate() {
        if tc.should_be_renamed {
            let suggestions = map
                .get(&idx)
                .unwrap_or_else(|| panic!("Track '{}' should produce a suggestion", tc.track_name));
            assert!(
                !suggestions.is_empty(),
                "Expected at least one suggestion for '{}'",
                tc.track_name
            );

            // Find an Edit suggestion and verify album changed
            let mut found = false;
            for s in suggestions {
                if let ScrubActionSuggestion::Edit(edit) = &s.suggestion {
                    if let Some(expected) = &tc.expected_album {
                        if edit.album_name.as_deref() == Some(expected.as_str()) {
                            found = true;
                            break;
                        }
                    }
                }
            }
            if let Some(expected) = &tc.expected_album {
                assert!(
                    found,
                    "Expected album to be rewritten to '{}' for '{}'",
                    expected, tc.track_name
                );
            }
        } else {
            assert!(
                !map.contains_key(&idx),
                "Track '{}' should not be rewritten from '{}' to '{:?}'",
                tc.track_name,
                original_album,
                tc.expected_album
            );
        }
    }
}

#[test_log::test(tokio::test)]
#[ignore = "hits the live MusicBrainz API"]
async fn elliott_smith_xo() {
    // Rule: remove "(Deluxe Edition)" from album names, but only when MB confirms the
    // (artist, title, album) exists
    let rule = RewriteRule::new()
        .with_album_name(SdRule::new(r"^(.*) \(Deluxe Edition\)$", "$1").with_flags("i"))
        .with_musicbrainz_confirmation_required(true);

    check_mb_confirmation_rule(
        rule,
        "Elliott Smith",
        "XO (Deluxe Edition)",
        vec![
            TrackTestCase {
                track_name: "Miss Misery".to_string(),
                should_be_renamed: false,
                expected_album: Some("XO".to_string()),
            },
            TrackTestCase {
                track_name: "Independence Day".to_string(),
                should_be_renamed: true,
                expected_album: Some("XO".to_string()),
            },
        ],
    )
    .await;
}

#[test_log::test(tokio::test)]
#[ignore = "hits the live MusicBrainz API"]
async fn jeff_buckley_grace() {
    // Rule: remove "(Legacy Edition)" from album names, but only when MB confirms
    let rule = RewriteRule::new()
        .with_album_name(SdRule::new(r"^(.*) \(Legacy Edition\)$", "$1").with_flags("i"))
        .with_musicbrainz_confirmation_required(true);

    check_mb_confirmation_rule(
        rule,
        "Jeff Buckley",
        "Grace (Legacy Edition)",
        vec![
            TrackTestCase {
                track_name: "Grace".to_string(),
                should_be_renamed: true,
                expected_album: Some("Grace".to_string()),
            },
            TrackTestCase {
                track_name: "I Want Someone Badly".to_string(),
                should_be_renamed: false,
                expected_album: Some("Grace".to_string()),
            },
        ],
    )
    .await;
}

#[test_log::test(tokio::test)]
#[ignore = "hits the live MusicBrainz API"]
async fn greatest_hits_to_original() {
    let provider = CompilationToCanonicalProvider::new();

    // "Bohemian Rhapsody" from Queen's Greatest Hits -> should suggest "A Night at the Opera"
    let track = Track {
        name: "Bohemian Rhapsody".to_string(),
        artist: "Queen".to_string(),
        album: Some("Greatest Hits".to_string()),
        album_artist: Some("Queen".to_string()),
        timestamp: Some(1_600_000_000),
        playcount: 1,
    };

    let results = provider
        .analyze_tracks(&[track], None, None)
        .await
        .expect("Provider should not error");

    // The API might not always return suggestions, especially for well-known albums
    // that might be considered canonical releases themselves
    if results.is_empty() {
        log::warn!("No suggestions returned for 'Bohemian Rhapsody' from Greatest Hits - this can happen if MusicBrainz considers it a primary release");
        return;
    }

    let (idx, suggestions) = &results[0];
    assert_eq!(*idx, 0);

    if suggestions.is_empty() {
        log::warn!(
            "No suggestions for this track - MusicBrainz might not have found earlier releases"
        );
        return;
    }

    if let ScrubActionSuggestion::Edit(edit) = &suggestions[0].suggestion {
        let suggested_album = edit
            .album_name
            .as_ref()
            .expect("Should have album suggestion");
        assert_ne!(
            suggested_album, "Greatest Hits",
            "Should not suggest the same compilation"
        );
        // Check it's not another compilation
        assert!(
            !suggested_album.to_lowercase().contains("greatest")
                && !suggested_album.to_lowercase().contains("hits")
                && !suggested_album.to_lowercase().contains("collection"),
            "Should not suggest another compilation: {suggested_album}"
        );
    } else {
        panic!("Expected Edit suggestion for compilation track");
    }
}

// -------------------------------------------------------------------------------------
// Network-free sanity checks (run in the normal suite)
// -------------------------------------------------------------------------------------

#[test]
fn provider_names_are_stable_snake_case_identifiers() {
    use scrobble_scrubber::musicbrainz::MusicBrainzScrubActionProvider;

    assert_eq!(
        MusicBrainzScrubActionProvider::default().provider_name(),
        "musicbrainz"
    );
    assert_eq!(
        CompilationToCanonicalProvider::new().provider_name(),
        "compilation_to_canonical"
    );
}

#[test_log::test(tokio::test)]
async fn empty_track_batches_short_circuit_without_network() {
    let mb = scrobble_scrubber::musicbrainz::MusicBrainzScrubActionProvider::default();
    assert!(mb.analyze_tracks(&[], None, None).await.unwrap().is_empty());

    let comp = CompilationToCanonicalProvider::new().with_enabled(false);
    let track = Track {
        name: "Song".to_string(),
        artist: "Artist".to_string(),
        album: Some("Album".to_string()),
        album_artist: None,
        timestamp: None,
        playcount: 1,
    };
    // Disabled provider must not touch the network even with tracks present.
    assert!(comp
        .analyze_tracks(&[track], None, None)
        .await
        .unwrap()
        .is_empty());
}
