#[path = "shared/common.rs"]
mod common;

use lastfm_edit::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let mut client = common::setup_client().await?;
    client.enable_debug();

    println!("=== Testing Edit Form Loading ===\n");

    // Test loading edit form values for a track
    let test_track = "Fish And Bird"; // This was in recent scrobbles
    let test_artist = "Noam Pikelny";

    println!(
        "🔍 Testing edit form loading for '{}' by '{}'...",
        test_track, test_artist
    );

    match client.load_edit_form_values(test_track, test_artist).await {
        Ok(edit_data) => {
            println!("✅ Successfully loaded edit form values!");
            println!("   Original Track: '{}'", edit_data.track_name_original);
            println!("   Original Album: '{}'", edit_data.album_name_original);
            println!("   Original Artist: '{}'", edit_data.artist_name_original);
            println!(
                "   Original Album Artist: '{}'",
                edit_data.album_artist_name_original
            );
            println!("   Timestamp: {}", edit_data.timestamp);
            println!("   Edit All: {}", edit_data.edit_all);

            println!("\n🎯 The TUI would now allow editing these values!");
            println!(
                "   Current track name that can be edited: '{}'",
                edit_data.track_name
            );
        }
        Err(e) => {
            println!("❌ Failed to load edit form values: {}", e);
            println!("This might be because the track isn't in recent scrobbles or there was another issue.");
        }
    }

    Ok(())
}
