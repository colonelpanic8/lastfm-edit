use lastfm_edit::{LastFmEditClientImpl, Result};
use std::env;

#[tokio::main]
async fn main() -> Result<()> {
    let username = env::var("LASTFM_EDIT_USERNAME")
        .expect("LASTFM_EDIT_USERNAME environment variable not set");
    let password = env::var("LASTFM_EDIT_PASSWORD")
        .expect("LASTFM_EDIT_PASSWORD environment variable not set");

    // Login and create client in one step
    let http_client = http_client::native::NativeClient::new();
    println!("Attempting to login as {username}...");
    let client =
        LastFmEditClientImpl::login_with_credentials(Box::new(http_client), &username, &password)
            .await?;

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
