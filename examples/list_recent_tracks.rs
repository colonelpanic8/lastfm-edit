#[path = "shared/common.rs"]
mod common;

use lastfm_edit::Result;
use std::env;

#[tokio::main]
async fn main() -> Result<()> {
    let client = common::setup_client().await?;

    // Parse command line arguments
    let args: Vec<String> = env::args().collect();
    let num_tracks: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(20);
    let starting_page: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);

    println!("Fetching {num_tracks} recent tracks starting from page {starting_page}...");
    println!();

    let mut count = 0;
    let mut page = starting_page;

    // Fetch tracks using pagination directly
    while count < num_tracks {
        match client.get_recent_scrobbles(page).await {
            Ok(tracks) => {
                if tracks.is_empty() {
                    println!("No more tracks available.");
                    break;
                }

                for track in tracks {
                    if count >= num_tracks {
                        break;
                    }

                    let timestamp_str = if let Some(ts) = track.timestamp {
                        format!(
                            " ({})",
                            chrono::DateTime::from_timestamp(ts as i64, 0)
                                .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                                .unwrap_or_else(|| format!("timestamp: {ts}"))
                        )
                    } else {
                        " (no timestamp)".to_string()
                    };

                    let album_str = if let Some(album) = &track.album {
                        format!(" [{album}]")
                    } else {
                        "".to_string()
                    };

                    println!(
                        "{}. {} - {}{}{}",
                        count + 1,
                        track.artist,
                        track.name,
                        album_str,
                        timestamp_str
                    );

                    count += 1;
                }

                page += 1;
            }
            Err(e) => {
                println!("❌ Error fetching tracks from page {page}: {e}");
                break;
            }
        }
    }

    println!();
    println!("Fetched {count} tracks total.");

    println!();
    println!("Usage: cargo run --example list_recent_tracks [num_tracks] [starting_page]");
    println!("  num_tracks    - Number of tracks to fetch (default: 20)");
    println!("  starting_page - Page number to start from (default: 1)");
    println!();
    println!("Examples:");
    println!("  cargo run --example list_recent_tracks 50     # Fetch 50 tracks from page 1");
    println!(
        "  cargo run --example list_recent_tracks 20 5   # Fetch 20 tracks starting from page 5"
    );

    Ok(())
}
