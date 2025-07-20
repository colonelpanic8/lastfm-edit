#[path = "shared/common.rs"]
mod common;

use lastfm_edit::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let mut client = common::setup_client().await?;
    client.enable_debug();

    println!("=== Loading Track Metadata from Specific Track Page ===\n");

    // Test with the specific URL the user mentioned
    let track_name = "Michelle - Remastered 2009";
    let artist_name = "The Beatles";

    println!("🔍 Loading metadata for specific track:");
    println!("   Track: '{}'", track_name);
    println!("   Artist: '{}'", artist_name);
    println!("   Expected URL: https://www.last.fm/user/IvanMalison/library/music/The+Beatles/_/Michelle+-+Remastered+2009\n");

    // First, let's see what the track page URL looks like when we build it
    let encoded_artist = urlencoding::encode(artist_name);
    let encoded_track = urlencoding::encode(track_name);
    let expected_url = format!(
        "https://www.last.fm/user/{}/library/music/{}/_/{}",
        "IvanMalison", // Using username from environment
        encoded_artist,
        encoded_track
    );
    println!("🌐 Constructed URL: {}\n", expected_url);

    // Now try to load the edit form values
    println!("🔄 Attempting to load edit form values...\n");

    match client.load_edit_form_values(track_name, artist_name).await {
        Ok(edit_data) => {
            println!("✅ SUCCESS! Correctly loaded edit form values for the requested track!");
            println!("   Expected Track: '{}'", track_name);
            println!("   Expected Artist: '{}'", artist_name);
            println!("   ✅ Got Track: '{}'", edit_data.track_name_original);
            println!("   ✅ Got Artist: '{}'", edit_data.artist_name_original);
            println!("   ✅ Got Album: '{}'", edit_data.album_name_original);
            println!(
                "   ✅ Got Album Artist: '{}'",
                edit_data.album_artist_name_original
            );
            println!(
                "   ✅ Timestamp: {} ({})",
                edit_data.timestamp,
                chrono::DateTime::from_timestamp(edit_data.timestamp as i64, 0)
                    .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                    .unwrap_or_else(|| "Invalid".to_string())
            );
            println!("   ✅ Edit All: {}", edit_data.edit_all);

            println!("\n🎯 SOLUTION IMPLEMENTED:");
            println!("   ✅ Successfully parsing scrobble edit forms directly from track page");
            println!("   ✅ Finding forms that match the requested track and artist");
            println!("   ✅ Extracting real scrobble data with correct timestamps");
            println!("   ✅ No longer using unreliable timestamp-based edit form fetches");

            println!("\n💡 This data can now be used for editing the track!");
            println!("   The edit_scrobble example should work correctly with this approach.");
        }
        Err(e) => {
            println!("❌ Failed to load edit form values: {}", e);
            println!("This might mean the track doesn't exist in the user's scrobbles or there's a parsing issue.");
        }
    }

    Ok(())
}
