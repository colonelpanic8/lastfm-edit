#[path = "shared/common.rs"]
mod common;

use lastfm_edit::Result;
use std::env;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();

    if args.len() != 3 {
        eprintln!("Usage: cargo run --example list_album_tracks -- \"Artist Name\" \"Album Name\"");
        eprintln!("Example: cargo run --example list_album_tracks -- \"The Beatles\" \"Abbey Road\"");
        std::process::exit(1);
    }

    let artist = &args[1];
    let album = &args[2];
    let mut client = common::setup_client().await?;

    println!("💿 Tracks from '{}' by {}:\n", album, artist);

    // Get all editable tracks from this album
    match client.get_album_tracks_for_editing(album, artist).await {
        Ok(editable_tracks) => {
            if editable_tracks.is_empty() {
                println!("❌ No editable tracks found for album '{}' by '{}'", album, artist);
                println!("\n💡 This usually means:");
                println!("   • No tracks from this album have been scrobbled recently");
                println!("   • The album name doesn't match exactly as it appears on Last.fm");
                println!("   • The tracks aren't in your recent listening history");
                println!("\n🔍 Try:");
                println!("   • Check the exact album name on Last.fm");
                println!("   • Scrobble a track from this album to make it appear in recent history");
                return Ok(());
            }

            println!("✅ Found {} editable tracks from this album:\n", editable_tracks.len());

            for (index, track_data) in editable_tracks.iter().enumerate() {
                println!("{}. {}", index + 1, track_data.track_name_original);
                println!("   Album: {}", track_data.album_name_original);
                println!("   Artist: {}", track_data.artist_name_original);
                println!("   Scrobble timestamp: {}", track_data.timestamp);
                if index < editable_tracks.len() - 1 {
                    println!();
                }
            }

            println!("\n📊 Summary:");
            println!("   • {} tracks can be edited from this album", editable_tracks.len());
            println!("   • Each track represents a scrobble that's in your recent listening history");
            println!("   • To rename the album, all {} tracks would be updated", editable_tracks.len());
        }
        Err(e) => {
            println!("❌ Error loading tracks from album: {}", e);
            println!("\n💡 This might happen if:");
            println!("   • The album doesn't exist in your library");
            println!("   • There are network issues");
            println!("   • The album/artist names don't match exactly");
        }
    }

    Ok(())
}