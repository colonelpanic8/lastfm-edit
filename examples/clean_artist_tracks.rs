#[path = "shared/common.rs"]
mod common;

use lastfm_edit::{LastFmEditClient, Result};
use regex::Regex;
use std::collections::HashSet;
use std::env;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();

    if args.len() != 3 {
        eprintln!(
            "Usage: cargo run --example clean_artist_tracks -- \"Artist Name\" \"Regex Pattern\""
        );
        eprintln!("Examples:");
        eprintln!("  # Remove remastered suffixes:");
        eprintln!("  cargo run --example clean_artist_tracks -- \"The Beatles\" \" - Remastered( \\d{{4}})?$\"");
        eprintln!("  # Remove live suffixes:");
        eprintln!("  cargo run --example clean_artist_tracks -- \"Pink Floyd\" \" \\(Live\\)$\"");
        eprintln!("  # Remove explicit tags:");
        eprintln!("  cargo run --example clean_artist_tracks -- \"Eminem\" \" \\(Explicit\\)$\"");
        std::process::exit(1);
    }

    let artist = &args[1];
    let pattern = &args[2];

    let regex = match Regex::new(pattern) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("❌ Invalid regex pattern '{pattern}': {e}");
            std::process::exit(1);
        }
    };

    let client = common::setup_client().await?;

    println!("=== Artist Catalog Cleanup Tool ===\n");
    println!("🎨 ARTIST: {artist}");
    println!("🔍 PATTERN: {pattern}");
    println!("📝 This will clean track names by removing text matching the regex pattern");
    println!("\n🚀 Starting catalog scan...\n");

    // Track statistics
    let mut total_tracks_scanned = 0;
    let mut matching_tracks_found = 0;
    let mut tracks_successfully_cleaned = 0;
    let mut tracks_failed_to_clean = 0;
    let mut already_cleaned_tracks = HashSet::new();

    // Step 1: Collect all matching tracks first
    println!("🔍 Step 1: Scanning entire {artist} catalog for matching tracks...");
    let mut all_matching_tracks = Vec::new();
    let mut page = 1;

    loop {
        match client.get_artist_tracks_page(artist, page).await {
            Ok(track_page) => {
                if track_page.tracks.is_empty() {
                    println!(
                        "📚 Reached end of {artist} catalog - scanned {total_tracks_scanned} tracks total"
                    );
                    break;
                }

                for track in track_page.tracks {
                    total_tracks_scanned += 1;

                    // Print progress every 50 tracks
                    if total_tracks_scanned % 50 == 0 {
                        println!("📖 Scanned {total_tracks_scanned} tracks so far...");
                    }

                    // Check if this track matches our pattern
                    if regex.is_match(&track.name) {
                        let base_name = regex.replace(&track.name, "").to_string();
                        if !already_cleaned_tracks.contains(&base_name) {
                            all_matching_tracks.push(track);
                            already_cleaned_tracks.insert(base_name);
                            matching_tracks_found += 1;
                        }
                    }
                }

                if !track_page.has_next_page {
                    println!(
                        "📚 Reached end of {artist} catalog - scanned {total_tracks_scanned} tracks total"
                    );
                    break;
                }

                page += 1;
            }
            Err(e) => {
                println!("❌ Error fetching tracks page {page}: {e}");
                break;
            }
        }
    }

    // Step 2: Process all found matching tracks
    if all_matching_tracks.is_empty() {
        println!("\n🎉 No matching tracks found! Your {artist} catalog is already clean.");
        return Ok(());
    }

    println!(
        "\n🎯 Step 2: Processing {} matching tracks...",
        all_matching_tracks.len()
    );
    already_cleaned_tracks.clear(); // Reset for actual processing

    for (index, track) in all_matching_tracks.iter().enumerate() {
        let clean_name = regex.replace(&track.name, "").to_string();

        println!(
            "\n🎵 [{}/{}] Cleaning: '{}' → '{}'",
            index + 1,
            all_matching_tracks.len(),
            track.name,
            clean_name
        );

        // Skip if we've already processed this track name in this session
        if already_cleaned_tracks.contains(&clean_name) {
            println!("   ⏭️  Skipping - already processed in this session");
            continue;
        }

        // Load real edit form values from the track page
        let edit_template = lastfm_edit::ScrobbleEdit::from_track_and_artist(&track.name, artist);
        match client
            .discover_scrobble_edit_variations(&edit_template)
            .await
        {
            Ok(exact_edit_vec) => {
                if let Some(exact_edit) = exact_edit_vec.into_iter().next() {
                    let mut edit_data = exact_edit.to_scrobble_edit();
                    println!(
                        "   📋 Loaded edit form data - Album: '{}'",
                        edit_data
                            .album_name_original
                            .as_deref()
                            .unwrap_or("unknown")
                    );

                    // Update the track name to the cleaned version
                    edit_data.track_name = Some(clean_name.clone());

                    println!("   🔧 Submitting edit...");

                    // Perform the edit
                    match client.edit_scrobble(&edit_data).await {
                        Ok(_response) => {
                            println!("   ✅ Successfully cleaned: '{clean_name}'");
                            tracks_successfully_cleaned += 1;
                            already_cleaned_tracks.insert(clean_name);
                        }
                        Err(e) => {
                            println!("   ❌ Error editing '{}': {}", track.name, e);
                            tracks_failed_to_clean += 1;
                        }
                    }
                } else {
                    println!("   ⚠️  No edit data found for track");
                    tracks_failed_to_clean += 1;
                }
            }
            Err(e) => {
                println!("   ⚠️  Couldn't load edit form for '{}': {}", track.name, e);
                println!("      This track might not be in your recent scrobbles");
                tracks_failed_to_clean += 1;
            }
        }

        // Add a small delay to be respectful to Last.fm servers
        println!("   ⏳ Waiting 1.2s before next track...");
        tokio::time::sleep(tokio::time::Duration::from_millis(1200)).await;
    }

    // Print final statistics
    println!("\n{}", "=".repeat(60));
    println!("🎼 {} CATALOG CLEANUP COMPLETE", artist.to_uppercase());
    println!("{}", "=".repeat(60));
    println!("📊 STATISTICS:");
    println!("   • Total tracks scanned: {total_tracks_scanned}");
    println!("   • Matching tracks found: {matching_tracks_found}");
    println!("   • Tracks successfully cleaned: {tracks_successfully_cleaned}");
    println!("   • Tracks failed to clean: {tracks_failed_to_clean}");

    if tracks_successfully_cleaned > 0 {
        println!(
            "\n✨ Your {artist} catalog is now cleaner! Pattern '{pattern}' has been removed from track names."
        );
    }

    if tracks_failed_to_clean > 0 {
        println!("\n⚠️  Some tracks couldn't be cleaned. This usually happens when:");
        println!("   • The track hasn't been scrobbled recently");
        println!("   • The track data isn't in your listening history");
        println!("   • There were temporary server issues");
        println!("\n💡 You can re-run this script later to try cleaning the remaining tracks.");
    }

    println!("\n🎵 {artist} catalog cleanup completed!");

    Ok(())
}
