mod common;

use lastfm_edit::Result;
use std::collections::HashSet;

#[tokio::main]
async fn main() -> Result<()> {
    let mut client = common::setup_client().await?;

    let artist = "The Beatles";
    println!("\nTesting pagination for: {}", artist);
    println!("Expected: 7 pages with ~329 different songs");

    let mut iterator = client.artist_tracks(artist);
    let mut all_tracks = Vec::new();
    let mut page_count = 0;
    let mut track_names = HashSet::new();

    // Collect all pages
    while let Some(page) = iterator.next_page().await? {
        page_count += 1;
        println!(
            "Page {}: {} tracks (has_next: {}, total_pages: {:?})",
            page.page_number,
            page.tracks.len(),
            page.has_next_page,
            page.total_pages
        );

        // Debug: Show what URL would be constructed for next page
        if page_count <= 2 {
            let next_page_url = format!(
                "https://www.last.fm/user/IvanMalison/library/music/{}/+tracks?page={}",
                urlencoding::encode(artist),
                page.page_number + 1
            );
            println!("  Next page URL would be: {}", next_page_url);
        }

        // Show first few tracks from each page to verify different content
        if page_count <= 3 || page_count == 7 {
            // Show first 3 and last page
            println!("  Sample tracks:");
            for (i, track) in page.tracks.iter().take(3).enumerate() {
                println!("    {}. {} - {} plays", i + 1, track.name, track.playcount);
            }
        }

        // Track unique song names
        for track in &page.tracks {
            track_names.insert(track.name.clone());
        }

        all_tracks.extend(page.tracks);

        // Safety break to avoid infinite loops
        if page_count >= 10 {
            println!("⚠️  Stopping at page 10 for safety");
            break;
        }
    }

    println!("\n--- Summary ---");
    println!("Total pages fetched: {}", page_count);
    println!("Total track entries: {}", all_tracks.len());
    println!("Unique song names: {}", track_names.len());

    // Verify expectations
    if page_count == 7 {
        println!("✅ Page count matches expected (7)");
    } else {
        println!("❌ Page count mismatch: expected 7, got {}", page_count);
    }

    if track_names.len() >= 320 && track_names.len() <= 340 {
        println!("✅ Unique songs approximately match expected (~329)");
    } else {
        println!(
            "❌ Unique songs mismatch: expected ~329, got {}",
            track_names.len()
        );
    }

    // Check if pagination stopped correctly
    if page_count < 10 {
        println!("✅ Pagination stopped naturally");
    } else {
        println!("⚠️  Pagination was stopped artificially at page 10");
    }

    // Show some unique song names as a sanity check
    println!("\nSample unique song names:");
    for (i, name) in track_names.iter().take(10).enumerate() {
        println!("  {}. {}", i + 1, name);
    }

    Ok(())
}
