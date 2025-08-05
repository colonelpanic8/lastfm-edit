#[path = "shared/common.rs"]
mod common;

use lastfm_edit::{LastFmEditClient, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let client = common::setup_client().await?;

    let artist = "The Beatles";

    println!("=== Artist Tracks Direct Example ===\n");
    println!("🎵 Comparing album-based vs direct approach for: {artist}");

    // Test the direct approach
    println!("\n📄 Using direct paginated endpoint:");
    let mut direct_tracks = client.artist_tracks_direct(artist);
    let direct_tracks_list = direct_tracks.take(10).await?;

    println!(
        "✅ Found {} tracks using direct approach:",
        direct_tracks_list.len()
    );
    for (i, track) in direct_tracks_list.iter().enumerate() {
        println!(
            "  {}. {} (played {} times)",
            i + 1,
            track.name,
            track.playcount
        );
        if let Some(album) = &track.album {
            println!("     Album: {album}");
        }
    }

    // Compare with album-based approach
    println!("\n📀 Using album-based approach:");
    let mut album_tracks = client.artist_tracks(artist);
    let album_tracks_list = album_tracks.take(10).await?;

    println!(
        "✅ Found {} tracks using album-based approach:",
        album_tracks_list.len()
    );
    for (i, track) in album_tracks_list.iter().enumerate() {
        println!(
            "  {}. {} (played {} times)",
            i + 1,
            track.name,
            track.playcount
        );
        if let Some(album) = &track.album {
            println!("     Album: {album}");
        }
    }

    println!("\n💡 The direct approach is more efficient as it uses:");
    println!(
        "   /user/{{username}}/library/music/{}/+tracks?page=N&ajax=true",
        artist.replace(" ", "+")
    );
    println!("   Instead of iterating through albums first.");

    Ok(())
}
