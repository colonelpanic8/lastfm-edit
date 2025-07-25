use lastfm_edit::{LastFmEditClientImpl, Result};
use std::env;

pub async fn setup_client() -> Result<LastFmEditClientImpl> {
    // Initialize logger to handle log::debug! calls
    env_logger::init();

    let username = env::var("LASTFM_EDIT_USERNAME")
        .expect("LASTFM_EDIT_USERNAME environment variable not set");
    let password = env::var("LASTFM_EDIT_PASSWORD")
        .expect("LASTFM_EDIT_PASSWORD environment variable not set");

    // Login and create client
    let http_client = http_client::native::NativeClient::new();
    println!("Logging in as {username}...");
    let client =
        LastFmEditClientImpl::login_with_credentials(Box::new(http_client), &username, &password)
            .await?;
    println!("✓ Logged in successfully");

    Ok(client)
}
