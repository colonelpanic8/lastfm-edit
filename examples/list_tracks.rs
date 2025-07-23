#[path = "shared/common.rs"]
mod common;

use lastfm_edit::Result;
use std::env;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();

    if args.len() != 2 {
        eprintln!("Usage: cargo run --example list_tracks -- \"Artist Name\"");
        eprintln!("Example: cargo run --example list_tracks -- \"The Beatles\"");
        std::process::exit(1);
    }

    let artist = &args[1];
    let client = common::setup_client().await?;

    println!("🎵 Tracks by {artist}:\n");

    let mut count = 0;
    let mut page = 1;

    loop {
        match client.get_artist_tracks_page(artist, page).await {
            Ok(track_page) => {
                if track_page.tracks.is_empty() {
                    break;
                }

                for track in track_page.tracks {
                    count += 1;
                    println!("{}. {} ({} plays)", count, track.name, track.playcount);
                }

                if !track_page.has_next_page {
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

    println!("\n📊 Total: {count} tracks");

    Ok(())
}
