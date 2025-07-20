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

    // Manual URL construction for debugging
    let artist = "The Beatles";
    let url = format!(
        "https://www.last.fm/user/{}/library/music/{}/+tracks",
        username,
        urlencoding::encode(artist)
    );

    println!("Fetching URL: {}", url);

    // This is a bit hacky - we'll access the private get method by making it public temporarily
    // For now, let's just test with the existing public API and examine the results

    let page = client.get_artist_tracks_page(artist, 1).await?;

    println!("Found {} tracks on page 1", page.tracks.len());
    println!("Sample tracks:");

    for (i, track) in page.tracks.iter().take(10).enumerate() {
        println!(
            "{}. '{}' by '{}' - {} plays",
            i + 1,
            track.name,
            track.artist,
            track.playcount
        );
    }

    // Check if all play counts are 1 (which would indicate parsing issues)
    let all_ones = page.tracks.iter().all(|t| t.playcount == 1);
    if all_ones && page.tracks.len() > 5 {
        println!("\n⚠️  WARNING: All play counts are 1 - this suggests parsing issues");
        println!("The selector '.chartlist-count-bar-value' might not be correct");
    }

    Ok(())
}
