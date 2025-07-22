#[path = "shared/common.rs"]
mod common;

use lastfm_edit::Result;
use std::env;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();

    if args.len() != 3 {
        eprintln!("Usage: cargo run --example list_album_tracks -- \"Album Name\" \"Artist Name\"");
        eprintln!(
            "Example: cargo run --example list_album_tracks -- \"Abbey Road\" \"The Beatles\""
        );
        std::process::exit(1);
    }

    let album = &args[1];
    let artist = &args[2];
    let client = common::setup_client().await?;

    println!("💿 All tracks from '{album}' by {artist}:\n");

    // Get tracks from the album page (single request)
    match client.get_album_tracks(album, artist).await {
        Ok(tracks) => {
            if tracks.is_empty() {
                println!("❌ No tracks found for album '{album}' by '{artist}'");
                println!("\n💡 This might mean:");
                println!("   • The album name doesn't match exactly as it appears on Last.fm");
                println!("   • The album doesn't exist in your library");
                println!("   • The artist name is incorrect");
                return Ok(());
            }

            println!("✅ Found {} tracks:\n", tracks.len());

            for (index, track) in tracks.iter().enumerate() {
                println!("{}. {} ({} plays)", index + 1, track.name, track.playcount);
            }

            println!("\n📊 Total: {} tracks from '{}'", tracks.len(), album);
        }
        Err(e) => {
            println!("❌ Error loading album tracks: {e}");
            println!("\n💡 This might happen if:");
            println!("   • The album doesn't exist in your library");
            println!("   • There are network issues");
            println!("   • The album/artist names don't match exactly");
        }
    }

    Ok(())
}
