use http_client::{HttpClient, Request};
use http_client_vcr::{VcrClient, VcrMode};
use http_types::{Method, Url};

#[tokio::test]
async fn test_vcr_post_recording() -> Result<(), Box<dyn std::error::Error>> {
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Debug)
        .is_test(true)
        .try_init();

    println!("🧪 Testing VCR POST request recording");

    // Test with httpbin.org/post (known to accept POST requests)
    let test_url = "https://httpbin.org/post";
    let form_data = "test_field=test_value&another_field=another_value";

    {
        let cassette_path = "tests/fixtures/post_test.yaml";

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

        let mut post_request = Request::new(Method::Post, Url::parse(test_url).unwrap());
        let _ = post_request.insert_header("User-Agent", "test-agent");
        let _ = post_request.insert_header("Content-Type", "application/x-www-form-urlencoded");
        post_request.set_body(form_data);

        println!("📤 Sending POST request through VCR...");

        let mut response = vcr_client.send(post_request).await?;
        let response_body = response.body_string().await?;

        println!(
            "✅ POST response received, body length: {}",
            response_body.len()
        );

        // Check what was recorded
        if std::path::Path::new(cassette_path).exists() {
            let cassette_content = std::fs::read_to_string(cassette_path)?;
            let interaction_count = cassette_content.matches("- request:").count();
            println!("📼 Cassette recorded {interaction_count} interactions");

            if cassette_content.contains("method: POST") {
                println!("✅ POST request was recorded in cassette");
            } else {
                println!("❌ POST request was NOT recorded in cassette");
            }

            // Print first few lines for inspection
            println!("\n📼 Cassette content (first 20 lines):");
            for (i, line) in cassette_content.lines().enumerate() {
                if i >= 20 {
                    break;
                }
                println!("{}: {}", i + 1, line);
            }
        }

        // Clean up
        std::fs::remove_file(cassette_path).ok();
    }

    Ok(())
}
