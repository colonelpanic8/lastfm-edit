use http_client_vcr::{VcrClient, VcrMode};
use lastfm_edit::LastFmEditClientImpl;
use std::env;

#[tokio::test]
async fn debug_vcr_login_with_logging() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize env_logger to see all the debug output
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Debug)
        .is_test(true)
        .try_init();

    let username = env::var("LASTFM_EDIT_USERNAME").unwrap_or("test_user".to_string());
    let password = env::var("LASTFM_EDIT_PASSWORD").unwrap_or("test_password".to_string());

    println!("🔍 Debug VCR vs Direct login with detailed logging");
    println!("Username: {username}");

    // Test 1: Direct login first (should work)
    println!("\n📡 === DIRECT LOGIN TEST ===");
    let direct_client = Box::new(http_client::native::NativeClient::new());
    let direct_result =
        LastFmEditClientImpl::login_with_credentials(direct_client, &username, &password).await;

    match direct_result {
        Ok(_) => println!("✅ Direct login: SUCCESS"),
        Err(e) => {
            println!("❌ Direct login: FAILED - {e}");
            return Ok(()); // Don't continue if direct login fails
        }
    }

    // Test 2: VCR login with detailed logging
    {
        println!("\n📼 === VCR LOGIN TEST (with logging) ===");
        let cassette_path = "tests/fixtures/debug_vcr_login.yaml";

        // Ensure fixtures directory exists
        if let Some(parent_dir) = std::path::Path::new(cassette_path).parent() {
            std::fs::create_dir_all(parent_dir)?;
        }

        let inner_client = Box::new(http_client::native::NativeClient::new());
        let vcr_client = VcrClient::builder()
            .inner_client(inner_client)
            .mode(VcrMode::Record)
            .cassette_path(cassette_path)
            .build()
            .await?;

        let vcr_result = LastFmEditClientImpl::login_with_credentials(
            Box::new(vcr_client),
            &username,
            &password,
        )
        .await;

        match vcr_result {
            Ok(_) => println!("✅ VCR login: SUCCESS"),
            Err(e) => {
                println!("❌ VCR login: FAILED - {e}");

                // Read the cassette to see what was actually recorded
                if std::path::Path::new(cassette_path).exists() {
                    match std::fs::read_to_string(cassette_path) {
                        Ok(content) => {
                            println!("\n📼 Recorded cassette content:");
                            println!("{content}");
                        }
                        Err(e) => println!("Failed to read cassette: {e}"),
                    }
                }
            }
        }

        // Clean up
        std::fs::remove_file(cassette_path).ok();
    }

    Ok(())
}
