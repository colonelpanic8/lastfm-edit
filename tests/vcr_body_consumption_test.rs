use http_client::{HttpClient, Request};
use http_client_vcr::{VcrClient, VcrMode};
use http_types::{Method, Url};

#[tokio::test]
async fn test_vcr_body_consumption() -> Result<(), Box<dyn std::error::Error>> {
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Debug)
        .is_test(true)
        .try_init();

    println!("🧪 Testing VCR body consumption behavior");

    // Test with a simple GET request to httpbin.org/html (known to return HTML)
    let test_url = "https://httpbin.org/html";

    // Direct test first
    println!("\n📡 === DIRECT TEST ===");
    let direct_client = Box::new(http_client::native::NativeClient::new());
    let mut direct_request = Request::new(Method::Get, test_url.parse::<Url>().unwrap());
    let _ = direct_request.insert_header("User-Agent", "test-agent");

    let mut direct_response = direct_client.send(direct_request).await?;
    let direct_body = direct_response.body_string().await?;
    println!("✅ Direct response body length: {}", direct_body.len());

    {
        // VCR test
        println!("\n📼 === VCR TEST ===");
        let cassette_path = "tests/fixtures/body_consumption_test.yaml";

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

        let mut vcr_request = Request::new(Method::Get, test_url.parse::<Url>().unwrap());
        let _ = vcr_request.insert_header("User-Agent", "test-agent");

        let mut vcr_response = vcr_client.send(vcr_request).await?;
        let vcr_body = vcr_response.body_string().await.unwrap_or_else(|e| {
            println!("❌ Failed to read VCR response body: {e}");
            String::new()
        });

        println!("📼 VCR response body length: {}", vcr_body.len());

        if vcr_body.is_empty() {
            println!("❌ VCR response body is empty - this confirms the body consumption issue!");
        } else {
            println!("✅ VCR response body is not empty");
        }

        // Clean up
        std::fs::remove_file(cassette_path).ok();
    }

    Ok(())
}
