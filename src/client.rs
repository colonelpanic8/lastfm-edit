use crate::{
    ArtistTracksIterator, EditResponse, LastFmError, Result, ScrobbleEdit, Track, TrackPage,
};
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

    /// Fetch recent scrobbles from the user's listening history
    /// This gives us real scrobble data with timestamps for editing
    pub async fn get_recent_scrobbles(&mut self, page: u32) -> Result<Vec<Track>> {
        let url = format!(
            "{}/user/{}/library?page={}",
            self.base_url, self.username, page
        );

        self.debug_log(&format!("Fetching recent scrobbles page {}", page));
        let mut response = self.get(&url).await?;
        let content = response
            .body_string()
            .await
            .map_err(|e| LastFmError::Http(e.to_string()))?;

        self.debug_log(&format!(
            "Recent scrobbles response: {} status, {} chars",
            response.status(),
            content.len()
        ));

        let document = Html::parse_document(&content);
        self.parse_recent_scrobbles(&document)
    }

    /// Find the most recent scrobble for a specific track
    /// This searches through recent listening history to find real scrobble data
    pub async fn find_recent_scrobble_for_track(
        &mut self,
        track_name: &str,
        artist_name: &str,
        max_pages: u32,
    ) -> Result<Option<Track>> {
        self.debug_log(&format!(
            "Searching for recent scrobble: '{}' by '{}'",
            track_name, artist_name
        ));

        for page in 1..=max_pages {
            let scrobbles = self.get_recent_scrobbles(page).await?;

            for scrobble in scrobbles {
                if scrobble.name == track_name && scrobble.artist == artist_name {
                    self.debug_log(&format!(
                        "Found recent scrobble: '{}' with timestamp {:?}",
                        scrobble.name, scrobble.timestamp
                    ));
                    return Ok(Some(scrobble));
                }
            }

            // Small delay between pages to be polite
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        }

        self.debug_log(&format!(
            "No recent scrobble found for '{}' by '{}' in {} pages",
            track_name, artist_name, max_pages
        ));
        Ok(None)
    }

    pub async fn edit_scrobble(&mut self, edit: &ScrobbleEdit) -> Result<EditResponse> {
        if !self.is_logged_in() {
            return Err(LastFmError::Auth(
                "Must be logged in to edit scrobbles".to_string(),
            ));
        }

        let edit_url = format!(
            "{}/user/{}/library/edit?edited-variation=library-track-scrobble",
            self.base_url, self.username
        );

        self.debug_log("=== STEP 1: Getting fresh CSRF token for edit ===");

        // First request: Get the edit form to extract fresh CSRF token
        let mut form_response = self.get(&edit_url).await?;
        let form_html = form_response
            .body_string()
            .await
            .map_err(|e| LastFmError::Http(e.to_string()))?;

        self.debug_log(&format!(
            "Edit form response status: {}",
            form_response.status()
        ));

        // Parse HTML to get fresh CSRF token
        let form_document = Html::parse_document(&form_html);
        let fresh_csrf_token = self.extract_csrf_token(&form_document)?;

        self.debug_log(&format!("Fresh CSRF token: {}", fresh_csrf_token));
        self.debug_log("=== STEP 2: Submitting edit with fresh token ===");

        let mut form_data = HashMap::new();

        // Add fresh CSRF token (required)
        form_data.insert("csrfmiddlewaretoken", fresh_csrf_token.as_str());

        // Include ALL form fields as they were extracted from the track page
        form_data.insert("track_name_original", &edit.track_name_original);
        form_data.insert("track_name", &edit.track_name);
        form_data.insert("artist_name_original", &edit.artist_name_original);
        form_data.insert("artist_name", &edit.artist_name);
        form_data.insert("album_name_original", &edit.album_name_original);
        form_data.insert("album_name", &edit.album_name);
        form_data.insert("album_artist_name_original", &edit.album_artist_name_original);
        form_data.insert("album_artist_name", &edit.album_artist_name);

        // ALWAYS include timestamp - Last.fm requires it even with edit_all=true
        let timestamp_str = edit.timestamp.to_string();
        form_data.insert("timestamp", &timestamp_str);

        // Edit flags
        if edit.edit_all {
            form_data.insert("edit_all", "1");
        }
        form_data.insert("submit", "edit-scrobble");
        form_data.insert("ajax", "1");

        self.debug_log(&format!(
            "Editing scrobble: '{}' -> '{}'",
            edit.track_name_original, edit.track_name
        ));
        self.debug_log(&format!("Using fresh CSRF token: {}", fresh_csrf_token));
        self.debug_log(&format!(
            "Session cookies count: {}",
            self.session_cookies.len()
        ));

        let mut request = Request::new(Method::Post, edit_url.parse::<Url>().unwrap());

        // Add comprehensive headers matching your browser request
        request.insert_header("Accept", "*/*");
        request.insert_header("Accept-Language", "en-US,en;q=0.9");
        request.insert_header(
            "Content-Type",
            "application/x-www-form-urlencoded;charset=UTF-8",
        );
        request.insert_header("Priority", "u=1, i");
        request.insert_header("User-Agent", "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/138.0.0.0 Safari/537.36");
        request.insert_header("X-Requested-With", "XMLHttpRequest");
        request.insert_header("Sec-Fetch-Dest", "empty");
        request.insert_header("Sec-Fetch-Mode", "cors");
        request.insert_header("Sec-Fetch-Site", "same-origin");
        request.insert_header(
            "sec-ch-ua",
            "\"Not)A;Brand\";v=\"8\", \"Chromium\";v=\"138\", \"Google Chrome\";v=\"138\"",
        );
        request.insert_header("sec-ch-ua-mobile", "?0");
        request.insert_header("sec-ch-ua-platform", "\"Linux\"");

        // Add session cookies
        if !self.session_cookies.is_empty() {
            let cookie_header = self.session_cookies.join("; ");
            request.insert_header("Cookie", &cookie_header);
        }

        // Add specific referer (matching browser pattern)
        request.insert_header(
            "Referer",
            &format!(
                "{}/user/{}/library/music/The+Beatles/+tracks?page=1",
                self.base_url, self.username
            ),
        );

        // Convert form data to URL-encoded string
        let form_string: String = form_data
            .iter()
            .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
            .collect::<Vec<_>>()
            .join("&");

        self.debug_log(&format!("Form data: {}", form_string));
        request.set_body(form_string);

        let mut response = self
            .client
            .send(request)
            .await
            .map_err(|e| LastFmError::Http(e.to_string()))?;

        self.debug_log(&format!("Edit response status: {}", response.status()));

        let response_text = response
            .body_string()
            .await
            .map_err(|e| LastFmError::Http(e.to_string()))?;

        self.debug_log(&format!("Edit response: {}", response_text));

        // Parse the HTML response to check for actual success/failure
        let document = Html::parse_document(&response_text);

        // Check for success indicator
        let success_selector = Selector::parse(".alert-success").unwrap();
        let error_selector = Selector::parse(".alert-danger, .alert-error, .error").unwrap();

        let has_success_alert = document.select(&success_selector).next().is_some();
        let has_error_alert = document.select(&error_selector).next().is_some();

        // Also check if we can see the edited track in the response
        // The response contains the track data in a table format within a script template
        let mut actual_track_name = None;
        let mut actual_album_name = None;

        // Try direct selectors first
        let track_name_selector = Selector::parse("td.chartlist-name a").unwrap();
        let album_name_selector = Selector::parse("td.chartlist-album a").unwrap();

        if let Some(track_element) = document.select(&track_name_selector).next() {
            actual_track_name = Some(track_element.text().collect::<String>().trim().to_string());
        }

        if let Some(album_element) = document.select(&album_name_selector).next() {
            actual_album_name = Some(album_element.text().collect::<String>().trim().to_string());
        }

        // If not found, try extracting from the raw response text
        if actual_track_name.is_none() || actual_album_name.is_none() {
            // Look for track name in href="/music/The+Beatles/_/Things+We+Said+Today"
            if let Some(track_match) = response_text.find("href=\"/music/The+Beatles/_/") {
                let track_start = response_text[track_match..]
                    .find("_/")
                    .map(|pos| track_match + pos + 2);
                if let Some(start) = track_start {
                    let track_end = response_text[start..].find("\"").map(|pos| start + pos);
                    if let Some(end) = track_end {
                        let raw_track = &response_text[start..end];
                        // URL decode the track name
                        let decoded_track = raw_track.replace("+", " ").replace("%27", "'");
                        actual_track_name = Some(decoded_track);
                    }
                }
            }

            // Look for album name in href="/music/The+Beatles/A+Hard+Day%27s+Night"
            if let Some(album_match) = response_text.find("href=\"/music/The+Beatles/A+Hard+Day") {
                let album_start = response_text[album_match..]
                    .find("/The+Beatles/")
                    .map(|pos| album_match + pos + 13);
                if let Some(start) = album_start {
                    let album_end = response_text[start..].find("\"").map(|pos| start + pos);
                    if let Some(end) = album_end {
                        let raw_album = &response_text[start..end];
                        // URL decode the album name
                        let decoded_album = raw_album.replace("+", " ").replace("%27", "'");
                        actual_album_name = Some(decoded_album);
                    }
                }
            }
        }

        self.debug_log(&format!(
            "Response analysis: success_alert={}, error_alert={}, track='{}', album='{}'",
            has_success_alert,
            has_error_alert,
            actual_track_name.as_deref().unwrap_or("not found"),
            actual_album_name.as_deref().unwrap_or("not found")
        ));

        // Determine if edit was truly successful
        let success = response.status().is_success() && has_success_alert && !has_error_alert;

        // Check if the edit actually took effect by comparing with expected values
        let edit_took_effect =
            if let (Some(track), Some(album)) = (&actual_track_name, &actual_album_name) {
                // The track name should match what we edited to
                let track_matches = track == &edit.track_name;
                // The album should also match what we edited to (if we edited it)
                let album_matches = album == &edit.album_name;

                self.debug_log(&format!(
                    "Edit validation: expected_track='{}', actual_track='{}', matches={}",
                    edit.track_name, track, track_matches
                ));
                self.debug_log(&format!(
                    "Edit validation: expected_album='{}', actual_album='{}', matches={}",
                    edit.album_name, album, album_matches
                ));

                track_matches && album_matches
            } else {
                false
            };

        let final_success = success && edit_took_effect;

        // Create detailed message
        let message = if has_error_alert {
            // Extract error message
            if let Some(error_element) = document.select(&error_selector).next() {
                Some(format!(
                    "Edit failed: {}",
                    error_element.text().collect::<String>().trim()
                ))
            } else {
                Some("Edit failed with unknown error".to_string())
            }
        } else if final_success {
            Some(format!(
                "Edit successful - Track: '{}', Album: '{}'",
                actual_track_name.as_deref().unwrap_or("unknown"),
                actual_album_name.as_deref().unwrap_or("unknown")
            ))
        } else if success && !edit_took_effect {
            Some(format!(
                "Server returned success but edit didn't take effect. Expected track='{}', album='{}' but got track='{}', album='{}'",
                edit.track_name,
                edit.album_name,
                actual_track_name.as_deref().unwrap_or("unknown"),
                actual_album_name.as_deref().unwrap_or("unknown")
            ))
        } else {
            Some(format!("Edit failed with status: {}", response.status()))
        };

        Ok(EditResponse {
            success: final_success,
            message,
        })
    }

    /// Load prepopulated form values for editing a specific track
    /// This extracts scrobble data directly from the track page forms
    pub async fn load_edit_form_values(
        &mut self,
        track_name: &str,
        artist_name: &str,
    ) -> Result<crate::ScrobbleEdit> {
        self.debug_log(&format!(
            "Loading edit form values for '{}' by '{}'",
            track_name, artist_name
        ));

        // Get the specific track page to find scrobble forms
        // Add +noredirect to avoid redirects as per lastfm-bulk-edit approach
        // Use the correct URL format with underscore: artist/_/track
        let track_url = format!(
            "{}/user/{}/library/music/+noredirect/{}/_/{}",
            self.base_url,
            self.username,
            urlencoding::encode(artist_name),
            urlencoding::encode(track_name)
        );

        self.debug_log(&format!("Fetching track page: {}", track_url));

        let mut response = self.get(&track_url).await?;
        let html = response
            .body_string()
            .await
            .map_err(|e| crate::LastFmError::Http(e.to_string()))?;

        let document = Html::parse_document(&html);

        // Extract scrobble data directly from the track page forms
        self.extract_scrobble_data_from_track_page(&document, track_name, artist_name)
    }

    /// Extract scrobble edit data directly from track page forms
    /// Based on the approach used in lastfm-bulk-edit
    fn extract_scrobble_data_from_track_page(
        &self,
        document: &Html,
        expected_track: &str,
        expected_artist: &str,
    ) -> Result<crate::ScrobbleEdit> {
        self.debug_log("Extracting scrobble data directly from track page forms...");
        
        // Look for the chartlist table that contains scrobbles
        let table_selector = Selector::parse("table.chartlist:not(.chartlist__placeholder)").unwrap();
        let table = document.select(&table_selector).next()
            .ok_or_else(|| crate::LastFmError::Parse("No chartlist table found on track page".to_string()))?;

        // Look for table rows that contain scrobble edit forms
        let row_selector = Selector::parse("tr").unwrap();
        for row in table.select(&row_selector) {
            // Check if this row has a count bar link (means it's an aggregation, not individual scrobbles)
            let count_bar_link_selector = Selector::parse(".chartlist-count-bar-link").unwrap();
            if row.select(&count_bar_link_selector).next().is_some() {
                self.debug_log("Found count bar link, skipping aggregated row");
                continue;
            }

            // Look for scrobble edit form in this row
            let form_selector = Selector::parse("form[data-edit-scrobble]").unwrap();
            if let Some(form) = row.select(&form_selector).next() {
                // Extract all form values directly
                let extract_form_value = |name: &str| -> Option<String> {
                    let selector = Selector::parse(&format!("input[name='{}']", name)).unwrap();
                    form.select(&selector).next()
                        .and_then(|input| input.value().attr("value"))
                        .map(|s| s.to_string())
                };

                // Get the track and artist from this form
                let form_track = extract_form_value("track_name").unwrap_or_default();
                let form_artist = extract_form_value("artist_name").unwrap_or_default();
                let form_album = extract_form_value("album_name").unwrap_or_default();
                let form_album_artist = extract_form_value("album_artist_name").unwrap_or_else(|| form_artist.clone());
                let form_timestamp = extract_form_value("timestamp").unwrap_or_default();

                self.debug_log(&format!(
                    "Found scrobble form - Track: '{}', Artist: '{}', Album: '{}', Timestamp: {}", 
                    form_track, form_artist, form_album, form_timestamp
                ));

                // Check if this form matches the expected track and artist
                if form_track == expected_track && form_artist == expected_artist {
                    let timestamp = form_timestamp.parse::<u64>()
                        .map_err(|_| crate::LastFmError::Parse("Invalid timestamp in form".to_string()))?;

                    self.debug_log(&format!(
                        "✅ Found matching scrobble form for '{}' by '{}'", expected_track, expected_artist
                    ));

                    // Create ScrobbleEdit with the extracted values
                    return Ok(crate::ScrobbleEdit {
                        track_name_original: form_track.clone(),
                        album_name_original: form_album.clone(),
                        artist_name_original: form_artist.clone(),
                        album_artist_name_original: form_album_artist.clone(),
                        track_name: form_track,
                        album_name: form_album,
                        artist_name: form_artist,
                        album_artist_name: form_album_artist,
                        timestamp,
                        edit_all: true,
                    });
                }
            }
        }

        Err(crate::LastFmError::Parse(format!(
            "No scrobble form found for track '{}' by '{}'", expected_track, expected_artist
        )))
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

                    // Try to find timestamp for this track
                    let timestamp = self.find_timestamp_for_track(document, track_name);

                    let track = Track {
                        name: track_name.to_string(),
                        artist: artist.to_string(),
                        playcount,
                        timestamp,
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

    fn find_timestamp_for_track(&self, document: &Html, track_name: &str) -> Option<u64> {
        // Look for timestamp in hidden form inputs for edit scrobble forms
        let form_selector = Selector::parse("form[data-edit-scrobble]").unwrap();
        let timestamp_selector = Selector::parse("input[name=\"timestamp\"]").unwrap();

        for form in document.select(&form_selector) {
            // Check if this form is for our track
            let track_input_selector = Selector::parse("input[name=\"track_name\"]").unwrap();
            if let Some(track_input) = form.select(&track_input_selector).next() {
                if let Some(value) = track_input.value().attr("value") {
                    if value == track_name {
                        // Found the form for our track, get the timestamp
                        if let Some(timestamp_input) = form.select(&timestamp_selector).next() {
                            if let Some(timestamp_str) = timestamp_input.value().attr("value") {
                                if let Ok(timestamp) = timestamp_str.parse::<u64>() {
                                    return Some(timestamp);
                                }
                            }
                        }
                    }
                }
            }
        }
        None
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
            timestamp: None, // Not available in table parsing mode
        })
    }

    /// Parse recent scrobbles from the user's library page
    /// This extracts real scrobble data with timestamps for editing
    fn parse_recent_scrobbles(&self, document: &Html) -> Result<Vec<Track>> {
        let mut tracks = Vec::new();

        // Recent scrobbles are typically in a chartlist table
        let table_selector = Selector::parse("table.chartlist").unwrap();
        let row_selector = Selector::parse("tbody tr").unwrap();

        if let Some(table) = document.select(&table_selector).next() {
            for row in table.select(&row_selector) {
                if let Ok(track) = self.parse_recent_scrobble_row(&row) {
                    tracks.push(track);
                }
            }
        } else {
            self.debug_log("No chartlist table found in recent scrobbles");
        }

        self.debug_log(&format!("Parsed {} recent scrobbles", tracks.len()));
        Ok(tracks)
    }

    /// Parse a single row from the recent scrobbles table
    fn parse_recent_scrobble_row(&self, row: &scraper::ElementRef) -> Result<Track> {
        // Extract track name
        let name_selector = Selector::parse(".chartlist-name a").unwrap();
        let name = row
            .select(&name_selector)
            .next()
            .ok_or(LastFmError::Parse("Missing track name".to_string()))?
            .text()
            .collect::<String>()
            .trim()
            .to_string();

        // Extract artist name
        let artist_selector = Selector::parse(".chartlist-artist a").unwrap();
        let artist = row
            .select(&artist_selector)
            .next()
            .ok_or(LastFmError::Parse("Missing artist name".to_string()))?
            .text()
            .collect::<String>()
            .trim()
            .to_string();

        // Extract timestamp from data attributes or hidden inputs
        let timestamp = self.extract_scrobble_timestamp(row);

        // For recent scrobbles, playcount is typically 1 since they're individual scrobbles
        let playcount = 1;

        Ok(Track {
            name,
            artist,
            playcount,
            timestamp,
        })
    }

    /// Extract timestamp from scrobble row elements
    fn extract_scrobble_timestamp(&self, row: &scraper::ElementRef) -> Option<u64> {
        // Look for timestamp in various places:

        // 1. Check for data-timestamp attribute
        if let Some(timestamp_str) = row.value().attr("data-timestamp") {
            if let Ok(timestamp) = timestamp_str.parse::<u64>() {
                return Some(timestamp);
            }
        }

        // 2. Look for hidden timestamp input
        let timestamp_input_selector = Selector::parse("input[name='timestamp']").unwrap();
        if let Some(input) = row.select(&timestamp_input_selector).next() {
            if let Some(value) = input.value().attr("value") {
                if let Ok(timestamp) = value.parse::<u64>() {
                    return Some(timestamp);
                }
            }
        }

        // 3. Look for edit form with timestamp
        let edit_form_selector =
            Selector::parse("form[data-edit-scrobble] input[name='timestamp']").unwrap();
        if let Some(timestamp_input) = row.select(&edit_form_selector).next() {
            if let Some(value) = timestamp_input.value().attr("value") {
                if let Ok(timestamp) = value.parse::<u64>() {
                    return Some(timestamp);
                }
            }
        }

        // 4. Look for time element with datetime attribute
        let time_selector = Selector::parse("time").unwrap();
        if let Some(time_elem) = row.select(&time_selector).next() {
            if let Some(datetime) = time_elem.value().attr("datetime") {
                // Parse ISO datetime to timestamp
                if let Ok(parsed_time) = chrono::DateTime::parse_from_rfc3339(datetime) {
                    return Some(parsed_time.timestamp() as u64);
                }
            }
        }

        None
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
