use http_client::Request;
use http_client_vcr::{RequestMatcher, SerializableRequest};
use std::fmt::Debug;

/// Last.fm-specific matcher that handles authentication flows properly
/// Ignores cookies and session-related headers that change between test runs
#[derive(Debug)]
pub struct LastFmMatcher {
    match_method: bool,
    match_url: bool,
    match_body: bool,
}

impl LastFmMatcher {
    pub fn new() -> Self {
        Self {
            match_method: true,
            match_url: true,
            match_body: false,
        }
    }

    pub fn with_body(mut self, match_body: bool) -> Self {
        self.match_body = match_body;
        self
    }
}

impl RequestMatcher for LastFmMatcher {
    fn matches(&self, request: &Request, recorded_request: &SerializableRequest) -> bool {
        if self.match_method && request.method().to_string() != recorded_request.method {
            return false;
        }

        if self.match_url && request.url().to_string() != recorded_request.url {
            return false;
        }

        // For Last.fm, we explicitly ignore unstable headers that change between sessions
        // This includes cookies, session tokens, CSRF tokens, and other ephemeral data
        let unstable_headers = [
            "cookie",
            "set-cookie",
            "authorization",
            "x-csrf-token",
            "csrf-token",
            "sessionid",
            "session",
            "x-session-id",
            "x-auth-token",
            "auth-token",
        ];

        // Match on all headers EXCEPT the unstable ones
        for (header_name, recorded_values) in &recorded_request.headers {
            let header_lower = header_name.to_lowercase();

            // Skip unstable headers
            if unstable_headers
                .iter()
                .any(|unstable| header_lower.contains(unstable))
            {
                continue;
            }

            let request_header = request.header(header_name.as_str());

            match request_header {
                Some(req_val) => {
                    let req_values: Vec<String> =
                        req_val.iter().map(|v| v.as_str().to_string()).collect();
                    if &req_values != recorded_values {
                        return false;
                    }
                }
                None => {
                    // If the recorded request has a header but the current request doesn't,
                    // that's a mismatch (unless it's an unstable header we're ignoring)
                    return false;
                }
            }
        }

        true
    }

    fn matches_serializable(
        &self,
        request: &SerializableRequest,
        recorded_request: &SerializableRequest,
    ) -> bool {
        if self.match_method && request.method != recorded_request.method {
            return false;
        }

        if self.match_url && request.url != recorded_request.url {
            return false;
        }

        // Same logic as above - ignore unstable headers
        let unstable_headers = [
            "cookie",
            "set-cookie",
            "authorization",
            "x-csrf-token",
            "csrf-token",
            "sessionid",
            "session",
            "x-session-id",
            "x-auth-token",
            "auth-token",
        ];

        // Match on all headers EXCEPT the unstable ones
        for (header_name, recorded_values) in &recorded_request.headers {
            let header_lower = header_name.to_lowercase();

            // Skip unstable headers
            if unstable_headers
                .iter()
                .any(|unstable| header_lower.contains(unstable))
            {
                continue;
            }

            let request_header = request.headers.get(header_name);

            match request_header {
                Some(req_values) => {
                    if req_values != recorded_values {
                        return false;
                    }
                }
                None => {
                    // If the recorded request has a header but the current request doesn't,
                    // that's a mismatch (unless it's an unstable header we're ignoring)
                    return false;
                }
            }
        }

        true
    }
}

impl Default for LastFmMatcher {
    fn default() -> Self {
        Self::new()
    }
}
