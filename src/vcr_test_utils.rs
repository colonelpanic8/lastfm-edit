use crate::vcr_form_data::{encode_form_data, parse_form_data};
use http_client_vcr::{Filter, FilterChain, SerializableRequest, SerializableResponse};

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
    let filter_chain = FilterChain::new().add_filter(Box::new(LastFmPasswordOnlyFilter));
    Ok(filter_chain)
}
