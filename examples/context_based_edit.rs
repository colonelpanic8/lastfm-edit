#[path = "shared/common.rs"]
mod common;

use lastfm_edit::{IntoEditContext, Result, ScrobbleEditContext};
use regex::Regex;

#[tokio::main]
async fn main() -> Result<()> {
    let mut client = common::setup_client().await?;

    // Enable debug logging to see what's happening
    client.enable_debug();

    println!("=== Context-Based Beatles Track Cleanup ===\n");

    let artist = "The Beatles";
    println!("🔍 Searching for Beatles tracks with 'Remastered' in the title...\n");

    // Search through Beatles tracks to find one with "Remastered" in the name
    let mut iterator = client.artist_tracks(artist);
    let regex = Regex::new(r" - Remastered( \d{4})?$").unwrap();

    let mut found_track = None;
    let mut checked_count = 0;

    // Check first few pages to find a remastered track
    for _page_num in 1..=3 {
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

    // Convert track to edit context - this is the bridge!
    let edit_context = track.into_edit_context();

    // Extract clean track name (remove "- Remastered YYYY" suffix)
    let clean_name = regex.replace(&edit_context.track_name, "").to_string();

    println!("\n🎵 Edit Context Details:");
    println!("  📀 Original: '{}'", edit_context.track_name);
    println!("  ✨ Clean:    '{}'", clean_name);
    println!("  🎤 Artist:   '{}'", edit_context.artist_name);
    println!("  📊 Strategy: {:?}", edit_context.strategy);
    println!(
        "  📝 Description: {}",
        edit_context.describe_edit(&clean_name)
    );

    println!("\n🔄 Performing context-based edit...");

    // Execute the edit using the context
    match edit_context
        .execute_edit(&mut client, clean_name.clone(), None)
        .await
    {
        Ok(true) => {
            println!("✅ Edit successful!");
            println!(
                "Successfully cleaned: '{}' → '{}'",
                edit_context.track_name, clean_name
            );
        }
        Ok(false) => {
            println!("❌ Edit failed - server returned failure");
        }
        Err(e) => {
            println!("❌ Error performing edit: {}", e);
        }
    }

    Ok(())
}
