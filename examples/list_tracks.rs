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
    let mut client = common::setup_client().await?;

    println!("🎵 Tracks by {}:\n", artist);

    let mut iterator = client.artist_tracks(artist);
    let mut count = 0;

    while let Some(track) = iterator.next().await? {
        count += 1;
        println!("{}. {} ({} plays)", count, track.name, track.playcount);
    }

    println!("\n📊 Total: {} tracks", count);

    Ok(())
}
