use crate::vcr_form_data::{encode_form_data, parse_form_data};
use http_client::Error;
use http_client_vcr::{
    analyze_cassette_file, mutate_all_interactions, Cassette, CassetteAnalysis, Filter,
    FilterChain, SerializableRequest, SerializableResponse,
};
use std::path::PathBuf;

/// Last.fm-specific cassette analysis results
#[derive(Debug)]
pub struct LastFmCassetteAnalysis {
    pub base_analysis: CassetteAnalysis,
    pub has_login_flow: bool,
    pub username_found: Option<String>,
    pub password_filtered: bool,
    pub session_data_filtered: bool,
}

/// Last.fm-specific utilities for test cassette management.
/// These helpers understand the Last.fm authentication flow and handle credential replacement appropriately.
/// A custom filter that only filters passwords while preserving usernames and CSRF tokens
#[derive(Debug)]
pub struct LastFmPasswordOnlyFilter;

impl Filter for LastFmPasswordOnlyFilter {
    fn filter_request(&self, request: &mut SerializableRequest) {
        // Filter passwords in form data
        if let Some(body) = &mut request.body {
            if body.contains('=') && (body.contains('&') || !body.contains(' ')) {
                let mut params = parse_form_data(body);

                // Only filter password field - keep username and CSRF token for request matching
                if params.contains_key("password") {
                    params.insert("password".to_string(), "test_password".to_string());
                    *body = encode_form_data(&params);
                }
            }
        }

        // Filter session cookies in request headers
        if let Some(cookie_values) = request.headers.get_mut("cookie") {
            for cookie_header in cookie_values.iter_mut() {
                *cookie_header = self.filter_session_cookies(cookie_header);
            }
        }
    }

    fn filter_response(&self, response: &mut SerializableResponse) {
        // Filter session cookies in response set-cookie headers
        if let Some(set_cookie_values) = response.headers.get_mut("set-cookie") {
            for set_cookie_header in set_cookie_values.iter_mut() {
                if set_cookie_header.contains("sessionid=") {
                    *set_cookie_header = self.filter_set_cookie_session(set_cookie_header);
                }
            }
        }
    }
}

impl LastFmPasswordOnlyFilter {
    /// Filter session cookies from a cookie header string
    fn filter_session_cookies(&self, cookie_header: &str) -> String {
        let mut filtered_cookies = Vec::new();

        for cookie in cookie_header.split(';') {
            let cookie = cookie.trim();
            if cookie.starts_with("sessionid=") {
                // Replace session ID with test value
                filtered_cookies.push("sessionid=test_session_id");
            } else {
                filtered_cookies.push(cookie);
            }
        }

        filtered_cookies.join("; ")
    }

    /// Filter session cookies from a set-cookie header string
    fn filter_set_cookie_session(&self, set_cookie_header: &str) -> String {
        if set_cookie_header.starts_with("sessionid=") {
            // Extract everything after the session value up to the first semicolon or end
            if let Some(semicolon_pos) = set_cookie_header.find(';') {
                let attributes = &set_cookie_header[semicolon_pos..];
                format!("sessionid=test_session_id{attributes}")
            } else {
                "sessionid=test_session_id".to_string()
            }
        } else {
            set_cookie_header.to_string()
        }
    }
}

/// Create a Last.fm test filter chain that:
/// - Keeps usernames and CSRF tokens intact (needed for proper request matching)
/// - Filters passwords from request bodies with predictable test values
/// - Filters session tokens with predictable test values
pub fn create_lastfm_test_filter_chain() -> Result<FilterChain, regex::Error> {
    println!("🔧 Creating test filter chain - filtering passwords and session cookies");

    let filter_chain = FilterChain::new().add_filter(Box::new(LastFmPasswordOnlyFilter));

    println!("✅ Created filter chain with 1 custom filter (passwords + session cookies)");
    Ok(filter_chain)
}

/// Apply Last.fm test-specific filtering to a cassette
/// This preserves usernames and CSRF tokens but filters passwords and session data
pub async fn prepare_lastfm_test_cassette<P: Into<PathBuf>>(cassette_path: P) -> Result<(), Error> {
    let path = cassette_path.into();

    println!("🧪 Preparing Last.fm test cassette: {path:?}");

    // Apply Last.fm test-specific mutations
    mutate_all_interactions(
        &path,
        |request| {
            // Filter passwords from form data while keeping usernames and CSRF tokens
            if let Some(body) = &mut request.body {
                if body.contains('=') && (body.contains('&') || !body.contains(' ')) {
                    let mut params = parse_form_data(body);

                    // Filter password only - keep username and CSRF token
                    if params.contains_key("password") {
                        params.insert("password".to_string(), "test_password".to_string());
                    }

                    *body = encode_form_data(&params);
                }
            }
        },
        |response| {
            // Filter session data from response bodies and headers
            if let Some(body) = &mut response.body {
                // Replace session IDs with predictable test values
                *body = regex::Regex::new(r"sessionid=[^;,\s]+")
                    .unwrap()
                    .replace_all(body, "sessionid=test_session_id")
                    .to_string();

                // Handle JSON session data
                if body.contains(r#""sessionid""#) {
                    *body = regex::Regex::new(r#""sessionid":"[^"]+""#)
                        .unwrap()
                        .replace_all(body, r#""sessionid":"test_session_id""#)
                        .to_string();
                }
            }

            // Filter session data from headers
            let session_regex = regex::Regex::new(r"sessionid=[^;,\s]+").unwrap();
            for (_, values) in response.headers.iter_mut() {
                for value in values.iter_mut() {
                    *value = session_regex
                        .replace_all(value, "sessionid=test_session_id")
                        .to_string();
                }
            }
        },
    )
    .await?;

    println!("✅ Last.fm test cassette prepared successfully");
    println!("   - Usernames and CSRF tokens preserved for proper request matching");
    println!("   - Passwords filtered to predictable test values");
    println!("   - Session tokens filtered to predictable test values");

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
