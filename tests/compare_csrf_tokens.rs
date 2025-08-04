use http_client_vcr::{VcrClient, VcrMode};
use lastfm_edit::login::LoginManager;
use std::env;
use std::sync::Arc;

#[tokio::test]
async fn compare_csrf_tokens_direct_vs_vcr() -> Result<(), Box<dyn std::error::Error>> {
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Debug)
        .is_test(true)
        .try_init();

    println!("🔍 Comparing CSRF token handling: Direct vs VCR");

    let username = env::var("LASTFM_EDIT_USERNAME").expect("LASTFM_EDIT_USERNAME not set");
    let password = env::var("LASTFM_EDIT_PASSWORD").expect("LASTFM_EDIT_PASSWORD not set");

    // Test 1: Direct HTTP client
    println!("\n=== DIRECT HTTP CLIENT TEST ===");
    let direct_client: Box<dyn http_client::HttpClient + Send + Sync> =
        Box::new(http_client::native::NativeClient::new());
    let direct_client_arc: Arc<dyn http_client::HttpClient + Send + Sync> =
        Arc::from(direct_client);
    let direct_login_manager =
        LoginManager::new(direct_client_arc, "https://www.last.fm".to_string());

    println!("🔑 Testing CSRF token extraction with direct client...");
    let direct_result = direct_login_manager.login(&username, &password).await;

    match &direct_result {
        Ok(_) => println!("✅ Direct login: SUCCESS"),
        Err(e) => println!("❌ Direct login: FAILED - {e}"),
    }

    // Test 2: VCR client in Record mode
    println!("\n=== VCR CLIENT TEST (RECORD MODE) ===");
    let cassette_path = "tests/fixtures/csrf_comparison_test.yaml";

    // Ensure fixtures directory exists
    if let Some(parent_dir) = std::path::Path::new(cassette_path).parent() {
        std::fs::create_dir_all(parent_dir)?;
    }

    // Clean up any existing cassette to force fresh recording
    std::fs::remove_file(cassette_path).ok();

    let inner_client: Box<dyn http_client::HttpClient + Send + Sync> =
        Box::new(http_client::native::NativeClient::new());
    let vcr_client = VcrClient::builder()
        .inner_client(inner_client)
        .mode(VcrMode::Record)
        .cassette_path(cassette_path)
        .build()
        .await?;

    let vcr_client_arc: Arc<dyn http_client::HttpClient + Send + Sync> = Arc::new(vcr_client);
    let vcr_login_manager = LoginManager::new(vcr_client_arc, "https://www.last.fm".to_string());

    println!("🎬 Testing CSRF token extraction with VCR client (Record mode)...");
    let vcr_result = vcr_login_manager.login(&username, &password).await;

    match &vcr_result {
        Ok(_) => println!("✅ VCR login: SUCCESS"),
        Err(e) => println!("❌ VCR login: FAILED - {e}"),
    }

    // Test 3: VCR client in Replay mode
    println!("\n=== VCR CLIENT TEST (REPLAY MODE) ===");
    let replay_inner_client: Box<dyn http_client::HttpClient + Send + Sync> =
        Box::new(http_client::native::NativeClient::new());
    let replay_vcr_client = VcrClient::builder()
        .inner_client(replay_inner_client)
        .mode(VcrMode::Replay)
        .cassette_path(cassette_path)
        .build()
        .await?;

    let replay_vcr_client_arc: Arc<dyn http_client::HttpClient + Send + Sync> =
        Arc::new(replay_vcr_client);
    let replay_login_manager =
        LoginManager::new(replay_vcr_client_arc, "https://www.last.fm".to_string());

    println!("📼 Testing CSRF token extraction with VCR client (Replay mode)...");
    let replay_result = replay_login_manager.login(&username, &password).await;

    match &replay_result {
        Ok(_) => println!("✅ VCR replay login: SUCCESS"),
        Err(e) => println!("❌ VCR replay login: FAILED - {e}"),
    }

    // Analysis
    println!("\n=== ANALYSIS ===");
    println!(
        "Direct result: {}",
        if direct_result.is_ok() {
            "✅ SUCCESS"
        } else {
            "❌ FAILED"
        }
    );
    println!(
        "VCR record result: {}",
        if vcr_result.is_ok() {
            "✅ SUCCESS"
        } else {
            "❌ FAILED"
        }
    );
    println!(
        "VCR replay result: {}",
        if replay_result.is_ok() {
            "✅ SUCCESS"
        } else {
            "❌ FAILED"
        }
    );

    // Check cassette content for debugging
    if std::path::Path::new(cassette_path).exists() {
        let cassette_content = std::fs::read_to_string(cassette_path)?;
        let interaction_count = cassette_content.matches("- request:").count();
        println!("📼 Cassette contains {interaction_count} interactions");

        // Look for CSRF tokens in the cassette
        if cassette_content.contains("csrfmiddlewaretoken") {
            println!("🔑 Found CSRF token references in cassette");
        } else {
            println!("❌ No CSRF token references found in cassette");
        }

        // Look for 403 responses
        if cassette_content.contains("status: 403") {
            println!("⚠️  Found 403 Forbidden responses in cassette");
        }
    }

    // Clean up
    std::fs::remove_file(cassette_path).ok();

    Ok(())
}
