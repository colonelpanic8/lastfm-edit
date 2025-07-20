use crate::{ArtistTracksIterator, LastFmError, Result, Track, TrackPage};
use http_client::{HttpClient, Request, Response};
use http_types::{Method, Url};
use scraper::{Html, Selector};
use std::collections::HashMap;

pub struct LastFmClient {
    client: Box<dyn HttpClient>,
    username: String,
    csrf_token: Option<String>,
    base_url: String,
    session_cookies: Vec<String>,
    debug_enabled: bool,
    debug_callback: Option<Box<dyn Fn(&str) + Send + Sync>>,
}

impl LastFmClient {
    pub fn new(client: Box<dyn HttpClient>) -> Self {
        Self::with_base_url(client, "https://www.last.fm".to_string())
    }

    pub fn with_base_url(client: Box<dyn HttpClient>, base_url: String) -> Self {
        Self {
            client,
            username: String::new(),
            csrf_token: None,
            base_url,
            session_cookies: Vec::new(),
            debug_enabled: false,
            debug_callback: None,
        }
    }

    pub fn enable_debug(&mut self) {
        self.debug_enabled = true;
    }

    pub fn disable_debug(&mut self) {
        self.debug_enabled = false;
    }

    pub fn set_debug_callback<F>(&mut self, callback: F)
    where
        F: Fn(&str) + Send + Sync + 'static,
    {
        self.debug_callback = Some(Box::new(callback));
        self.debug_enabled = true;
    }

    fn debug_log(&self, message: &str) {
        if self.debug_enabled {
            if let Some(ref callback) = self.debug_callback {
                callback(message);
            } else {
                eprintln!("DEBUG: {}", message);
            }
        }
    }

