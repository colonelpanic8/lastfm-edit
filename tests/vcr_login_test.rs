use http_client_vcr::VcrMode;
use lastfm_edit::{LastFmEditClient, LastFmEditClientImpl};
use std::env;

mod common;

async fn login_with_client(
    client: Box<dyn http_client::HttpClient + Send + Sync>,
    _test_name: &str,
) {
    // Use credentials from environment for recording
    let username = env::var("LASTFM_EDIT_USERNAME").unwrap_or("test_user".to_string());
    let password = env::var("LASTFM_EDIT_PASSWORD").unwrap_or("test_password".to_string());

    let client = LastFmEditClientImpl::login_with_credentials(client, &username, &password)
        .await
        .expect("Login failed");

    // Try getting one recent track to verify the session works
    let mut recent_tracks = client.recent_tracks();
    let _track = recent_tracks
        .next()
        .await
        .expect("Failed to get recent track")
        .expect("Should have at least one recent track");
}

#[test_log::test(tokio::test)]
async fn vcr_login_test() {
    let cassette_path = "tests/fixtures/vcr_login_test.yaml";

    // Create VCR client using helper (no filters for simple login test)
    let vcr_client = common::create_vcr_client(
        cassette_path,
        VcrMode::Record, // Always record for this test
        None,            // No filters
    )
    .await
    .expect("Failed to create VCR client");

    login_with_client(Box::new(vcr_client), "VCR").await;
}

// Disabled - we only want to test VCR login now
// #[test_log::test(tokio::test)]
// async fn direct_login_test() -> Result<(), Box<dyn std::error::Error>> {
//     // Create direct HTTP client (no VCR)
//     let direct_client = Box::new(http_client::native::NativeClient::new());
//     test_login_with_client(direct_client, "DIRECT").await
// }
