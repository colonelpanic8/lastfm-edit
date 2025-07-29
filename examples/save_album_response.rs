#[path = "shared/common.rs"]
mod common;

use lastfm_edit::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let client = common::setup_client().await?;

    // Test the URL construction manually to see what's wrong
    let artist = "Radiohead";
    let album = "In Rainbows";

    // Get the session details to construct URLs manually
    let session = client.get_session();
    println!("Username: {}", session.username);
    println!("Base URL: {}", session.base_url);

    // Construct the URL we're using
    let url = format!(
        "{}/user/{}/library/music/{}/{}/+tracks?page=1&ajax=true",
        session.base_url,
        session.username,
        artist.replace(" ", "+"),
        album.replace(" ", "+")
    );

    println!("Constructed URL: {url}");

    // Let's also try some variations (manually encoded):
    let artist_encoded = artist.replace(" ", "%20");
    let album_encoded = album.replace(" ", "%20");
    let url_encoded = format!(
        "{}/user/{}/library/music/{}/{}/+tracks?page=1&ajax=true",
        session.base_url, session.username, artist_encoded, album_encoded
    );

    println!("URL encoded version: {url_encoded}");

    // Test with a manually constructed HTTP client to see what we get
    println!("\nMaking direct HTTP request to see response...");

    // Make the request using the client's internal HTTP client
    // We'll access this by making the client call directly and examining response
    match client.get_album_tracks_page(album, artist, 1).await {
        Ok(tracks_page) => {
            println!("Success: {} tracks", tracks_page.tracks.len());
        }
        Err(e) => {
            println!("Error: {e}");
        }
    }

    // Let's also check what a working artist tracks URL looks like for comparison
    let artist_tracks_url = format!(
        "{}/user/{}/library/music/{}/+tracks?page=1&ajax=true",
        session.base_url,
        session.username,
        artist.replace(" ", "+")
    );
    println!("Artist tracks URL (working): {artist_tracks_url}");

    // And albums URL
    let artist_albums_url = format!(
        "{}/user/{}/library/music/{}/+albums?page=1&ajax=true",
        session.base_url,
        session.username,
        artist.replace(" ", "+")
    );
    println!("Artist albums URL (working): {artist_albums_url}");

    Ok(())
}