    pub async fn login(&mut self, username: &str, password: &str) -> Result<()> {
        // Get login page to extract CSRF token
        let login_url = format!("{}/login", self.base_url);
        let mut response = self.get(&login_url).await?;

        // Extract any initial cookies from the login page
        self.extract_cookies(&response);

        let html = response
            .body_string()
            .await
            .map_err(|e| LastFmError::Http(e.to_string()))?;
        let document = Html::parse_document(&html);

        let csrf_token = self.extract_csrf_token(&document)?;

        // Submit login form
        let mut form_data = HashMap::new();
        form_data.insert("csrfmiddlewaretoken", csrf_token.as_str());
        form_data.insert("username_or_email", username);
        form_data.insert("password", password);

        // Check if there's a 'next' field in the form
        let next_selector = Selector::parse("input[name=\"next\"]").unwrap();
        if let Some(next_input) = document.select(&next_selector).next() {
            if let Some(next_value) = next_input.value().attr("value") {
                form_data.insert("next", next_value);
            }
        }

        let mut request = Request::new(Method::Post, login_url.parse::<Url>().unwrap());
        request.insert_header("Referer", &login_url);
        request.insert_header("Origin", &self.base_url);
        request.insert_header("Content-Type", "application/x-www-form-urlencoded");
        request.insert_header("User-Agent", "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/138.0.0.0 Safari/537.36");
        request.insert_header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7");
        request.insert_header("Accept-Language", "en-US,en;q=0.9");
        request.insert_header("Accept-Encoding", "gzip, deflate, br");
        request.insert_header("DNT", "1");
        request.insert_header("Connection", "keep-alive");
        request.insert_header("Upgrade-Insecure-Requests", "1");
        request.insert_header(
            "sec-ch-ua",
            "\"Not)A;Brand\";v=\"8\", \"Chromium\";v=\"138\", \"Google Chrome\";v=\"138\"",
        );
        request.insert_header("sec-ch-ua-mobile", "?0");
        request.insert_header("sec-ch-ua-platform", "\"Linux\"");
        request.insert_header("Sec-Fetch-Dest", "document");
        request.insert_header("Sec-Fetch-Mode", "navigate");
        request.insert_header("Sec-Fetch-Site", "same-origin");
        request.insert_header("Sec-Fetch-User", "?1");

        // Add any cookies we already have
        if !self.session_cookies.is_empty() {
            let cookie_header = self.session_cookies.join("; ");
            request.insert_header("Cookie", &cookie_header);
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
        self.extract_cookies(&response);

        self.debug_log(&format!("Login response status: {}", response.status()));

        // If we get a 403, login definitely failed
        if response.status() == 403 {
            return Err(LastFmError::Auth(
                "Login failed - 403 Forbidden. Check credentials.".to_string(),
            ));
        }

        // Check if we got a new sessionid that looks like a real Last.fm session
        let has_real_session = self
            .session_cookies
            .iter()
            .any(|cookie| cookie.starts_with("sessionid=.") && cookie.len() > 50);

        if has_real_session && (response.status() == 302 || response.status() == 200) {
            // We got a real session ID, login was successful
            self.username = username.to_string();
            self.csrf_token = Some(csrf_token);
            self.debug_log("Login successful - authenticated session established");
            return Ok(());
        }

        let response_html = response
            .body_string()
            .await
            .map_err(|e| LastFmError::Http(e.to_string()))?;

        // Check if we were redirected away from login page (success) by looking for login forms in response
        let success_doc = Html::parse_document(&response_html);
        let login_form_selector =
            Selector::parse("form[action*=\"login\"], input[name=\"username_or_email\"]").unwrap();
        let has_login_form = success_doc.select(&login_form_selector).next().is_some();

        if !has_login_form && response.status() == 200 {
            self.username = username.to_string();
            self.csrf_token = Some(csrf_token);
            Ok(())
        } else {
            // Parse the login page for specific error messages
            let error_doc = success_doc;
            let error_selector =
                Selector::parse(".alert-danger, .form-error, .error-message").unwrap();

            let mut error_messages = Vec::new();
            for error in error_doc.select(&error_selector) {
                let error_text = error.text().collect::<String>().trim().to_string();
                if !error_text.is_empty() {
                    error_messages.push(error_text);
                }
            }

            let error_msg = if error_messages.is_empty() {
                "Login failed - please check your credentials".to_string()
            } else {
                format!("Login failed: {}", error_messages.join("; "))
            };

            Err(LastFmError::Auth(error_msg))
        }
    }

    pub fn username(&self) -> &str {
        &self.username
    }

    pub fn is_logged_in(&self) -> bool {
        !self.username.is_empty() && self.csrf_token.is_some()
    }

    pub fn artist_tracks<'a>(&'a mut self, artist: &str) -> ArtistTracksIterator<'a> {
        ArtistTracksIterator::new(self, artist.to_string())
    }

    pub async fn get_artist_tracks_page(&mut self, artist: &str, page: u32) -> Result<TrackPage> {
        // Use AJAX endpoint for page content
        let url = format!(
            "{}/user/{}/library/music/{}/+tracks?page={}&ajax=true",
            self.base_url,
            self.username,
            artist.replace(" ", "+"),
            page
        );

        self.debug_log(&format!(
            "Fetching tracks page {} for artist: {}",
            page, artist
        ));
        let mut response = self.get(&url).await?;
        let content = response
            .body_string()
            .await
            .map_err(|e| LastFmError::Http(e.to_string()))?;

        self.debug_log(&format!(
            "AJAX response: {} status, {} chars",
            response.status(),
            content.len()
        ));

        // Check if we got JSON or HTML
        if content.trim_start().starts_with("{") || content.trim_start().starts_with("[") {
            self.debug_log("Parsing JSON response from AJAX endpoint");
            return self.parse_json_tracks_page(&content, page, artist);
        } else {
            self.debug_log("Parsing HTML response from AJAX endpoint");
            let document = Html::parse_document(&content);
            return self.parse_tracks_page(&document, page, artist);
        }
    }

    fn parse_json_tracks_page(
        &self,
        _json_content: &str,
        page_number: u32,
        _artist: &str,
    ) -> Result<TrackPage> {
        // JSON parsing not yet implemented - fallback to empty page
        self.debug_log("JSON parsing not implemented, returning empty page");
        Ok(TrackPage {
            tracks: Vec::new(),
            page_number,
            has_next_page: false,
            total_pages: Some(1),
        })
    }

    fn parse_tracks_page(
        &self,
        document: &Html,
        page_number: u32,
        artist: &str,
    ) -> Result<TrackPage> {
        let mut tracks = Vec::new();

        // Try parsing track data from data attributes (AJAX response)
        let track_selector = Selector::parse("[data-track-name]").unwrap();
        let track_elements: Vec<_> = document.select(&track_selector).collect();

        if !track_elements.is_empty() {
            self.debug_log(&format!(
                "Found {} track elements with data-track-name",
                track_elements.len()
            ));

            // Use a set to track unique tracks (since each track might appear multiple times)
            let mut seen_tracks = std::collections::HashSet::new();

            for element in track_elements {
                if let Some(track_name) = element.value().attr("data-track-name") {
                    // Skip if we've already processed this track
                    if seen_tracks.contains(track_name) {
                        continue;
                    }
                    seen_tracks.insert(track_name.to_string());

                    // Find the play count for this track
                    let playcount = self.find_playcount_for_track(document, track_name)?;

                    let track = Track {
                        name: track_name.to_string(),
                        artist: artist.to_string(),
                        playcount,
                    };
                    tracks.push(track);

                    if tracks.len() >= 50 {
                        break; // Last.fm shows 50 tracks per page
                    }
                }
            }

            self.debug_log(&format!(
                "Successfully parsed {} unique tracks",
                tracks.len()
            ));
        } else {
            // Fallback to old table parsing method
            self.debug_log("No data-track-name elements found, trying table parsing");

            let table_selector = Selector::parse("table.chartlist").unwrap();
            let row_selector = Selector::parse("tbody tr").unwrap();

            if let Some(table) = document.select(&table_selector).next() {
                for row in table.select(&row_selector) {
                    if let Ok(mut track) = self.parse_track_row(&row) {
                        track.artist = artist.to_string();
                        tracks.push(track);
                    }
                }
            } else {
                self.debug_log("No table.chartlist found either");
            }
        }

        // Check for pagination
        let (has_next_page, total_pages) = self.parse_pagination(&document, page_number)?;

        Ok(TrackPage {
            tracks,
            page_number,
            has_next_page,
            total_pages,
        })
    }

    fn find_playcount_for_track(&self, document: &Html, track_name: &str) -> Result<u32> {
        // Look for chartlist-count-bar-value elements near the track
        let count_selector = Selector::parse(".chartlist-count-bar-value").unwrap();
        let link_selector = Selector::parse("a[href*=\"/music/\"]").unwrap();

        // Find all track links that match our track name
        for link in document.select(&link_selector) {
            let link_text = link.text().collect::<String>().trim().to_string();
            if link_text == track_name {
                // Found the track link, now look for the play count in the same row
                if let Some(row) = self.find_ancestor_row(link) {
                    // Look for play count in this row
                    for count_elem in row.select(&count_selector) {
                        let count_text = count_elem.text().collect::<String>();
                        if let Some(number_part) = count_text.split_whitespace().next() {
                            if let Ok(count) = number_part.parse::<u32>() {
                                return Ok(count);
                            }
                        }
                    }
                }
            }
        }

        // Default fallback
        Ok(1)
    }

    fn find_ancestor_row<'a>(
        &self,
        element: scraper::ElementRef<'a>,
    ) -> Option<scraper::ElementRef<'a>> {
        let mut current = element;

        // Traverse up the DOM to find the table row
        while let Some(parent) = current.parent() {
            if let Some(parent_elem) = scraper::ElementRef::wrap(parent) {
                if parent_elem.value().name() == "tr" {
                    return Some(parent_elem);
                }
                current = parent_elem;
            } else {
                break;
            }
        }
        None
    }

    fn parse_track_row(&self, row: &scraper::ElementRef) -> Result<Track> {
        let name_selector = Selector::parse(".chartlist-name a").unwrap();

        let name = row
            .select(&name_selector)
            .next()
            .map(|el| el.text().collect::<String>().trim().to_string())
            .ok_or_else(|| LastFmError::Parse("Missing track name".to_string()))?;

        // Parse play count from .chartlist-count-bar-value
        // Format is like "59 scrobbles" or just "59"
        let playcount_selector = Selector::parse(".chartlist-count-bar-value").unwrap();
        let mut playcount = 1; // default fallback

        if let Some(element) = row.select(&playcount_selector).next() {
            let text = element.text().collect::<String>().trim().to_string();
            // Extract just the number part (before "scrobbles" if present)
            if let Some(number_part) = text.split_whitespace().next() {
                if let Ok(count) = number_part.parse::<u32>() {
                    playcount = count;
                }
            }
        }

        // On artist tracks pages, the artist is consistent (it's the artist we're looking at)
        // We could extract it from the page context, but for now let's keep it simple
        let artist = "".to_string(); // We'll fill this in when we call the method

        Ok(Track {
            name,
            artist,
            playcount,
        })
    }

    fn parse_pagination(&self, document: &Html, current_page: u32) -> Result<(bool, Option<u32>)> {
        let pagination_selector = Selector::parse(".pagination-list").unwrap();

        if let Some(pagination) = document.select(&pagination_selector).next() {
            // Try multiple possible selectors for next page link
            let next_selectors = [
                "a[aria-label=\"Next\"]",
                ".pagination-next a",
                "a:contains(\"Next\")",
                ".next a",
            ];

            let mut has_next = false;
            for selector_str in &next_selectors {
                if let Ok(selector) = Selector::parse(selector_str) {
                    if pagination.select(&selector).next().is_some() {
                        has_next = true;
                        break;
                    }
                }
            }

            // Alternative: check if there are more page numbers after current
            if !has_next {
                let page_link_selector = Selector::parse("a").unwrap();
                for link in pagination.select(&page_link_selector) {
                    if let Some(href) = link.value().attr("href") {
                        if href.contains(&format!("page={}", current_page + 1)) {
                            has_next = true;
                            break;
                        }
                    }
                }
            }

            // Try to find total pages from pagination numbers
            let page_link_selector = Selector::parse("a").unwrap();
            let mut max_page = current_page;

            for link in pagination.select(&page_link_selector) {
                if let Some(href) = link.value().attr("href") {
                    if let Some(page_param) = href.split("page=").nth(1) {
                        if let Some(page_num_str) = page_param.split('&').next() {
                            if let Ok(page_num) = page_num_str.parse::<u32>() {
                                max_page = max_page.max(page_num);
                            }
                        }
                    }
                }

                // Also check link text for page numbers
                let link_text = link.text().collect::<String>().trim().to_string();
                if let Ok(page_num) = link_text.parse::<u32>() {
                    max_page = max_page.max(page_num);
                }
            }

            Ok((
                has_next,
                if max_page > current_page {
                    Some(max_page)
                } else {
                    None
                },
            ))
        } else {
            // No pagination found, single page
            Ok((false, Some(1)))
        }
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

    async fn get(&mut self, url: &str) -> Result<Response> {
        self.get_with_redirects(url, 0).await
    }

    async fn get_with_redirects(&mut self, url: &str, redirect_count: u32) -> Result<Response> {
        if redirect_count > 5 {
            return Err(LastFmError::Http("Too many redirects".to_string()));
        }

        let mut request = Request::new(Method::Get, url.parse::<Url>().unwrap());
        request.insert_header("User-Agent", "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/138.0.0.0 Safari/537.36");

        // Add session cookies for all authenticated requests
        if !self.session_cookies.is_empty() {
            let cookie_header = self.session_cookies.join("; ");
            request.insert_header("Cookie", &cookie_header);
        } else if url.contains("page=") {
            self.debug_log("No cookies available for paginated request!");
        }

        // Add browser-like headers for all requests
        if url.contains("ajax=true") {
            // AJAX request headers
            request.insert_header("Accept", "*/*");
            request.insert_header("X-Requested-With", "XMLHttpRequest");
        } else {
            // Regular page request headers
            request.insert_header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7");
        }
        request.insert_header("Accept-Language", "en-US,en;q=0.9");
        request.insert_header("Accept-Encoding", "gzip, deflate, br");
        request.insert_header("DNT", "1");
        request.insert_header("Connection", "keep-alive");
        request.insert_header("Upgrade-Insecure-Requests", "1");

        // Add referer for paginated requests
        if url.contains("page=") {
            let base_url = url.split('?').next().unwrap_or(url);
            request.insert_header("Referer", base_url);
        }

        let response = self
            .client
            .send(request)
            .await
            .map_err(|e| LastFmError::Http(e.to_string()))?;

        // Extract any new cookies from the response
        self.extract_cookies(&response);

        // Handle redirects manually
        if response.status() == 302 || response.status() == 301 {
            if let Some(location) = response.header("location") {
                if let Some(redirect_url) = location.get(0) {
                    let redirect_url_str = redirect_url.as_str();
                    if url.contains("page=") {
                        self.debug_log(&format!(
                            "Following redirect from {} to {}",
                            url, redirect_url_str
                        ));

                        // Check if this is a redirect to login - authentication issue
                        if redirect_url_str.contains("/login") {
                            self.debug_log("Redirect to login page - authentication failed for paginated request");
                            return Err(LastFmError::Auth(
                                "Session expired or invalid for paginated request".to_string(),
                            ));
                        }
                    }

                    // Handle relative URLs
                    let full_redirect_url = if redirect_url_str.starts_with('/') {
                        format!("{}{}", self.base_url, redirect_url_str)
                    } else if redirect_url_str.starts_with("http") {
                        redirect_url_str.to_string()
                    } else {
                        // Relative to current path
                        let base_url = url
                            .rsplit('/')
                            .skip(1)
                            .collect::<Vec<_>>()
                            .into_iter()
                            .rev()
                            .collect::<Vec<_>>()
                            .join("/");
                        format!("{}/{}", base_url, redirect_url_str)
                    };

                    // Make a new request to the redirect URL
                    return Box::pin(
                        self.get_with_redirects(&full_redirect_url, redirect_count + 1),
                    )
                    .await;
                }
            }
        }

        if response.status() == 429 {
            return Err(LastFmError::RateLimit { retry_after: 60 });
        }

        Ok(response)
    }

    fn extract_cookies(&mut self, response: &Response) {
        // Extract Set-Cookie headers and store them (avoiding duplicates)
        if let Some(cookie_headers) = response.header("set-cookie") {
            let mut new_cookies = 0;
            for cookie_header in cookie_headers {
                let cookie_str = cookie_header.as_str();
                // Extract just the cookie name=value part (before any semicolon)
                if let Some(cookie_value) = cookie_str.split(';').next() {
                    let cookie_name = cookie_value.split('=').next().unwrap_or("");

                    // Remove any existing cookie with the same name
                    self.session_cookies
                        .retain(|existing| !existing.starts_with(&format!("{}=", cookie_name)));

                    self.session_cookies.push(cookie_value.to_string());
                    new_cookies += 1;
                }
            }
            if new_cookies > 0 {
                self.debug_log(&format!(
                    "Extracted {} new cookies, total: {}",
                    new_cookies,
                    self.session_cookies.len()
                ));
                self.debug_log(&format!("Updated cookies: {:?}", &self.session_cookies));

                // Check if sessionid changed
                for cookie in &self.session_cookies {
                    if cookie.starts_with("sessionid=") {
                        self.debug_log(&format!(
                            "Current sessionid: {}",
                            &cookie[10..50.min(cookie.len())]
                        ));
                        break;
                    }
                }
            }
        }
    }
}
