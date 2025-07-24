use lastfm_edit::{LastFmEditClientImpl, ScrobbleEdit};
use std::env;

/// Example demonstrating the new ScrobbleEdit enrichment functionality.
///
/// This example shows how you can create a ScrobbleEdit with just track and artist names,
/// and the client will automatically discover all the unique album variations for that track
/// from your library, handling pagination as needed.
///
/// Run with: direnv exec . cargo run --example enrich_metadata_example -- "Artist Name" "Track Name"
/// Example: direnv exec . cargo run --example enrich_metadata_example -- "Jimi Hendrix" "Lover Man"
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: {} <artist_name> <track_name>", args[0]);
        eprintln!("Example: {} \"Jimi Hendrix\" \"Lover Man\"", args[0]);
        std::process::exit(1);
    }

    let artist_name = &args[1];
    let track_name = &args[2];

    let username = env::var("LASTFM_EDIT_USERNAME")
        .expect("LASTFM_EDIT_USERNAME environment variable not set");
    let password = env::var("LASTFM_EDIT_PASSWORD")
        .expect("LASTFM_EDIT_PASSWORD environment variable not set");

    // Create and authenticate client
    let http_client = http_client::native::NativeClient::new();
    let client = LastFmEditClientImpl::new(Box::new(http_client));

    println!("🔐 Logging in to Last.fm...");
    client.login(&username, &password).await?;
    println!("✅ Successfully authenticated as {}", client.username());

    // Example 1: Create a ScrobbleEdit with just track and artist names
    // The system will automatically find all album variations
    println!("\n📀 Testing metadata enrichment for '{artist_name}' - '{track_name}'");

    let basic_edit = ScrobbleEdit::from_track_and_artist(track_name, artist_name);

    println!("Created basic ScrobbleEdit:");
    println!("  Track: {}", basic_edit.track_name_original);
    println!("  Artist: {}", basic_edit.artist_name_original);
    println!("  Album: {:?}", basic_edit.album_name_original);
    println!("  Timestamp: {:?}", basic_edit.timestamp);

    // Use discover_album_variations to discover all album variations
    println!("\n🔍 Discovering album variations from library...");
    match client
        .discover_album_variations(track_name, artist_name)
        .await
    {
        Ok(scrobble_edits) => {
            println!("✅ Found {} unique album variations:", scrobble_edits.len());

            for (i, edit) in scrobble_edits.iter().enumerate() {
                println!(
                    "  {}. Album: '{}', Album Artist: '{}', Timestamp: {:?}",
                    i + 1,
                    edit.album_name_original.as_deref().unwrap_or("Unknown"),
                    edit.album_artist_name_original
                        .as_deref()
                        .unwrap_or("Unknown"),
                    edit.timestamp
                );
            }

            // Example 2: Show how edit_scrobble would work with multiple edits
            println!("\\n🎭 Testing how edit_scrobble handles multiple album variations...");
            println!("When you call edit_scrobble with minimal info (just track/artist),");
            println!("it now performs edits on ALL album variations automatically!");

            // Show what would happen if we made an edit
            let minimal_edit = ScrobbleEdit::from_track_and_artist(track_name, artist_name)
                .with_track_name(&format!("{track_name} (Updated)"));

            println!(
                "\\nIf we called edit_scrobble to rename '{}' to '{}':",
                track_name, minimal_edit.track_name
            );
            println!("The system would:");
            for (i, edit) in scrobble_edits.iter().enumerate() {
                println!(
                    "  {}. Edit '{}' on album '{}' (timestamp: {:?})",
                    i + 1,
                    track_name,
                    edit.album_name_original.as_deref().unwrap_or("Unknown"),
                    edit.timestamp
                );
            }
            println!("\\n✨ This ensures consistency across all your scrobbles of this track!");
        }
        Err(e) => {
            println!("❌ Could not discover album variations: {e}");
            println!("This might mean:");
            println!("  - The track is not in your recent scrobbles");
            println!("  - The track/artist names don't match exactly");
            println!("  - There's a network or parsing issue");
        }
    }

    Ok(())
}
