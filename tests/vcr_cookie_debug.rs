use http_client::HttpClient;
use http_client_vcr::{VcrClient, VcrMode};

#[tokio::test]
async fn debug_vcr_cookie_handling() -> Result<(), Box<dyn std::error::Error>> {
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Debug)
        .is_test(true)
        .try_init();

    println!("🔍 Debugging VCR cookie handling");

    let cassette_path = "tests/fixtures/cookie_debug_test.yaml";

    // Clean up any existing cassette
    std::fs::remove_file(cassette_path).ok();

    let inner_client: Box<dyn HttpClient + Send + Sync> =
        Box::new(http_client::native::NativeClient::new());
    let vcr_client = VcrClient::builder()
        .inner_client(inner_client)
        .mode(VcrMode::Record)
        .cassette_path(cassette_path)
        .build()
        .await?;

    // Step 1: Make GET request to login page
    println!("\n=== STEP 1: GET /login ===");
    let login_url: http_types::Url = "https://www.last.fm/login".parse()?;
    let mut get_request = http_types::Request::new(http_types::Method::Get, login_url);
    get_request.insert_header("User-Agent", "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/138.0.0.0 Safari/537.36")?;

    let get_response = vcr_client.send(get_request).await?;
    println!("✅ GET response status: {}", get_response.status());

    // Step 2: Extract cookies manually (like our login code does)
    let mut cookies = Vec::new();
    if let Some(set_cookie_headers) = get_response.header("set-cookie") {
        for cookie_header in set_cookie_headers {
            let cookie_str = cookie_header.as_str();
            if let Some(cookie_value) = cookie_str.split(';').next() {
                let cookie_name = cookie_value.split('=').next().unwrap_or("");

                // Remove any existing cookie with the same name (like extract_cookies_from_response does)
                cookies
                    .retain(|existing: &String| !existing.starts_with(&format!("{cookie_name}=")));
                cookies.push(cookie_value.to_string());
            }
        }
    }

    println!("🍪 Extracted cookies: {cookies:?}");

    // Step 3: Make POST request with cookies manually added
    println!("\n=== STEP 2: POST /login with cookies ===");
    let post_url: http_types::Url = "https://www.last.fm/login".parse()?;
    let mut post_request = http_types::Request::new(http_types::Method::Post, post_url);
    post_request.insert_header("User-Agent", "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/138.0.0.0 Safari/537.36")?;
    post_request.insert_header("Content-Type", "application/x-www-form-urlencoded")?;

    // Add cookies manually (like our login code does)
    if !cookies.is_empty() {
        let cookie_header = cookies.join("; ");
        println!("📤 Adding Cookie header: {cookie_header}");
        post_request.insert_header("Cookie", &cookie_header)?;
    }

    // Simple form data (not trying to login, just testing cookie handling)
    let form_data = "test=value";
    post_request.set_body(form_data);

    let post_response = vcr_client.send(post_request).await?;
    println!("✅ POST response status: {}", post_response.status());

    // Step 4: Check what was recorded in the cassette
    println!("\n=== STEP 3: Check recorded cassette ===");
    drop(vcr_client); // Ensure cassette is saved

    let cassette_content = std::fs::read_to_string(cassette_path)?;

    // Check if POST request has Cookie header
    if cassette_content.contains("method: POST") {
        if cassette_content.contains("cookie:") || cassette_content.contains("Cookie:") {
            println!("✅ POST request in cassette DOES contain Cookie header");
        } else {
            println!("❌ POST request in cassette MISSING Cookie header");
        }
    }

    // Print relevant parts of cassette
    println!("\n=== CASSETTE CONTENT ===");
    for (i, line) in cassette_content.lines().enumerate() {
        if line.contains("method: POST") || line.contains("cookie:") || line.contains("Cookie:") {
            println!("Line {}: {}", i + 1, line);
        }
    }

    Ok(())
}
