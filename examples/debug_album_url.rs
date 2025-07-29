#[path = "shared/common.rs"]
mod common;

use lastfm_edit::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let client = common::setup_client().await?;

    let artist = "Radiohead";
    let album = "In Rainbows";

    println!("=== Debug Album URL Test ===\n");
    println!("🔍 Testing URL construction for album tracks...");
    println!("Artist: {artist}");
    println!("Album: {album}\n");

    // First, let's see if the artist has albums at all
    println!("1. Testing artist albums page...");
    match client.get_artist_albums_page(artist, 1).await {
        Ok(albums_page) => {
            println!("✅ Found {} albums for {artist}", albums_page.albums.len());
            for (i, album_item) in albums_page.albums.iter().enumerate().take(5) {
                println!(
                    "   [{i}] '{}' ({} plays)",
                    album_item.name, album_item.playcount
                );
            }
        }
        Err(e) => {
            println!("❌ Error getting albums: {e}");
            return Ok(());
        }
    }

    println!("\n2. Testing album tracks page...");
    match client.get_album_tracks_page(album, artist, 1).await {
        Ok(tracks_page) => {
            println!(
                "✅ Album tracks page returned {} tracks",
                tracks_page.tracks.len()
            );
            println!("   Has next page: {}", tracks_page.has_next_page);
            println!("   Total pages: {:?}", tracks_page.total_pages);
            for (i, track) in tracks_page.tracks.iter().enumerate().take(10) {
                println!(
                    "   [{i}] '{}' - Album: '{}'",
                    track.name,
                    track.album.as_deref().unwrap_or("(none)")
                );
            }
        }
        Err(e) => {
            println!("❌ Error getting album tracks: {e}");
        }
    }

    // Let's also test with an album we know exists from the albums list
    println!("\n3. Testing with first album from albums list...");
    match client.get_artist_albums_page(artist, 1).await {
        Ok(albums_page) => {
            if let Some(first_album) = albums_page.albums.first() {
                println!("Testing with album: '{}'", first_album.name);
                match client
                    .get_album_tracks_page(&first_album.name, artist, 1)
                    .await
                {
                    Ok(tracks_page) => {
                        println!(
                            "✅ Found {} tracks for '{}'",
                            tracks_page.tracks.len(),
                            first_album.name
                        );
                        for (i, track) in tracks_page.tracks.iter().enumerate().take(5) {
                            println!("   [{i}] '{}'", track.name);
                        }
                    }
                    Err(e) => {
                        println!("❌ Error: {e}");
                    }
                }
            } else {
                println!("No albums found in list");
            }
        }
        Err(e) => {
            println!("❌ Error getting albums: {e}");
        }
    }

    Ok(())
}
