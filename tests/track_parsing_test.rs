use lastfm_edit::{LastFmClient, Result};
use scraper::Html;
use std::fs;

#[tokio::test]
async fn test_neil_young_track_parsing() -> Result<()> {
    // Load the saved HTML file
    let html_file = "neil_young_tracks_page_1.html";
    let html_content = fs::read_to_string(html_file)
        .map_err(|e| lastfm_edit::LastFmError::Parse(format!("Failed to read HTML file: {}", e)))?;

    // Parse the HTML using scraper
    let document = Html::parse_document(&html_content);

    // Create a client to access the parsing method (we don't need to login for parsing)
    let http_client = http_client::native::NativeClient::new();
    let client = LastFmClient::new(Box::new(http_client));

    // Test the parsing function
    let track_page = client.parse_tracks_page(&document, 1, "Neil Young")?;

    // Basic sanity checks
    assert!(!track_page.tracks.is_empty(), "Should find some tracks");

    // Verify we can find some known tracks
    let track_names: Vec<&str> = track_page.tracks.iter().map(|t| t.name.as_str()).collect();
    assert!(
        track_names.contains(&"Heart of Gold"),
        "Should contain 'Heart of Gold'"
    );

    // The critical test: "Comes a Time - 2016" should be found
    let target_track = "Comes a Time - 2016";
    let found_target = track_page.tracks.iter().any(|t| t.name == target_track);

    if !found_target {
        // Print debug info before failing
        println!("❌ Track '{}' not found in parsed results", target_track);
        println!("📊 Total tracks parsed: {}", track_page.tracks.len());
        println!("🔍 All parsed tracks:");
        for (i, track) in track_page.tracks.iter().enumerate() {
            println!("  [{:2}] '{}'", i + 1, track.name);
        }

        // Check if it exists in the HTML
        let html_contains_track = html_content.contains(target_track);
        println!(
            "📂 HTML contains '{}': {}",
            target_track, html_contains_track
        );

        if html_contains_track {
            println!("💥 PARSING BUG: Track exists in HTML but not in parsed results!");
        }
    }

    assert!(
        found_target,
        "Track '{}' should be found in parsed tracks",
        target_track
    );

    Ok(())
}

#[test]
fn test_html_contains_comes_a_time() {
    // Verify the HTML file actually contains the track we're looking for
    let html_content =
        fs::read_to_string("neil_young_tracks_page_1.html").expect("HTML file should exist");

    assert!(
        html_content.contains("Comes a Time - 2016"),
        "HTML should contain 'Comes a Time - 2016'"
    );

    // Count occurrences
    let occurrences = html_content.matches("Comes a Time - 2016").count();
    assert!(
        occurrences > 0,
        "Should find multiple occurrences of the track"
    );

    println!(
        "✅ HTML contains 'Comes a Time - 2016' {} times",
        occurrences
    );
}
