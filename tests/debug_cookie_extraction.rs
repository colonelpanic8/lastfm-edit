use http_client::HttpClient;
use http_client_vcr::{VcrClient, VcrMode};
use lastfm_edit::login::extract_cookies_from_response;

#[tokio::test]
async fn debug_cookie_extraction() -> Result<(), Box<dyn std::error::Error>> {
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Debug)
        .is_test(true)
        .try_init();

    println!("🔍 Debugging cookie extraction: Direct vs VCR");

    // Test 1: Direct HTTP client - fetch login page
    println!("\n=== DIRECT HTTP CLIENT ===");
    let direct_client: Box<dyn http_client::HttpClient + Send + Sync> =
        Box::new(http_client::native::NativeClient::new());

    let url: http_types::Url = "https://www.last.fm/login".parse()?;
    let mut request = http_types::Request::new(http_types::Method::Get, url);
    request.insert_header("User-Agent", "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/138.0.0.0 Safari/537.36")?;

    let response = direct_client.send(request).await?;
    let mut direct_cookies = Vec::new();
    extract_cookies_from_response(&response, &mut direct_cookies);

    println!("📋 Direct response status: {}", response.status());
    println!("🍪 Direct cookies extracted: {direct_cookies:?}");

    // Find CSRF token in direct cookies
    let direct_csrf = direct_cookies
        .iter()
        .find(|cookie| cookie.starts_with("csrftoken="))
        .map(|cookie| {
            cookie
                .split('=')
                .nth(1)
                .unwrap_or("")
                .split(';')
                .next()
                .unwrap_or("")
        })
        .unwrap_or("NOT_FOUND");
    println!("🔑 Direct CSRF token: {direct_csrf}");

    // Test 2: VCR client - fetch from recorded cassette
    println!("\n=== VCR CLIENT (REPLAY) ===");
    let cassette_path = "tests/fixtures/csrf_comparison_test.yaml";

    let vcr_inner_client: Box<dyn http_client::HttpClient + Send + Sync> =
        Box::new(http_client::native::NativeClient::new());
    let vcr_client = VcrClient::builder()
        .inner_client(vcr_inner_client)
        .mode(VcrMode::Replay)
        .cassette_path(cassette_path)
        .build()
        .await?;

    let vcr_url: http_types::Url = "https://www.last.fm/login".parse()?;
    let mut vcr_request = http_types::Request::new(http_types::Method::Get, vcr_url);
    vcr_request.insert_header("User-Agent", "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/138.0.0.0 Safari/537.36")?;

    let vcr_response = vcr_client.send(vcr_request).await?;
    let mut vcr_cookies = Vec::new();
    extract_cookies_from_response(&vcr_response, &mut vcr_cookies);

    println!("📋 VCR response status: {}", vcr_response.status());
    println!("🍪 VCR cookies extracted: {vcr_cookies:?}");

    // Find CSRF token in VCR cookies
    let vcr_csrf = vcr_cookies
        .iter()
        .find(|cookie| cookie.starts_with("csrftoken="))
        .map(|cookie| {
            cookie
                .split('=')
                .nth(1)
                .unwrap_or("")
                .split(';')
                .next()
                .unwrap_or("")
        })
        .unwrap_or("NOT_FOUND");
    println!("🔑 VCR CSRF token: {vcr_csrf}");

    // Compare results
    println!("\n=== COMPARISON ===");
    println!("Direct cookies count: {}", direct_cookies.len());
    println!("VCR cookies count: {}", vcr_cookies.len());
    println!("CSRF tokens match: {}", direct_csrf == vcr_csrf);

    if direct_csrf != vcr_csrf {
        println!("❌ CSRF token mismatch!");
        println!("   Direct: {direct_csrf}");
        println!("   VCR:    {vcr_csrf}");
    } else {
        println!("✅ CSRF tokens match");
    }

    Ok(())
}
