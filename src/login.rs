use crate::types::{LastFmEditSession, LastFmError};
use crate::Result;
use http_client::{HttpClient, Request};
use http_types::{Method, Url};
use scraper::{Html, Selector};
use std::collections::HashMap;
use std::sync::Arc;

/// Login functionality separated from the main client
pub struct LoginManager {
    client: Arc<dyn HttpClient + Send + Sync>,
    base_url: String,
}

impl LoginManager {
    pub fn new(client: Arc<dyn HttpClient + Send + Sync>, base_url: String) -> Self {
        Self { client, base_url }
    }

    /// Authenticate with Last.fm using username and password.
    ///
    /// This method:
    /// 1. Fetches the login page to extract CSRF tokens
    /// 2. Submits the login form with credentials
    /// 3. Validates the authentication by checking for session cookies
    /// 4. Returns a valid session for use with the client
    ///
    /// # Arguments
    ///
    /// * `username` - Last.fm username or email
    /// * `password` - Last.fm password
    ///
    /// # Returns
    ///
    /// Returns a [`LastFmEditSession`] on successful authentication, or [`LastFmError::Auth`] on failure.
    pub async fn login(&self, username: &str, password: &str) -> Result<LastFmEditSession> {
        // Get login page to extract CSRF token
        let login_url = format!("{}/login", self.base_url);
        let mut response = self.get(&login_url).await?;

        // Extract any initial cookies from the login page
        let mut cookies = Vec::new();
        extract_cookies_from_response(&response, &mut cookies);

        let html = response
            .body_string()
            .await
            .map_err(|e| LastFmError::Http(e.to_string()))?;

        // Parse HTML to extract login form data
        let (csrf_token, next_field) = self.extract_login_form_data(&html)?;

        // Submit login form
        let mut form_data = HashMap::new();
        form_data.insert("csrfmiddlewaretoken", csrf_token.as_str());
        form_data.insert("username_or_email", username);
        form_data.insert("password", password);

        // Add 'next' field if present
        if let Some(ref next_value) = next_field {
            form_data.insert("next", next_value);
        }

        let mut request = Request::new(Method::Post, login_url.parse::<Url>().unwrap());
        let _ = request.insert_header("Referer", &login_url);
        let _ = request.insert_header("Origin", &self.base_url);
        let _ = request.insert_header("Content-Type", "application/x-www-form-urlencoded");
        let _ = request.insert_header(
            "User-Agent",
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/138.0.0.0 Safari/537.36"
        );
        let _ = request.insert_header(
            "Accept",
            "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7"
        );
        let _ = request.insert_header("Accept-Language", "en-US,en;q=0.9");
        let _ = request.insert_header("Accept-Encoding", "gzip, deflate, br");
        let _ = request.insert_header("DNT", "1");
        let _ = request.insert_header("Connection", "keep-alive");
        let _ = request.insert_header("Upgrade-Insecure-Requests", "1");
        let _ = request.insert_header(
            "sec-ch-ua",
            "\"Not)A;Brand\";v=\"8\", \"Chromium\";v=\"138\", \"Google Chrome\";v=\"138\"",
        );
        let _ = request.insert_header("sec-ch-ua-mobile", "?0");
        let _ = request.insert_header("sec-ch-ua-platform", "\"Linux\"");
        let _ = request.insert_header("Sec-Fetch-Dest", "document");
        let _ = request.insert_header("Sec-Fetch-Mode", "navigate");
        let _ = request.insert_header("Sec-Fetch-Site", "same-origin");
        let _ = request.insert_header("Sec-Fetch-User", "?1");

        // Add any cookies we already have
        if !cookies.is_empty() {
            let cookie_header = cookies.join("; ");
            let _ = request.insert_header("Cookie", &cookie_header);
        }

        // Convert form data to URL-encoded string
        let form_string: String = form_data
            .iter()
            .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
            .collect::<Vec<_>>()
            .join("&");

        request.set_body(form_string);

        let mut response = self
            .client
            .send(request)
            .await
            .map_err(|e| LastFmError::Http(e.to_string()))?;

        // Extract session cookies from login response
        extract_cookies_from_response(&response, &mut cookies);

        log::debug!("Login response status: {}", response.status());

        // If we get a 403, it's likely an auth failure
        if response.status() == 403 {
            let response_html = response
                .body_string()
                .await
                .map_err(|e| LastFmError::Http(e.to_string()))?;

            let login_error = self.parse_login_error(&response_html);
            return Err(LastFmError::Auth(login_error));
        }

        // Check if we got a new sessionid that looks like a real Last.fm session
        let has_real_session = cookies
            .iter()
            .any(|cookie| cookie.starts_with("sessionid=.") && cookie.len() > 50);

        if has_real_session && (response.status() == 302 || response.status() == 200) {
            // We got a real session ID, login was successful
            log::debug!("Login successful - authenticated session established");
            return Ok(LastFmEditSession::new(
                username.to_string(),
                cookies,
                Some(csrf_token),
                self.base_url.clone(),
            ));
        }

        // At this point, we didn't get a 403, so read the response body for other cases
        let response_html = response
            .body_string()
            .await
            .map_err(|e| LastFmError::Http(e.to_string()))?;

        // Check if we were redirected away from login page (success) by parsing
        let has_login_form = self.check_for_login_form(&response_html);

        if !has_login_form && response.status() == 200 {
            Ok(LastFmEditSession::new(
                username.to_string(),
                cookies,
                Some(csrf_token),
                self.base_url.clone(),
            ))
        } else {
            // Parse error messages
            let error_msg = self.parse_login_error(&response_html);
            Err(LastFmError::Auth(error_msg))
        }
    }

