use scraper::{Html, Selector};

#[tokio::test]
async fn debug_simple_csrf_in_cassette() -> Result<(), Box<dyn std::error::Error>> {
    println!("🔍 Debug Simple CSRF - Examining cassette HTML directly");

    // Load the cassette and extract the HTML response body
    let cassette_content = std::fs::read_to_string("tests/fixtures/login_recent_tracks.yaml")?;

    println!("📄 Cassette loaded ({} chars)", cassette_content.len());

    // Find the body section in the YAML
    if let Some(body_start) = cassette_content.find("body: ") {
        let body_section = &cassette_content[body_start + 6..];

        // Find the end of the body section (next line that doesn't start with space)
        let body_end = body_section.find("\n  ").unwrap_or(body_section.len());
        let body_raw = &body_section[..body_end].trim();

        println!("📦 Raw body found ({} chars)", body_raw.len());

        // URL decode the body
        let body_decoded = urlencoding::decode(body_raw)?;
        println!("🔓 Body decoded ({} chars)", body_decoded.len());

        // Check for CSRF patterns
        let patterns = ["csrfmiddlewaretoken", "csrf", "token"];

        for pattern in &patterns {
            let count = body_decoded.matches(pattern).count();
            println!("  Pattern '{pattern}': {count} matches");
        }

        // Try to parse with scraper
        println!("🔧 Parsing HTML with scraper...");
        let document = Html::parse_document(&body_decoded);

        // Try the CSRF selector
        let csrf_selector = Selector::parse("input[name=\"csrfmiddlewaretoken\"]").unwrap();
        let csrf_inputs: Vec<_> = document.select(&csrf_selector).collect();

        println!("🎯 Found {} CSRF input elements", csrf_inputs.len());

        for (i, input) in csrf_inputs.iter().enumerate() {
            println!("  Input {}: {:?}", i, input.value());
            if let Some(value) = input.value().attr("value") {
                println!("    CSRF Token: '{value}'");
            }
        }

        // If no CSRF found, look for any input elements
        if csrf_inputs.is_empty() {
            println!("🔍 No CSRF inputs found, looking for any input elements...");
            let any_input_selector = Selector::parse("input").unwrap();
            let all_inputs: Vec<_> = document.select(&any_input_selector).collect();

            println!("  Found {} total input elements", all_inputs.len());
            for (i, input) in all_inputs.iter().take(10).enumerate() {
                println!("    Input {}: {:?}", i, input.value());
            }
        }

        // Show snippet around any csrf mention
        if body_decoded.contains("csrf") {
            println!("📋 Snippet around 'csrf':");
            if let Some(pos) = body_decoded.find("csrf") {
                let start = pos.saturating_sub(100);
                let end = std::cmp::min(pos + 200, body_decoded.len());
                println!("  ...{}...", &body_decoded[start..end]);
            }
        }
    } else {
        println!("❌ No body section found in cassette");
    }

    Ok(())
}
