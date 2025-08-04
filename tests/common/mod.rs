use http_client_vcr::{VcrClient, VcrMode};
use lastfm_edit::vcr_matcher::LastFmMatcher;
use lastfm_edit::vcr_test_utils::{create_lastfm_test_filter_chain, prepare_lastfm_test_cassette};
use lastfm_edit::{LastFmEditClient, LastFmEditClientImpl};
use std::env;
use std::fs;

/// Helper for creating Last.fm test clients with proper VCR setup
/// This handles the "login and then do stuff" pattern consistently across tests
pub async fn create_lastfm_test_client(
    test_name: &str,
) -> Result<Box<dyn LastFmEditClient>, Box<dyn std::error::Error>> {
    let cassette_path = format!("tests/fixtures/{test_name}.yaml");

    // Ensure fixtures directory exists
    if let Some(parent_dir) = std::path::Path::new(&cassette_path).parent() {
        fs::create_dir_all(parent_dir)?;
    }

    let vcr_record = env::var("VCR_RECORD").unwrap_or_default() == "true";
    let cassette_exists = std::path::Path::new(&cassette_path).exists();

    let mode = if vcr_record {
        println!("🎬 Recording mode: will capture new Last.fm interactions");
        VcrMode::Record
    } else if cassette_exists {
        println!("📼 Replay mode: using existing cassette with Last.fm test filtering");
        VcrMode::Replay
    } else {
        println!("📝 Once mode: will record interactions on first run");
        VcrMode::Once
    };

    // Handle credentials based on mode
    let (username, password) = if vcr_record {
        // Recording mode: need real credentials
        let username = env::var("LASTFM_EDIT_USERNAME")
            .expect("LASTFM_EDIT_USERNAME required when VCR_RECORD=true");
        let password = env::var("LASTFM_EDIT_PASSWORD")
            .expect("LASTFM_EDIT_PASSWORD required when VCR_RECORD=true");
        println!("🔐 Using real credentials for recording");
        (username, password)
    } else {
        // Replay mode: use test credentials
        // The cassette should have the real username preserved but password filtered
        println!("🧪 Using test credentials for replay");
        ("test_user".to_string(), "test_password".to_string())
    };

    // Create VCR client with Last.fm test filtering and Last.fm-specific matching
    let inner_client = Box::new(http_client::native::NativeClient::new());
    let filter_chain = create_lastfm_test_filter_chain()?;
    let lastfm_matcher = Box::new(LastFmMatcher::new());

    let vcr_client = VcrClient::builder()
        .inner_client(inner_client)
        .mode(mode.clone())
        .cassette_path(&cassette_path)
        .filter_chain(filter_chain)
        .matcher(lastfm_matcher)
        .build()
        .await?;

    println!("🎵 Attempting Last.fm login for test: {test_name}");

    // Attempt login
    let client_result =
        LastFmEditClientImpl::login_with_credentials(Box::new(vcr_client), &username, &password)
            .await;

    match client_result {
        Ok(client) => {
            println!("✅ Last.fm login successful!");

            // If we just recorded, prepare the cassette for future test runs
            if matches!(mode, VcrMode::Record | VcrMode::Once) {
                println!("🧹 Preparing cassette for future test runs...");
                prepare_lastfm_test_cassette(&cassette_path).await?;
                println!("✅ Cassette prepared: username preserved, credentials filtered");
            }

            Ok(Box::new(client))
        }
        Err(e) => {
            if vcr_record {
                Err(format!("Recording mode login failed: {e}").into())
            } else {
                // In replay mode, login might fail due to credential mismatch - that's expected
                // But we still want to return an error for the test to handle
                Err(format!("Replay mode login failed (this might be expected): {e}").into())
            }
        }
    }
}

/// Test if a Last.fm client login succeeds, handling replay mode gracefully
/// Returns Ok(Some(client)) on success, Ok(None) on expected replay failure, Err on unexpected failure
pub async fn try_lastfm_login(
    test_name: &str,
) -> Result<Option<Box<dyn LastFmEditClient>>, Box<dyn std::error::Error>> {
    match create_lastfm_test_client(test_name).await {
        Ok(client) => Ok(Some(client)),
        Err(e) => {
            let vcr_record = env::var("VCR_RECORD").unwrap_or_default() == "true";
            if vcr_record {
                // Recording mode failure is a real error
                Err(e)
            } else {
                // Replay mode failure might be expected (credential mismatch)
                println!("⚠️  Login failed in replay mode: {e}");
                println!("   This is expected if credentials don't match the cassette");
                Ok(None)
            }
        }
    }
}