    /// Make a simple HTTP GET request (without retry logic)
    async fn get(&self, url: &str) -> Result<http_types::Response> {
        let mut request = Request::new(Method::Get, url.parse::<Url>().unwrap());
        let _ = request.insert_header("User-Agent", "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/138.0.0.0 Safari/537.36");

        self.client
            .send(request)
            .await
            .map_err(|e| LastFmError::Http(e.to_string()))
    }

    /// Extract login form data (CSRF token and next field)
    fn extract_login_form_data(&self, html: &str) -> Result<(String, Option<String>)> {
        let document = Html::parse_document(html);

        let csrf_token = self.extract_csrf_token(&document)?;

        // Check if there's a 'next' field in the form
        let next_selector = Selector::parse("input[name=\"next\"]").unwrap();
        let next_field = document
            .select(&next_selector)
            .next()
            .and_then(|input| input.value().attr("value"))
            .map(|s| s.to_string());

        Ok((csrf_token, next_field))
    }

    fn extract_csrf_token(&self, document: &Html) -> Result<String> {
        let csrf_selector = Selector::parse("input[name=\"csrfmiddlewaretoken\"]").unwrap();

        document
            .select(&csrf_selector)
            .next()
            .and_then(|input| input.value().attr("value"))
            .map(|token| token.to_string())
            .ok_or(LastFmError::CsrfNotFound)
    }

    /// Parse login error messages from HTML
    fn parse_login_error(&self, html: &str) -> String {
        let document = Html::parse_document(html);

        let error_selector = Selector::parse(".alert-danger, .form-error, .error-message").unwrap();

        let mut error_messages = Vec::new();
        for error in document.select(&error_selector) {
            let error_text = error.text().collect::<String>().trim().to_string();
            if !error_text.is_empty() {
                error_messages.push(error_text);
            }
        }

        if error_messages.is_empty() {
            "Login failed - please check your credentials".to_string()
        } else {
            format!("Login failed: {}", error_messages.join("; "))
        }
    }

    /// Check if HTML contains a login form
    fn check_for_login_form(&self, html: &str) -> bool {
        let document = Html::parse_document(html);
        let login_form_selector =
            Selector::parse("form[action*=\"login\"], input[name=\"username_or_email\"]").unwrap();
        document.select(&login_form_selector).next().is_some()
    }
}

/// Extract cookies from HTTP response - utility function
pub fn extract_cookies_from_response(response: &http_types::Response, cookies: &mut Vec<String>) {
    if let Some(cookie_headers) = response.header("set-cookie") {
        for cookie_header in cookie_headers {
            let cookie_str = cookie_header.as_str();
            // Extract just the cookie name=value part (before any semicolon)
            if let Some(cookie_value) = cookie_str.split(';').next() {
                let cookie_name = cookie_value.split('=').next().unwrap_or("");

                // Remove any existing cookie with the same name
                cookies.retain(|existing| !existing.starts_with(&format!("{cookie_name}=")));
                cookies.push(cookie_value.to_string());
            }
        }
    }
}
