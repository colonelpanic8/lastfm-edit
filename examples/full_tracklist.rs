#[path = "shared/common.rs"]
mod common;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = common::setup_client().await?;

    let artist = "The Beatles";
    println!("\nFetching complete tracklist for {}...\n", artist);

    let mut iterator = client.artist_tracks(artist);
    
    // Collect all tracks
    let all_tracks = iterator.collect_all().await?;

    // Print them numbered
    for (index, track) in all_tracks.iter().enumerate() {
        println!("{}. {} - {} plays", index + 1, track.name, track.playcount);
    }

    println!("\nTotal tracks found: {}", all_tracks.len());
    Ok(())
}
