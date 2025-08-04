use http_client_vcr::{VcrClient, VcrMode};
use lastfm_edit::login::LoginManager;
use std::sync::Arc;

#[tokio::test]
async fn debug_login_vcr_issue() -> Result<(), Box<dyn std::error::Error>> {
    println!("🔍 Debug Login Test - Isolating the VCR/POST Issue");

    let cassette_path = "tests/fixtures/debug_login_cassette.yaml";

    // Ensure fixtures directory exists
    if let Some(parent_dir) = std::path::Path::new(cassette_path).parent() {
        std::fs::create_dir_all(parent_dir)?;
    }

    // Force recording mode to capture all requests
    let inner_client = Box::new(http_client::native::NativeClient::new());
    let vcr_client = VcrClient::builder()
        .inner_client(inner_client)
        .mode(VcrMode::Record)
        .cassette_path(cassette_path)
        .build()
        .await?;

    // Test credentials (will fail login but should capture both GET and POST)
    let username = "test_user";
    let password = "test_password";

    println!("🎬 Testing LoginManager directly with VCR client");

    let client_arc: Arc<dyn http_client::HttpClient + Send + Sync> = Arc::new(vcr_client);
    let login_manager = LoginManager::new(client_arc, "https://www.last.fm".to_string());

    println!("📡 Attempting login - this should generate 2 requests:");
    println!("   1. GET /login (to fetch CSRF token)");
    println!("   2. POST /login (to submit credentials)");

    let result = login_manager.login(username, password).await;

    match result {
        Ok(_) => println!("✅ Login succeeded (unexpected with test credentials)"),
        Err(e) => println!("❌ Login failed as expected: {e}"),
    }

    println!("🔍 Checking cassette file...");

    // Check how many interactions were recorded
    let cassette_content = std::fs::read_to_string(cassette_path)?;
    let interaction_count = cassette_content.matches("- request:").count();

    println!("📼 Cassette contains {interaction_count} interactions");

    if interaction_count >= 2 {
        println!("✅ Both GET and POST requests were captured!");

        // Check for specific methods
        let get_count = cassette_content.matches("method: GET").count();
        let post_count = cassette_content.matches("method: POST").count();

        println!("   - GET requests: {get_count}");
        println!("   - POST requests: {post_count}");

        if post_count > 0 {
            println!("✅ POST request was successfully sent through VCR!");
        } else {
            println!("❌ No POST request found - the issue is elsewhere");
        }
    } else {
        println!("❌ Only {interaction_count} interaction(s) captured - POST request never sent");
        println!("   This means the login code failed before sending the POST");
    }

    // Clean up
    std::fs::remove_file(cassette_path).ok();

    Ok(())
}
