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
        log::info!("🔐 Starting Last.fm login for username: {username}");

        // Step 1: Fetch login page and extract CSRF token and cookies
        let login_url = format!("{}/login", self.base_url);
        let (csrf_token, next_field, mut cookies) = self.fetch_login_page(&login_url).await?;

        // Step 2: Submit login form
        let response = self
            .submit_login_form(
                &login_url,
                username,
                password,
                &csrf_token,
                &next_field,
                &cookies,
            )
            .await?;

        // Step 3: Extract cookies from login response
        extract_cookies_from_response(&response, &mut cookies);
        log::debug!("🍪 Cookies after login response: {cookies:?}");

        // Step 4: Validate login response
        self.validate_login_response(response, username, cookies, csrf_token)
            .await
    }

    /// Fetch the login page and extract CSRF token, next field, and cookies
    async fn fetch_login_page(
        &self,
        login_url: &str,
    ) -> Result<(String, Option<String>, Vec<String>)> {
        log::debug!("📡 Fetching login page: {login_url}");
        let mut response = self.get(login_url).await?;

        log::debug!("📋 Login page response status: {}", response.status());
        log::debug!(
            "📋 Login page response headers: {:?}",
            response.iter().collect::<Vec<_>>()
        );

        // Extract cookies from the login page response
        let mut cookies = Vec::new();
        extract_cookies_from_response(&response, &mut cookies);
        log::debug!("🍪 Initial cookies from login page: {cookies:?}");

        // Read and parse the HTML response
        let html = response
            .body_string()
            .await
            .map_err(|e| LastFmError::Http(e.to_string()))?;

        log::debug!("📄 Login page HTML length: {} chars", html.len());
        if html.len() < 500 {
            log::debug!("📄 Login page HTML content (short): {html}");
        }

        // Extract CSRF token and next field from form
        let (csrf_token, next_field) = self.extract_login_form_data(&html)?;
        log::debug!("🔑 Extracted CSRF token: {csrf_token}",);
        log::debug!("➡️  Next field: {next_field:?}");

        Ok((csrf_token, next_field, cookies))
    }

    /// Submit the login form with credentials
    async fn submit_login_form(
        &self,
        login_url: &str,
        username: &str,
        password: &str,
        csrf_token: &str,
        next_field: &Option<String>,
        cookies: &[String],
    ) -> Result<http_types::Response> {
        // Prepare form data
        let mut form_data = HashMap::new();
        form_data.insert("csrfmiddlewaretoken", csrf_token);
        form_data.insert("username_or_email", username);
        form_data.insert("password", password);

        if let Some(ref next_value) = next_field {
            form_data.insert("next", next_value);
            log::debug!("➡️  Including next field in form: {next_value}");
        }

        log::debug!(
            "📝 Form data fields: {:?}",
            form_data.keys().collect::<Vec<_>>()
        );
        log::debug!("📝 Form username: {username}");
        log::debug!("📝 Form password length: {} chars", password.len());

        // Create and configure the POST request
        let mut request = self.create_login_request(login_url, cookies)?;

        // Convert form data to URL-encoded string
        let form_string: String = form_data
            .iter()
            .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
            .collect::<Vec<_>>()
            .join("&");

        log::debug!("📤 Sending POST request to: {login_url}");
        log::debug!("📤 Form body length: {} chars", form_string.len());
        log::debug!("📤 Form body (masked): {}", form_string);
        log::debug!("📤 Request headers: Referer={}, Origin={}, Content-Type=application/x-www-form-urlencoded", 
            login_url, &self.base_url);

        request.set_body(form_string);

        // Send the request
        let response = self
            .client
            .send(request)
            .await
            .map_err(|e| LastFmError::Http(e.to_string()))?;

        log::debug!("📥 Login response status: {}", response.status());
        log::debug!(
            "📥 Login response headers: {:?}",
            response.iter().collect::<Vec<_>>()
        );

        Ok(response)
    }

    /// Create and configure the login POST request with all necessary headers
    fn create_login_request(&self, login_url: &str, cookies: &[String]) -> Result<Request> {
        let mut request = Request::new(Method::Post, login_url.parse::<Url>().unwrap());

        // Set all the required headers
        let _ = request.insert_header("Referer", login_url);
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

        // Add cookies if we have any
        if !cookies.is_empty() {
            let cookie_header = cookies.join("; ");
            let _ = request.insert_header("Cookie", &cookie_header);
        }

        Ok(request)
    }

    /// Validate the login response and return a session if successful
    async fn validate_login_response(
        &self,
        mut response: http_types::Response,
        username: &str,
        cookies: Vec<String>,
        csrf_token: String,
    ) -> Result<LastFmEditSession> {
        // Handle 403 Forbidden responses (likely CSRF failures)
        if response.status() == 403 {
            return self.handle_403_response(response).await;
        }

        // Check for successful session establishment
        if let Some(session) =
            self.check_session_success(&response, username, &cookies, &csrf_token)
        {
            return Ok(session);
        }

        // For other cases, analyze the response body
        let response_html = response
            .body_string()
            .await
            .map_err(|e| LastFmError::Http(e.to_string()))?;

        log::debug!(
            "📄 Login response HTML length: {} chars",
            response_html.len()
        );
        if response_html.len() < 500 {
            log::debug!("📄 Login response HTML content (short): {response_html}");
        }

        // Check if we were redirected away from login page (success indicator)
        let has_login_form = self.check_for_login_form(&response_html);
        log::debug!("🔍 Final login validation:");
        log::debug!("   - Response contains login form: {has_login_form}");
        log::debug!("   - Response status: {}", response.status());

        if !has_login_form && response.status() == 200 {
            log::info!("✅ Login successful - no login form detected in response");
            Ok(LastFmEditSession::new(
                username.to_string(),
                cookies,
                Some(csrf_token),
                self.base_url.clone(),
            ))
        } else {
            // Parse and return error message
            let error_msg = self.parse_login_error(&response_html);
            log::warn!("❌ Login failed: {error_msg}");
            Err(LastFmError::Auth(error_msg))
        }
    }

    /// Handle 403 Forbidden responses
    async fn handle_403_response(
        &self,
        mut response: http_types::Response,
    ) -> Result<LastFmEditSession> {
        let response_html = response
            .body_string()
            .await
            .map_err(|e| LastFmError::Http(e.to_string()))?;

        log::debug!("📄 403 response HTML length: {} chars", response_html.len());
        if response_html.len() < 2000 {
            log::debug!("📄 403 response HTML content: {response_html}");
        } else {
            // Log first and last 500 chars for large responses
            log::debug!("📄 403 response HTML start: {}", &response_html[..500]);
            log::debug!(
                "📄 403 response HTML end: {}",
                &response_html[response_html.len() - 500..]
            );
        }

        let login_error = self.parse_login_error(&response_html);
        Err(LastFmError::Auth(login_error))
    }

    /// Check if the response indicates successful session establishment
    fn check_session_success(
        &self,
        response: &http_types::Response,
        username: &str,
        cookies: &[String],
        csrf_token: &str,
    ) -> Option<LastFmEditSession> {
        let has_real_session = cookies
            .iter()
            .any(|cookie| cookie.starts_with("sessionid=.") && cookie.len() > 50);

        log::debug!("🔍 Session validation:");
        log::debug!("   - Has real session cookie: {has_real_session}");
        log::debug!("   - Response status: {}", response.status());
        log::debug!("   - All cookies: {cookies:?}");

        if has_real_session && (response.status() == 302 || response.status() == 200) {
            log::info!("✅ Login successful - authenticated session established");
            Some(LastFmEditSession::new(
                username.to_string(),
                cookies.to_vec(),
                Some(csrf_token.to_string()),
                self.base_url.clone(),
            ))
        } else {
            None
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

        let csrf_token = document
            .select(&csrf_selector)
            .next()
            .and_then(|input| input.value().attr("value"))
            .map(|token| token.to_string())
            .ok_or(LastFmError::CsrfNotFound)?;

        log::debug!("🔑 CSRF token extracted from HTML: {csrf_token}");
        Ok(csrf_token)
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

/// Mask password in form data for safe logging
fn mask_password_in_form(form_data: &str) -> String {
    form_data
        .split('&')
        .map(|field| {
            if field.starts_with("password=") {
                "password=***MASKED***".to_string()
            } else {
                field.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("&")
}
