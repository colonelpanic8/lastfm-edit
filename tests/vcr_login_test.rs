use http_client_vcr::{VcrClient, VcrMode};
use lastfm_edit::{LastFmEditClient, LastFmEditClientImpl};
use std::env;

async fn test_login_with_client(
    client: Box<dyn http_client::HttpClient + Send + Sync>,
    test_name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    // Use credentials from environment for recording
    let username = env::var("LASTFM_EDIT_USERNAME").unwrap_or("test_user".to_string());
    let password = env::var("LASTFM_EDIT_PASSWORD").unwrap_or("test_password".to_string());

    println!("🧪 {} - Testing with username: {username}", test_name);

    println!("🔐 Attempting login...");
    let client_result =
        LastFmEditClientImpl::login_with_credentials(client, &username, &password).await;

    match client_result {
        Ok(client) => {
            println!("✅ {} - Login successful!", test_name);

            // Try getting one recent track to verify the session works
            let mut recent_tracks = client.recent_tracks();
            if let Some(track) = recent_tracks.next().await? {
                println!(
                    "📻 {} - First track: {} - {} ({})",
                    test_name,
                    track.artist,
                    track.name,
                    track.album.as_deref().unwrap_or("N/A")
                );
            }

            Ok(())
        }
        Err(e) => {
            println!("❌ {} - Login failed: {e}", test_name);
            Err(e.into())
        }
    }
}

#[test_log::test(tokio::test)]
async fn vcr_login_test() -> Result<(), Box<dyn std::error::Error>> {
    let cassette_path = "tests/fixtures/simple_login_test.yaml";

    // Ensure fixtures directory exists
    if let Some(parent_dir) = std::path::Path::new(cassette_path).parent() {
        std::fs::create_dir_all(parent_dir)?;
    }

    // Create simple VCR client with no filtering, no matching
    let inner_client = Box::new(http_client::native::NativeClient::new());
    let vcr_client = VcrClient::builder()
        .inner_client(inner_client)
        .mode(VcrMode::Record) // Record once, then replay
        .cassette_path(cassette_path)
        .build()
        .await?;

    test_login_with_client(Box::new(vcr_client), "VCR").await
}

#[test_log::test(tokio::test)]
async fn direct_login_test() -> Result<(), Box<dyn std::error::Error>> {
    // Create direct HTTP client (no VCR)
    let direct_client = Box::new(http_client::native::NativeClient::new());
    test_login_with_client(direct_client, "DIRECT").await
}
