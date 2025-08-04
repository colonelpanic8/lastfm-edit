use crate::vcr_form_data::{encode_form_data, parse_form_data};
use http_client::Error;
use http_client_vcr::{
    analyze_cassette_file, mutate_all_interactions, BodyFilter, Cassette, CassetteAnalysis,
    FilterChain,
};
use std::path::PathBuf;

/// Last.fm-specific utilities for test cassette management.
/// These helpers understand the Last.fm authentication flow and handle credential replacement appropriately.
/// Create a Last.fm test filter chain that:
/// - Keeps usernames intact (needed for proper request matching and URI generation)
/// - Filters passwords from request bodies
/// - Filters session data from response bodies
/// - Ignores cookies for matching simplicity
pub fn create_lastfm_test_filter_chain() -> Result<FilterChain, regex::Error> {
    let filter_chain = FilterChain::new()
        // Filter passwords from request bodies while preserving usernames
        .add_filter(Box::new(
            BodyFilter::new()
                // Only filter password fields - keep username intact
                .replace_regex(r"password=[^&]+", "password=[FILTERED_PASSWORD]")?,
        ))
        // Filter sensitive session data from response bodies
        .add_filter(Box::new(
            BodyFilter::new()
                // Session IDs in response bodies (JSON, HTML, etc.)
                .replace_regex(r"sessionid=[^;,\s]+", "sessionid=[FILTERED_SESSION]")?
                // CSRF tokens in response bodies
                .replace_regex(r"csrftoken=[^;,\s]+", "csrftoken=[FILTERED_CSRF]")?
                // Session data in JSON responses
                .replace_regex(
                    r#""sessionid":"[^"]+""#,
                    r#""sessionid":"[FILTERED_SESSION]""#,
                )?
                .replace_regex(r#""csrftoken":"[^"]+""#, r#""csrftoken":"[FILTERED_CSRF]""#)?,
        ));

    Ok(filter_chain)
}

/// Apply Last.fm test-specific filtering to a cassette
/// This preserves usernames but filters passwords and session data
pub async fn prepare_lastfm_test_cassette<P: Into<PathBuf>>(cassette_path: P) -> Result<(), Error> {
    let path = cassette_path.into();

    println!("🧪 Preparing Last.fm test cassette: {path:?}");

    // Apply Last.fm test-specific mutations
    mutate_all_interactions(
        &path,
        |request| {
            // Filter passwords from form data while keeping usernames
            if let Some(body) = &mut request.body {
                if body.contains('=') && (body.contains('&') || !body.contains(' ')) {
                    let mut params = parse_form_data(body);

                    // Filter password but keep username
                    if params.contains_key("password") {
                        params.insert("password".to_string(), "[FILTERED_PASSWORD]".to_string());
                    }

                    *body = encode_form_data(&params);
                }
            }
        },
        |response| {
            // Filter session data from response bodies
            if let Some(body) = &mut response.body {
                // Replace session IDs in various formats
                *body = body.replace(r"sessionid=", "sessionid=[FILTERED_SESSION];");

                // Replace CSRF tokens
                if body.contains("csrftoken") {
                    // Handle different formats of CSRF tokens
                    *body = regex::Regex::new(r"csrftoken=[^;,\s]+")
                        .unwrap()
                        .replace_all(body, "csrftoken=[FILTERED_CSRF]")
                        .to_string();
                }

                // Handle JSON session data
                if body.contains(r#""sessionid""#) {
                    *body = regex::Regex::new(r#""sessionid":"[^"]+""#)
                        .unwrap()
                        .replace_all(body, r#""sessionid":"[FILTERED_SESSION]""#)
                        .to_string();
                }
            }
        },
    )
    .await?;

    println!("✅ Last.fm test cassette prepared successfully");
    println!("   - Usernames preserved for proper request matching");
    println!("   - Passwords filtered for security");
    println!("   - Session data filtered for security");

    Ok(())
}

/// Replace the password in all requests with a test password
/// This is useful when you want to use a known test password for replay
pub async fn set_test_password_in_cassette<P: Into<PathBuf>>(
    cassette_path: P,
    test_password: &str,
) -> Result<(), Error> {
    let path = cassette_path.into();
    let password = test_password.to_string();

    println!("🔑 Setting test password in cassette: {path:?}");

    http_client_vcr::mutate_all_requests(&path, move |request| {
        if let Some(body) = &mut request.body {
            if body.contains('=') && (body.contains('&') || !body.contains(' ')) {
                let mut params = parse_form_data(body);

                if params.contains_key("password") {
                    params.insert("password".to_string(), password.clone());
                    *body = encode_form_data(&params);
                }
            }
        }
    })
    .await?;

    println!("✅ Test password set in cassette");
    Ok(())
}

/// Get the username from a cassette (useful for test setup)
/// Returns the first username found in form data
pub async fn extract_username_from_cassette<P: Into<PathBuf>>(
    cassette_path: P,
) -> Result<Option<String>, Error> {
    let path = cassette_path.into();
    let cassette = Cassette::load_from_file(path).await?;

    for interaction in &cassette.interactions {
        if let Some(body) = &interaction.request.body {
            if body.contains('=') && (body.contains('&') || !body.contains(' ')) {
                let params = parse_form_data(body);

                // Look for common username fields
                let username_fields = ["username", "username_or_email", "user", "email"];
                for field in &username_fields {
                    if let Some(username) = params.get(*field) {
                        // Skip filtered values
                        if !username.starts_with("[FILTERED") && !username.starts_with("[SANITIZED")
                        {
                            return Ok(Some(username.clone()));
                        }
                    }
                }
            }
        }
    }

    Ok(None)
}

/// Analyze a Last.fm cassette and provide specific recommendations
pub async fn analyze_lastfm_test_cassette<P: Into<PathBuf>>(
    cassette_path: P,
) -> Result<LastFmCassetteAnalysis, Error> {
    let path = cassette_path.into();
    let base_analysis = analyze_cassette_file(&path).await?;

    let mut lastfm_analysis = LastFmCassetteAnalysis {
        base_analysis,
        has_login_flow: false,
        username_found: None,
        password_filtered: false,
        session_data_filtered: false,
    };

    // Check for login flow patterns
    let cassette = Cassette::load_from_file(path.clone()).await?;

    for interaction in &cassette.interactions {
        // Check for login URLs
        if interaction.request.url.contains("/login") || interaction.request.url.contains("/signin")
        {
            lastfm_analysis.has_login_flow = true;
        }

        // Check for username in form data
        if let Some(body) = &interaction.request.body {
            if body.contains('=') {
                let params = parse_form_data(body);

                // Look for username
                let username_fields = ["username", "username_or_email", "user", "email"];
                for field in &username_fields {
                    if let Some(username) = params.get(*field) {
                        if !username.starts_with("[FILTERED") && !username.starts_with("[SANITIZED")
                        {
                            lastfm_analysis.username_found = Some(username.clone());
                        }
                    }
                }

                // Check if password is filtered
                if let Some(password) = params.get("password") {
                    lastfm_analysis.password_filtered =
                        password.contains("[FILTERED") || password.contains("[SANITIZED");
                }
            }
        }

        // Check if session data is filtered in responses
        if let Some(body) = &interaction.response.body {
            if body.contains("sessionid") {
                lastfm_analysis.session_data_filtered =
                    body.contains("[FILTERED_SESSION]") || body.contains("[SANITIZED");
            }
        }
    }

    Ok(lastfm_analysis)
}

#[derive(Debug)]
pub struct LastFmCassetteAnalysis {
    pub base_analysis: CassetteAnalysis,
    pub has_login_flow: bool,
    pub username_found: Option<String>,
    pub password_filtered: bool,
    pub session_data_filtered: bool,
}

impl LastFmCassetteAnalysis {
    pub fn print_lastfm_report(&self) {
        println!("🎵 Last.fm Cassette Analysis Report");
        println!("====================================");
        self.base_analysis.print_report();

        println!();
        println!("🎵 Last.fm Specific Analysis:");
        println!(
            "  Login flow detected: {}",
            if self.has_login_flow {
                "✅ Yes"
            } else {
                "❌ No"
            }
        );

        match &self.username_found {
            Some(username) => {
                println!("  Username found: ✅ {username} (preserved for request matching)")
            }
            None => println!("  Username found: ❌ None"),
        }

        println!(
            "  Password filtered: {}",
            if self.password_filtered {
                "✅ Yes"
            } else {
                "❌ No"
            }
        );
        println!(
            "  Session data filtered: {}",
            if self.session_data_filtered {
                "✅ Yes"
            } else {
                "❌ No"
            }
        );

        println!();
        println!("💡 Last.fm Test Recommendations:");

        if !self.has_login_flow {
            println!("  - This cassette doesn't appear to contain login flow");
        }

        if self.username_found.is_some() && !self.password_filtered {
            println!("  - ⚠️  Password should be filtered for security");
            println!("    Run: prepare_lastfm_test_cassette() on this file");
        }

        if self.has_login_flow && !self.session_data_filtered {
            println!("  - ⚠️  Session data should be filtered for security");
            println!("    Run: prepare_lastfm_test_cassette() on this file");
        }

        if self.username_found.is_some() && self.password_filtered && self.session_data_filtered {
            println!("  - ✅ Cassette is properly prepared for Last.fm tests");
            println!("    Username preserved, credentials filtered");
        }
    }
}
