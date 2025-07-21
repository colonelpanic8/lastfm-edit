#[path = "shared/common.rs"]
mod common;

use lastfm_edit::{AsyncPaginatedIterator, Result};
use std::env;

#[tokio::main]
async fn main() -> Result<()> {
    let mut client = common::setup_client().await?;

    // Parse command line argument for number of tracks (default 20)
    let num_tracks: usize = env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);

    println!("Fetching {num_tracks} recent tracks...");
    println!();

    // Get iterator for recent tracks
    let mut recent_tracks = client.recent_tracks();

    // Fetch and print tracks one by one
    let mut count = 0;
    while count < num_tracks {
        match recent_tracks.next().await? {
            Some(track) => {
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
            None => {
                println!("No more tracks available.");
                break;
            }
        }
    }

    println!();
    println!("Fetched {count} tracks total.");

    Ok(())
}
