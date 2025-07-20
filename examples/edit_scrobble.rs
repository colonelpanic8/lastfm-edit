#[path = "shared/common.rs"]
mod common;

use lastfm_edit::{Result, ScrobbleEdit};
use regex::Regex;

#[tokio::main]
async fn main() -> Result<()> {
    let mut client = common::setup_client().await?;

    // Enable debug logging to see what's happening
    client.enable_debug();

    println!("=== Beatles Track Cleanup: Remove Remastered Suffix ===\n");

    let artist = "The Beatles";
    println!("🔍 Searching for Beatles tracks with 'Remastered' in the title...\n");

    // Search through Beatles tracks to find one with "Remastered" in the name
    let mut iterator = client.artist_tracks(artist);
    let regex = Regex::new(r" - Remastered( \d{4})?$").unwrap();

    let mut found_track = None;
    let mut checked_count = 0;

    // Check first few pages to find a remastered track
    for page_num in 1..=3 {
        match iterator.next_page().await {
            Ok(Some(page)) => {
                for track in &page.tracks {
                    checked_count += 1;
                    if regex.is_match(&track.name) {
                        found_track = Some(track.clone());
                        println!("✅ Found remastered track: '{}'", track.name);
                        println!(
                            "   Playcount: {}, Has timestamp: {}",
                            track.playcount,
                            if track.timestamp.is_some() {
                                "yes"
                            } else {
                                "no"
                            }
                        );
                        break;
                    }
                }
                if found_track.is_some() {
                    break;
                }
                if !page.has_next_page {
                    break;
                }
            }
            Ok(None) => {
                // No more pages
                break;
            }
            Err(e) => {
                println!("❌ Error fetching tracks: {}", e);
                return Err(e);
            }
        }
    }

    let track = match found_track {
        Some(t) => t,
        None => {
            println!(
                "❌ No remastered tracks found in first {} tracks",
                checked_count
            );
            println!("You might need to check more pages or the tracks may already be cleaned up.");
            return Ok(());
        }
    };

    // Extract clean track name (remove "- Remastered YYYY" suffix)
    let clean_name = regex.replace(&track.name, "").to_string();

    println!("\n🎵 Track Edit Details:");
    println!("  📀 Track:    '{}' → '{}'", track.name, clean_name);
    println!("  🎤 Artist:   '{}'", artist);

    // Load real edit form values from Last.fm to get proper scrobble data
    println!("\n🔄 Loading real edit form values from Last.fm...");
    let mut edit = match client.load_edit_form_values(&track.name, artist).await {
        Ok(edit_data) => {
            println!("✅ Successfully loaded edit form values!");
            println!("   Track: '{}'", edit_data.track_name_original);
            println!("   Album: '{}'", edit_data.album_name_original);
            println!(
                "   Timestamp: {} (real scrobble data!)",
                edit_data.timestamp
            );
            edit_data
        }
        Err(e) => {
            println!("❌ Failed to load edit form values: {}", e);
            println!("This track might not be in your recent scrobbles.");
            println!("Try scrobbling this track recently, then run the example again.");
            return Ok(());
        }
    };

    // Update the track name to the cleaned version
    edit.track_name = clean_name.clone();

    println!("\n🔄 Performing edit...");

    match client.edit_scrobble(&edit).await {
        Ok(response) => {
            if response.success {
                println!("✅ Edit successful!");
                println!("Successfully cleaned: '{}' → '{}'", track.name, clean_name);
            } else {
                println!("❌ Edit failed!");
                if let Some(message) = response.message {
                    println!(
                        "Server response (first 500 chars): {}",
                        &message.chars().take(500).collect::<String>()
                    );
                }
            }
        }
        Err(e) => {
            println!("❌ Error performing edit: {}", e);
        }
    }

    Ok(())
}
