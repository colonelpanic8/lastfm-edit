#[path = "shared/common.rs"]
mod common;

use lastfm_edit::{AsyncPaginatedIterator, LastFmEditClient, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let client = common::setup_client().await?;

    let artist = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "The Beatles".to_string());
    let album = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "Abbey Road".to_string());

    println!("=== Album Tracks Test ===\n");
    println!("🎵 Testing get_album_tracks() with: '{album}' by '{artist}'\n");

    // Test the fixed get_album_tracks method
    match client.get_album_tracks(&album, &artist).await {
        Ok(tracks) => {
            println!("✅ SUCCESS: Got {} tracks", tracks.len());
            if tracks.is_empty() {
                println!("   (Album not found in your library, but no crash!)");
            } else {
                println!("   Tracks:");
                for (i, track) in tracks.iter().enumerate().take(10) {
                    println!("   [{:2}] {}", i + 1, track.name);
                }
                if tracks.len() > 10 {
                    println!("   ... and {} more tracks", tracks.len() - 10);
                }
            }
        }
        Err(e) => {
            println!("❌ ERROR: {e}");
        }
    }

    // Also test the iterator directly
    println!("\n=== Album Tracks Iterator Test ===");
    let mut tracks_iterator = client.album_tracks(&album, &artist);
    let mut count = 0;

    println!("🔍 Testing iterator...");
    while let Some(track) = tracks_iterator.next().await? {
        count += 1;
        if count <= 5 {
            println!("   [{count}] {}", track.name);
        }
        if count >= 5 {
            break;
        }
    }

    if count == 0 {
        println!("   No tracks found via iterator (album not in library)");
    } else {
        println!("   Iterator works - got {count} tracks");
    }

    println!("\n🎉 Both methods completed without crashing!");
    Ok(())
}
