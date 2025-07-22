use lastfm_edit::{LastFmEditClient, Result};
use std::env;

#[tokio::main]
async fn main() -> Result<()> {
    let username = env::var("LASTFM_EDIT_USERNAME")
        .expect("LASTFM_EDIT_USERNAME environment variable not set");
    let password = env::var("LASTFM_EDIT_PASSWORD")
        .expect("LASTFM_EDIT_PASSWORD environment variable not set");

    // Method 1: Traditional create + login pattern
    let http_client = http_client::native::NativeClient::new();
    let mut client = LastFmEditClient::new(Box::new(http_client));

    println!("Attempting to login as {username}...");
    client.login(&username, &password).await?;

    // Alternative Method 2: One-step initialization (commented out)
    // let http_client = http_client::native::NativeClient::new();
    // let client = LastFmEditClient::login_with_credentials(
    //     Box::new(http_client),
    //     &username,
    //     &password
    // ).await?;

    println!("Successfully logged in as {}", client.username());
    println!(
        "Login status: {}",
        if client.is_logged_in() {
            "✓ Authenticated"
        } else {
            "✗ Not authenticated"
        }
    );

    Ok(())
}
