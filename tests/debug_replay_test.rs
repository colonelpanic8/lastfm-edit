use http_client_vcr::{VcrClient, VcrMode};
use lastfm_edit::login::LoginManager;
use std::sync::Arc;

#[tokio::test]
async fn debug_replay_existing_cassette() -> Result<(), Box<dyn std::error::Error>> {
    println!("🔍 Debug Replay Test - Using existing cassette");

    let cassette_path = "tests/fixtures/login_recent_tracks.yaml";

    // Use existing cassette in replay mode
    let inner_client = Box::new(http_client::native::NativeClient::new());
    let vcr_client = VcrClient::builder()
        .inner_client(inner_client)
        .mode(VcrMode::Replay)
        .cassette_path(cassette_path)
        .build()
        .await?;

    // Test credentials
    let username = "test_user";
    let password = "test_password";

    println!("📼 Testing LoginManager with existing cassette (replay mode)");

    let client_arc: Arc<dyn http_client::HttpClient + Send + Sync> = Arc::new(vcr_client);
    let login_manager = LoginManager::new(client_arc, "https://www.last.fm".to_string());

    println!("📡 Attempting login with existing cassette...");

    let result = login_manager.login(username, password).await;

    match result {
        Ok(_) => println!("✅ Login succeeded (unexpected)"),
        Err(e) => {
            println!("❌ Login failed: {e}");

            // Check if the error is what we expect
            let error_str = e.to_string();
            if error_str.contains("CSRF token not found") {
                println!("🎯 Error is 'CSRF token not found' - this means:");
                println!("   1. GET request succeeded (got login page HTML)");
                println!("   2. CSRF token extraction failed");
                println!("   3. POST request was never attempted");
                println!("   4. This explains why cassette only has 1 interaction");
            } else if error_str.contains("no matching interaction found") {
                println!("🎯 Error is 'no matching interaction' - this means:");
                println!("   1. GET request succeeded");
                println!("   2. CSRF token was extracted successfully");
                println!("   3. POST request was attempted");
                println!("   4. But cassette doesn't have the POST request recorded");
            }
        }
    }

    Ok(())
}
