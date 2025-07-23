#[path = "shared/common.rs"]
mod common;

use lastfm_edit::{Result, Track};
use std::collections::HashMap;

#[tokio::main]
async fn main() -> Result<()> {
    let client = common::setup_client().await?;

    let artist = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "The Beatles".to_string());

    println!("=== Artist Tracks by Album ===\n");
    println!("🎵 Listing tracks for artist: {artist} grouped by album\n");

    // First, get all albums by the artist
    let mut albums = Vec::new();
    let mut page = 1;

    println!("🔍 Fetching albums...\n");

    loop {
        match client.get_artist_albums_page(&artist, page).await {
            Ok(album_page) => {
                if album_page.albums.is_empty() {
                    break;
                }

                albums.extend(album_page.albums);

                if !album_page.has_next_page {
                    break;
                }

                page += 1;
            }
            Err(e) => {
                println!("❌ Error fetching albums page {page}: {e}");
                break;
            }
        }
    }

    if albums.is_empty() {
        println!("❌ No albums found for artist '{artist}'");
        return Ok(());
    }

    println!("📚 Found {} albums\n", albums.len());

    // Group tracks by album
    let mut tracks_by_album: HashMap<String, Vec<Track>> = HashMap::new();
    let mut album_playcounts: HashMap<String, u32> = HashMap::new();

    for album in &albums {
        println!("🔍 Fetching tracks for album: {}", album.name);

        match client.get_album_tracks(&album.name, &artist).await {
            Ok(tracks) => {
                let total_plays: u32 = tracks.iter().map(|t| t.playcount).sum();

                tracks_by_album.insert(album.name.clone(), tracks.clone());
                album_playcounts.insert(album.name.clone(), total_plays);

                println!(
                    "   ✅ {} tracks ({} total plays)",
                    tracks.len(),
                    total_plays
                );
            }
            Err(e) => {
                println!("   ❌ Error fetching tracks: {e}");
                tracks_by_album.insert(album.name.clone(), Vec::new());
                album_playcounts.insert(album.name.clone(), 0);
            }
        }
    }

    // Display results organized by album
    println!("\n=== Tracks by Album ===\n");

    for album in &albums {
        let tracks = tracks_by_album.get(&album.name).unwrap();
        let total_plays = album_playcounts.get(&album.name).unwrap_or(&0);

        println!("💿 {} ({} plays)", album.name, total_plays);

        if tracks.is_empty() {
            println!("   📭 No tracks found");
        } else {
            for (index, track) in tracks.iter().enumerate() {
                let album_info = track
                    .album
                    .as_ref()
                    .map(|a| format!(" [Album: {a}]"))
                    .unwrap_or_default();
                println!(
                    "   {}. {} ({} plays){}",
                    index + 1,
                    track.name,
                    track.playcount,
                    album_info
                );
            }
        }
        println!();
    }

    // Summary
    let total_albums = albums.len();
    let total_tracks: usize = tracks_by_album.values().map(|tracks| tracks.len()).sum();
    let total_plays: u32 = album_playcounts.values().sum();

    println!("=== Summary ===");
    println!("📊 Artist: {artist}");
    println!("📚 Albums: {total_albums}");
    println!("🎵 Total tracks: {total_tracks}");
    println!("▶️ Total plays: {total_plays}");

    Ok(())
}
