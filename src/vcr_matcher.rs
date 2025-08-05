use http_client::Request;
use http_client_vcr::{RequestMatcher, SerializableRequest};
use std::fmt::Debug;

/// Last.fm Edit VCR matcher that handles authentication flows properly
/// Ignores cookies and session-related headers that change between test runs
#[derive(Debug)]
pub struct LastFmEditVcrMatcher {
    match_method: bool,
    match_url: bool,
    match_body: bool,
}

impl LastFmEditVcrMatcher {
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

impl RequestMatcher for LastFmEditVcrMatcher {
    fn matches(&self, request: &Request, recorded_request: &SerializableRequest) -> bool {
        log::debug!(
            "Matching request: {} {} against recorded: {} {}",
            request.method(),
            request.url(),
            recorded_request.method,
            recorded_request.url
        );

        if self.match_method && request.method().to_string() != recorded_request.method {
            log::debug!(
                "Method mismatch: {} != {}",
                request.method(),
                recorded_request.method
            );
            return false;
        }

        if self.match_url && request.url().to_string() != recorded_request.url {
            log::debug!(
                "URL mismatch: {} != {}",
                request.url(),
                recorded_request.url
            );
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
        log::debug!("Checking headers (ignoring unstable ones)");
        for (header_name, recorded_values) in &recorded_request.headers {
            let header_lower = header_name.to_lowercase();

            // Skip unstable headers
            if unstable_headers
                .iter()
                .any(|unstable| header_lower.contains(unstable))
            {
                log::debug!("Skipping unstable header: {header_name}");
                continue;
            }

            let request_header = request.header(header_name.as_str());
            log::debug!(
                "Comparing stable header '{header_name}': request={:?}, recorded={recorded_values:?}",
                request_header.map(|v| v.iter().map(|h| h.as_str()).collect::<Vec<_>>())
            );

            match request_header {
                Some(req_val) => {
                    let req_values: Vec<String> =
                        req_val.iter().map(|v| v.as_str().to_string()).collect();
                    if &req_values != recorded_values {
                        log::debug!(
                            "Header '{header_name}' values mismatch: {req_values:?} != {recorded_values:?}"
                        );
                        return false;
                    }
                }
                None => {
                    // If the recorded request has a header but the current request doesn't,
                    // that's a mismatch (unless it's an unstable header we're ignoring)
                    log::debug!("Header '{header_name}' missing from request");
                    return false;
                }
            }
        }

        log::debug!("Request matched successfully");

        true
    }

    fn matches_serializable(
        &self,
        request: &SerializableRequest,
        recorded_request: &SerializableRequest,
    ) -> bool {
        log::debug!(
            "Matching serializable request: {} {} vs {} {}",
            request.method,
            request.url,
            recorded_request.method,
            recorded_request.url
        );
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
        log::debug!(
            "Checking {} recorded headers",
            recorded_request.headers.len()
        );
        for (header_name, recorded_values) in &recorded_request.headers {
            let header_lower = header_name.to_lowercase();

            // Skip unstable headers
            if unstable_headers
                .iter()
                .any(|unstable| header_lower.contains(unstable))
            {
                log::debug!("Skipping unstable header: {header_name}");
                continue;
            }

            log::debug!("Checking stable header: {header_name} = {recorded_values:?}");

            let request_header = request.headers.get(header_name);

            match request_header {
                Some(req_values) => {
                    log::debug!(
                        "Comparing header '{header_name}': request={req_values:?} vs recorded={recorded_values:?}"
                    );
                    if req_values != recorded_values {
                        log::debug!(
                            "Header '{header_name}' MISMATCH! request={req_values:?} != {recorded_values:?}"
                        );
                        return false;
                    }
                }
                None => {
                    // Some headers like content-type are automatically added by HTTP clients
                    // but may not be present during replay. For GET requests, content-type is often optional.
                    if header_name.to_lowercase() == "content-type" && request.method == "GET" {
                        log::debug!("Ignoring missing content-type header for GET request");
                        continue;
                    }

                    // If the recorded request has a header but the current request doesn't,
                    // that's a mismatch (unless it's an unstable header we're ignoring)
                    log::debug!(
                        "Header '{header_name}' missing from current request (recorded has: {recorded_values:?})"
                    );
                    return false;
                }
            }
        }

        true
    }
}

impl Default for LastFmEditVcrMatcher {
    fn default() -> Self {
        Self::new()
    }
}
