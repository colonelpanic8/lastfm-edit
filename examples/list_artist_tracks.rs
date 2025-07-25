#[path = "shared/common.rs"]
mod common;

use lastfm_edit::{AsyncPaginatedIterator, LastFmEditClient, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let client = common::setup_client().await?;

    let artist = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "Queen".to_string());

    println!("=== Artist Tracks Listing (using Iterator) ===\n");
    println!("🎵 Listing all tracks for artist: {artist}\n");

    // Use the iterator the same way as Case 4: Artist-specific discovery
    let mut tracks_iterator = client.artist_tracks(&artist);
    let mut track_count = 0;

    println!("🔍 Fetching tracks using iterator...\n");

    while let Some(track) = tracks_iterator.next().await? {
        track_count += 1;
        println!(
            "[{:4}] '{}' | Album: '{}' | Plays: {} | Timestamp: {:?}",
            track_count,
            track.name,
            track.album.as_deref().unwrap_or("(no album)"),
            track.playcount,
            track.timestamp
        );

        // Limit output for testing to avoid overwhelming output
        if track_count >= 50 {
            println!("\n⚠️  Limiting output to first 50 tracks for testing...");
            break;
        }
    }

    println!("\n=== Summary ===");
    println!("📊 Total tracks displayed: {track_count}");

    if let Some(total_pages) = tracks_iterator.total_pages() {
        println!("📄 Total pages available: {total_pages}");
    }

    Ok(())
}
