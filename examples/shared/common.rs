use lastfm_edit::{LastFmClient, Result};
use std::env;

pub async fn setup_client() -> Result<LastFmClient> {
    // Initialize logger to handle log::debug! calls
    env_logger::init();

    let username = env::var("LASTFM_EDIT_USERNAME")
        .expect("LASTFM_EDIT_USERNAME environment variable not set");
    let password = env::var("LASTFM_EDIT_PASSWORD")
        .expect("LASTFM_EDIT_PASSWORD environment variable not set");

    // Create client and login
    let http_client = http_client::native::NativeClient::new();
    let mut client = LastFmClient::new(Box::new(http_client));

    println!("Logging in as {username}...");
    client.login(&username, &password).await?;
    println!("✓ Logged in successfully");

    Ok(client)
}
