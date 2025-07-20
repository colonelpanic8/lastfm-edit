#[path = "shared/common.rs"]
mod common;

use lastfm_edit::{Result, ScrobbleEdit};

#[tokio::main]
async fn main() -> Result<()> {
    let mut client = common::setup_client().await?;
    client.enable_debug();

    println!("=== Testing Minimal Edit Parameters ===\n");

    // Test with minimal parameters - just what we know works
    let edit = ScrobbleEdit::from_track_info(
        "Test Track Name",
        "Test Album Name", 
        "Test Artist Name",
        0, // No timestamp
    )
    .with_track_name("New Test Track Name")
    .with_edit_all(true);

    println!("Edit request details:");
    println!("  Original track: {}", edit.track_name_original);
    println!("  New track: {}", edit.track_name);
    println!("  Artist: {}", edit.artist_name);
    println!("  Edit all: {}", edit.edit_all);

    println!("\n🔄 Performing minimal edit test...");

    match client.edit_scrobble(&edit).await {
        Ok(response) => {
            if response.success {
                println!("✅ Edit successful!");
            } else {
                println!("❌ Edit failed!");
                if let Some(message) = response.message {
                    println!("Response message: {}", message);
                }
            }
        }
        Err(e) => {
            println!("❌ Error performing edit: {}", e);
        }
    }

    Ok(())
}