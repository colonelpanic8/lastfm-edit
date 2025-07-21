#[path = "shared/common.rs"]
mod common;

use lastfm_edit::AsyncPaginatedIterator;

use lastfm_edit::Result;
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

    let mut client = common::setup_client().await?;

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

    {
        let mut iterator = client.artist_tracks(artist);
        let mut track_count = 0;

        loop {
            match iterator.next().await {
                Ok(Some(track)) => {
                    total_tracks_scanned += 1;
                    track_count += 1;

                    // Print progress every 50 tracks
                    if track_count % 50 == 0 {
                        println!("📖 Scanned {track_count} tracks so far...");
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
                Ok(None) => {
                    println!(
                        "📚 Reached end of {artist} catalog - scanned {track_count} tracks total"
                    );
                    break;
                }
                Err(e) => {
                    println!("❌ Error fetching tracks: {e}");
                    break;
                }
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
