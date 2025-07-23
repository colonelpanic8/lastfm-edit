#[path = "shared/common.rs"]
mod common;

use lastfm_edit::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let client = common::setup_client().await?;

    let artist = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "The Beatles".to_string());

    println!("=== Artist Tracks Listing ===\n");
    println!("🎵 Listing all tracks for artist: {artist}\n");

    let mut track_count = 0;
    let mut page = 1;

    println!("🔍 Fetching tracks...\n");

    loop {
        match client.get_artist_tracks_page(&artist, page).await {
            Ok(track_page) => {
                if track_page.tracks.is_empty() {
                    println!("\n📚 Reached end of {artist} catalog");
                    break;
                }

                for track in track_page.tracks {
                    track_count += 1;
                    println!(
                        "[{:4}] '{}' (plays: {})",
                        track_count, track.name, track.playcount
                    );
                }

                if !track_page.has_next_page {
                    println!("\n📚 Reached end of {artist} catalog");
                    break;
                }

                page += 1;
            }
            Err(e) => {
                println!("❌ Error fetching tracks page {page}: {e}");
                break;
            }
        }
    }

    println!("\n=== Summary ===");
    println!("📊 Total tracks: {track_count}");

    Ok(())
}
