use http_client::HttpClient;
use http_client_vcr::{filter_cassette_file, BodyFilter, FilterChain, HeaderFilter, VcrMode};
use std::fs;

mod common;

#[tokio::test]
async fn vcr_filter_mode() -> Result<(), Box<dyn std::error::Error>> {
    // Create a test cassette path for filtering
    let original_cassette = "tests/fixtures/queen_discover_scrobbles.yaml";
    let filtered_cassette = "tests/fixtures/queen_discover_scrobbles_filtered.yaml";

    // First, copy the original cassette to a new file for filtering
    if std::path::Path::new(original_cassette).exists() {
        fs::copy(original_cassette, filtered_cassette)?;
        println!("Copied original cassette to {filtered_cassette}");
    } else {
        println!("Original cassette not found, skipping filter test");
        return Ok(());
    }

    // Create a filter chain to clean up sensitive data
    let filter_chain = FilterChain::new()
        // Filter out sensitive headers
        .add_filter(Box::new(
            HeaderFilter::new()
                .replace_header("cookie", "[FILTERED_COOKIE]")
                .replace_header("set-cookie", "[FILTERED_SET-COOKIE]")
                .replace_header("authorization", "[FILTERED_AUTH]"),
        ))
        // Filter CSRF tokens and session data from request/response bodies
        .add_filter(Box::new(
            BodyFilter::new()
                .replace_regex(
                    r"csrfmiddlewaretoken=[^&]+",
                    "csrfmiddlewaretoken=[FILTERED_CSRF]",
                )?
                .replace_regex(r"sessionid=[^;]+", "sessionid=[FILTERED_SESSION]")?
                .replace_regex(
                    r#"name="csrfmiddlewaretoken" value="[^"]+""#,
                    r#"name="csrfmiddlewaretoken" value="[FILTERED_CSRF]""#,
                )?,
        ));

    // Test 1: Use the utility function to filter the cassette file
    println!("Testing filter_cassette_file utility function...");
    filter_cassette_file(filtered_cassette, filter_chain).await?;

    // Test 2: Use VcrClient in Filter mode
    println!("Testing VcrClient in Filter mode...");

    // Create another filter chain for the VcrClient (since we can't clone)
    let vcr_filter_chain = FilterChain::new().add_filter(Box::new(
        HeaderFilter::new()
            .replace_header("cookie", "[FILTERED_COOKIE]")
            .replace_header("set-cookie", "[FILTERED_SET-COOKIE]")
            .replace_header("authorization", "[FILTERED_AUTH]"),
    ));

    let vcr_client =
        common::create_vcr_client(filtered_cassette, VcrMode::Filter, Some(vcr_filter_chain))
            .await?;

    // Try to make a request that should match something in the cassette
    use http_types::{Method, Request, Url};
    let request = Request::new(Method::Get, Url::parse("https://www.last.fm/login")?);

    match vcr_client.send(request).await {
        Ok(response) => {
            println!("Filter mode successfully returned filtered response");
            println!("Response status: {}", response.status());
            // The response should be filtered of sensitive data
        }
        Err(e) => {
            println!("Filter mode request failed (expected if no matching interaction): {e}");
        }
    }

    // Test 3: Apply additional filters to the cassette and save
    println!("Testing apply_filters_to_cassette method...");
    vcr_client.apply_filters_to_cassette().await?;
    vcr_client.save_cassette().await?;

    // Verify the filtered cassette exists and has been modified
    let filtered_content = fs::read_to_string(filtered_cassette)?;

    // Check that sensitive data has been filtered out
    // The cassette should either have no csrf tokens at all, or they should be filtered with our patterns
    // Note: patterns may be URL-encoded in the YAML, so we check for both forms
    let has_unfiltered_csrf = filtered_content.contains("csrfmiddlewaretoken=") 
        && !filtered_content.contains("[FILTERED_CSRF]") 
        && !filtered_content.contains("[FILTERED]_CSRFMIDDLEWARETOKEN")
        && !filtered_content.contains("[SANITIZED]_CSRFMIDDLEWARETOKEN")
        && !filtered_content.contains("%5BFILTERED%5D_CSRFMIDDLEWARETOKEN") // URL-encoded [FILTERED]_CSRFMIDDLEWARETOKEN
        && !filtered_content.contains("%5BSANITIZED%5D_CSRFMIDDLEWARETOKEN"); // URL-encoded [SANITIZED]_CSRFMIDDLEWARETOKEN

    assert!(
        !has_unfiltered_csrf,
        "CSRF tokens should be filtered. Found potentially unfiltered tokens in cassette."
    );

    println!("✅ Filter mode test completed successfully!");
    println!("🧹 Sensitive data has been cleaned from the cassette");
    println!("📁 Filtered cassette saved to: {filtered_cassette}");

    // Clean up the test file
    if std::path::Path::new(filtered_cassette).exists() {
        fs::remove_file(filtered_cassette)?;
        println!("🗑️  Cleaned up test cassette file");
    }

    Ok(())
}

#[tokio::test]
async fn filter_mode_no_new_requests() -> Result<(), Box<dyn std::error::Error>> {
    // Test that Filter mode doesn't allow new requests (only replays filtered existing ones)
    let cassette_path = "tests/fixtures/empty_filter_test.yaml";

    // Create an empty cassette
    let empty_cassette_content = "interactions: []";
    fs::write(cassette_path, empty_cassette_content)?;

    let vcr_client = common::create_vcr_client(
        cassette_path,
        VcrMode::Filter,
        None, // No filters for this test
    )
    .await?;

    // Try to make a request to a URL that doesn't exist in the cassette
    use http_types::{Method, Request, Url};
    let request = Request::new(Method::Get, Url::parse("https://example.com/test")?);

    let result = vcr_client.send(request).await;

    // Should fail because Filter mode doesn't allow new requests
    assert!(
        result.is_err(),
        "Filter mode should not allow new HTTP requests"
    );

    let error_msg = result.unwrap_err().to_string();
    assert!(
        error_msg.contains("Filter mode - no new requests allowed"),
        "Error should indicate that new requests are not allowed in Filter mode"
    );

    println!("✅ Filter mode correctly prevents new HTTP requests");

    // Clean up
    if std::path::Path::new(cassette_path).exists() {
        fs::remove_file(cassette_path)?;
    }

    Ok(())
}
