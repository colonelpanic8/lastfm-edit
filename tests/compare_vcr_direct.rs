use http_client_vcr::{VcrClient, VcrMode};
use lastfm_edit::LastFmEditClientImpl;
use std::env;

#[tokio::test]
async fn compare_vcr_vs_direct() -> Result<(), Box<dyn std::error::Error>> {
    let username = env::var("LASTFM_EDIT_USERNAME").unwrap_or("test_user".to_string());
    let password = env::var("LASTFM_EDIT_PASSWORD").unwrap_or("test_password".to_string());

    println!("🔍 Testing direct vs VCR login with username: {username}");

    // Test 1: Direct login (should work)
    println!("\n📡 Testing DIRECT login...");
    let direct_client = Box::new(http_client::native::NativeClient::new());
    let direct_result =
        LastFmEditClientImpl::login_with_credentials(direct_client, &username, &password).await;

    match direct_result {
        Ok(_) => println!("✅ Direct login: SUCCESS"),
        Err(e) => {
            println!("❌ Direct login: FAILED - {e}");
            return Ok(()); // Don't fail the test, just show the comparison
        }
    }

    // Test 2: VCR login (fails)
    println!("\n📼 Testing VCR login...");
    let cassette_path = "tests/fixtures/compare_test.yaml";

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

    let vcr_result =
        LastFmEditClientImpl::login_with_credentials(Box::new(vcr_client), &username, &password)
            .await;

    match vcr_result {
        Ok(_) => println!("✅ VCR login: SUCCESS"),
        Err(e) => println!("❌ VCR login: FAILED - {e}"),
    }

    // Keep the cassette file for analysis
    // std::fs::remove_file(cassette_path).ok();

    println!(
        "\n🤔 If direct works but VCR fails, there's something wrong with VCR's request forwarding"
    );

    Ok(())
}
