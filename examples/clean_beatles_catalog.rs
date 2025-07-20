#[path = "shared/common.rs"]
mod common;

use lastfm_edit::Result;
use regex::Regex;
use std::collections::HashSet;

#[tokio::main]
async fn main() -> Result<()> {
    let mut client = common::setup_client().await?;

    println!("=== Beatles Catalog Cleanup: Remove All Remastered Suffixes ===\n");

    let artist = "The Beatles";
    let regex = Regex::new(r" - Remastered( \d{4})?$").unwrap();

    println!(
        "🎯 TARGET: Remove all '- Remastered' and '- Remastered YYYY' suffixes from Beatles tracks"
    );
    println!("🎨 ARTIST: {}", artist);
    println!("🔍 PATTERN: Looking for tracks ending with '- Remastered' or '- Remastered YYYY'");
    println!("📝 EXAMPLES:");
    println!("   • 'Hey Jude - Remastered 2009' → 'Hey Jude'");
    println!("   • 'Let It Be - Remastered' → 'Let It Be'");
    println!("   • 'Yesterday - Remastered 2015' → 'Yesterday'");
    println!("\n🚀 Starting catalog scan...\n");

    // Track statistics
    let mut total_tracks_scanned = 0;
    let mut remastered_tracks_found = 0;
    let mut tracks_successfully_cleaned = 0;
    let mut tracks_failed_to_clean = 0;
    let mut already_cleaned_tracks = HashSet::new();

    // Step 1: Collect all remastered tracks first
    println!("🔍 Step 1: Scanning entire Beatles catalog for remastered tracks...");
    let mut all_remastered_tracks = Vec::new();

    {
        let mut iterator = client.artist_tracks(artist);
        let mut page_num = 1;

        loop {
            println!("📖 Scanning page {}...", page_num);

            match iterator.next_page().await {
                Ok(Some(page)) => {
                    total_tracks_scanned += page.tracks.len();

                    // Find remastered tracks on this page
                    for track in &page.tracks {
                        if regex.is_match(&track.name) {
                            let base_name = regex.replace(&track.name, "").to_string();
                            if !already_cleaned_tracks.contains(&base_name) {
                                all_remastered_tracks.push(track.clone());
                                already_cleaned_tracks.insert(base_name);
                                remastered_tracks_found += 1;
                            }
                        }
                    }

                    println!(
                        "   📊 Page {}: {} tracks scanned, {} total remastered found so far",
                        page_num,
                        page.tracks.len(),
                        all_remastered_tracks.len()
                    );

                    // Check if there are more pages
                    if !page.has_next_page {
                        println!("📚 Reached end of Beatles catalog");
                        break;
                    }

                    page_num += 1;
                }
                Ok(None) => {
                    println!("📚 No more pages available");
                    break;
                }
                Err(e) => {
                    println!("❌ Error fetching page {}: {}", page_num, e);
                    break;
                }
            }
        }
    }

    // Step 2: Process all found remastered tracks
    if all_remastered_tracks.is_empty() {
        println!("\n🎉 No remastered tracks found! Your Beatles catalog is already clean.");
        return Ok(());
    }

    println!(
        "\n🎯 Step 2: Processing {} remastered tracks...",
        all_remastered_tracks.len()
    );
    already_cleaned_tracks.clear(); // Reset for actual processing

    for (index, track) in all_remastered_tracks.iter().enumerate() {
        let clean_name = regex.replace(&track.name, "").to_string();

        println!(
            "\n🎵 [{}/{}] Cleaning: '{}' → '{}'",
            index + 1,
            all_remastered_tracks.len(),
            track.name,
            clean_name
        );

        // Skip if we've already processed this track name in this session
        if already_cleaned_tracks.contains(&clean_name) {
            println!("   ⏭️  Skipping - already processed in this session");
            continue;
        }

        // Load real edit form values from the track page
        match client.load_edit_form_values(&track.name, artist).await {
            Ok(mut edit_data) => {
                println!(
                    "   📋 Loaded edit form data - Album: '{}'",
                    edit_data.album_name_original
                );

                // Update the track name to the cleaned version
                edit_data.track_name = clean_name.clone();

                println!("   🔧 Submitting edit...");

                // Perform the edit
                match client.edit_scrobble(&edit_data).await {
                    Ok(response) => {
                        println!("   ✅ Successfully cleaned: '{}'", clean_name);
                        tracks_successfully_cleaned += 1;
                        already_cleaned_tracks.insert(clean_name);
                    }
                    Err(e) => {
                        println!("   ❌ Error editing '{}': {}", track.name, e);
                        tracks_failed_to_clean += 1;
                    }
                }
            }
            Err(e) => {
                println!("   ⚠️  Couldn't load edit form for '{}': {}", track.name, e);
                println!("      This track might not be in your recent scrobbles");
                tracks_failed_to_clean += 1;
            }
        }

        // Add a small delay to be respectful to Last.fm servers
        println!("   ⏳ Waiting 500ms before next track...");
        tokio::time::sleep(tokio::time::Duration::from_millis(1200)).await;
    }

    // Print final statistics
    println!("\n{}", "=".repeat(60));
    println!("🎼 BEATLES CATALOG CLEANUP COMPLETE");
    println!("{}", "=".repeat(60));
    println!("📊 STATISTICS:");
    println!("   • Total tracks scanned: {}", total_tracks_scanned);
    println!("   • Remastered tracks found: {}", remastered_tracks_found);
    println!(
        "   • Tracks successfully cleaned: {}",
        tracks_successfully_cleaned
    );
    println!("   • Tracks failed to clean: {}", tracks_failed_to_clean);

    if tracks_successfully_cleaned > 0 {
        println!("\n✨ Your Beatles catalog is now cleaner! All 'Remastered' suffixes have been removed.");
    }

    if tracks_failed_to_clean > 0 {
        println!("\n⚠️  Some tracks couldn't be cleaned. This usually happens when:");
        println!("   • The track hasn't been scrobbled recently");
        println!("   • The track data isn't in your listening history");
        println!("   • There were temporary server issues");
        println!("\n💡 You can re-run this script later to try cleaning the remaining tracks.");
    }

    println!("\n🎵 Beatles catalog cleanup completed!");

    Ok(())
}
