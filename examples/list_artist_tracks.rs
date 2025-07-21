#[path = "shared/common.rs"]
mod common;

use lastfm_edit::{AsyncPaginatedIterator, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let mut client = common::setup_client().await?;

    let artist = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "The Beatles".to_string());

    println!("=== Artist Tracks Listing ===\n");
    println!("🎵 Listing all tracks for artist: {artist}\n");

    let mut iterator = client.artist_tracks(&artist);
    let mut track_count = 0;

    println!("🔍 Fetching tracks...\n");

    loop {
        match iterator.next().await {
            Ok(Some(track)) => {
                track_count += 1;
                println!(
                    "[{:4}] '{}' (plays: {})",
                    track_count, track.name, track.playcount
                );
            }
            Ok(None) => {
                println!("\n📚 Reached end of {artist} catalog");
                break;
            }
            Err(e) => {
                println!("❌ Error fetching tracks: {e}");
                break;
            }
        }
    }

    println!("\n=== Summary ===");
    println!("📊 Total tracks: {track_count}");

    Ok(())
}
