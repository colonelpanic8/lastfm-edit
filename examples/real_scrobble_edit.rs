#[path = "shared/common.rs"]
mod common;

use lastfm_edit::{Result, ScrobbleEditContext, IntoEditContext};
use regex::Regex;

#[tokio::main]
async fn main() -> Result<()> {
    let mut client = common::setup_client().await?;
    client.enable_debug();

    println!("=== Real Scrobble Data Edit Test ===\n");

    let artist = "The Beatles";
    
    // First, let's look at recent scrobbles to see what we have
    println!("🔍 Fetching recent scrobbles to see what's available...");
    match client.get_recent_scrobbles(1).await {
        Ok(recent_scrobbles) => {
            println!("✅ Found {} recent scrobbles:", recent_scrobbles.len());
            
            // Show first few scrobbles
            for (i, scrobble) in recent_scrobbles.iter().take(5).enumerate() {
                println!("  {}. '{}' by '{}' - Timestamp: {:?}", 
                    i + 1, scrobble.name, scrobble.artist, scrobble.timestamp);
            }
            
            // Look for a Beatles track to edit
            if let Some(beatles_track) = recent_scrobbles.iter()
                .find(|s| s.artist == artist) {
                
                println!("\n🎵 Found Beatles track in recent scrobbles: '{}'", beatles_track.name);
                
                // Check if it has "Remastered" in the name
                let regex = Regex::new(r" - Remastered( \d{4})?$").unwrap();
                if regex.is_match(&beatles_track.name) {
                    let clean_name = regex.replace(&beatles_track.name, "").to_string();
                    
                    println!("✨ This track has 'Remastered' suffix!");
                    println!("   Original: '{}'", beatles_track.name);
                    println!("   Clean:    '{}'", clean_name);
                    
                    if let Some(timestamp) = beatles_track.timestamp {
                        println!("   Timestamp: {} (real scrobble data!)", timestamp);
                        
                        // Create edit context and perform the edit with real data
                        let edit_context = beatles_track.clone().into_edit_context();
                        
                        println!("\n🔄 Performing edit with real scrobble data...");
                        match edit_context.execute_edit_with_real_data(&mut client, clean_name.clone(), None).await {
                            Ok(true) => {
                                println!("✅ Edit successful with real scrobble data!");
                                println!("Successfully cleaned: '{}' → '{}'", beatles_track.name, clean_name);
                            }
                            Ok(false) => {
                                println!("❌ Edit failed - server returned failure");
                            }
                            Err(e) => {
                                println!("❌ Error performing edit: {}", e);
                            }
                        }
                    } else {
                        println!("❌ No timestamp available for this scrobble");
                    }
                } else {
                    println!("ℹ️ This track doesn't have 'Remastered' suffix, nothing to clean");
                }
            } else {
                println!("ℹ️ No Beatles tracks found in recent scrobbles");
                println!("Try scrobbling a Beatles track first, then run this example again");
            }
        }
        Err(e) => {
            println!("❌ Error fetching recent scrobbles: {}", e);
        }
    }

    // Also demonstrate the search functionality
    println!("\n🔍 Testing search for specific track in recent history...");
    match client.find_recent_scrobble_for_track("Yesterday", "The Beatles", 3).await {
        Ok(Some(scrobble)) => {
            println!("✅ Found 'Yesterday' by The Beatles in recent scrobbles!");
            println!("   Timestamp: {:?}", scrobble.timestamp);
        }
        Ok(None) => {
            println!("ℹ️ 'Yesterday' by The Beatles not found in recent scrobbles");
        }
        Err(e) => {
            println!("❌ Error searching for track: {}", e);
        }
    }

    Ok(())
}