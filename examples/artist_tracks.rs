use lastfm_edit::{LastFmClient, Result};
use std::env;

#[tokio::main]
async fn main() -> Result<()> {
    let username = env::var("LASTFM_EDIT_USERNAME")
        .expect("LASTFM_EDIT_USERNAME environment variable not set");
    let password = env::var("LASTFM_EDIT_PASSWORD")
        .expect("LASTFM_EDIT_PASSWORD environment variable not set");

    // Create client and login
    let http_client = http_client::native::NativeClient::new();
    let mut client = LastFmClient::new(Box::new(http_client));

    println!("Logging in as {}...", username);
    client.login(&username, &password).await?;
    println!("✓ Logged in successfully");

    // Test artist tracks pagination
    let artist = "The Beatles";
    println!("\nFetching tracks for: {}", artist);

    let mut iterator = client.artist_tracks(artist);

    // Get first page
    println!("\n--- First Page ---");
    if let Some(page) = iterator.next_page().await? {
        println!(
            "Page {}/{:?} - {} tracks",
            page.page_number,
            page.total_pages.unwrap_or(0),
            page.tracks.len()
        );

        for (i, track) in page.tracks.iter().take(5).enumerate() {
            println!("{}. {} - {} plays", i + 1, track.name, track.playcount);
        }

        if page.tracks.len() > 5 {
            println!("... and {} more tracks", page.tracks.len() - 5);
        }

        if page.has_next_page {
            println!("\n--- Second Page ---");
            if let Some(page2) = iterator.next_page().await? {
                println!(
                    "Page {}/{:?} - {} tracks",
                    page2.page_number,
                    page2.total_pages.unwrap_or(0),
                    page2.tracks.len()
                );

                for (i, track) in page2.tracks.iter().take(3).enumerate() {
                    println!("{}. {} - {} plays", i + 1, track.name, track.playcount);
                }
            }
        }
    } else {
        println!("No tracks found for {}", artist);
    }

    // Test taking a specific number of tracks
    println!("\n--- Using take() method ---");
    let mut iterator2 = client.artist_tracks(artist);
    let first_10_tracks = iterator2.take(10).await?;
    println!("Got {} tracks using take(10):", first_10_tracks.len());
    for (i, track) in first_10_tracks.iter().enumerate() {
        println!("{}. {} - {} plays", i + 1, track.name, track.playcount);
    }

    Ok(())
}
