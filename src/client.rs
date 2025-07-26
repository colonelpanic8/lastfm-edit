use crate::edit::{ExactScrobbleEdit, SingleEditResponse};
use crate::edit_analysis;
use crate::events::{
    ClientEvent, ClientEventReceiver, RateLimitType, RequestInfo, SharedEventBroadcaster,
};
use crate::headers;
use crate::login::extract_cookies_from_response;
use crate::parsing::LastFmParser;
use crate::r#trait::LastFmEditClient;
use crate::retry::{self, RetryConfig};
use crate::session::LastFmEditSession;
use crate::{AlbumPage, EditResponse, LastFmError, Result, ScrobbleEdit, Track, TrackPage};
use async_trait::async_trait;
use http_client::{HttpClient, Request, Response};
use http_types::{Method, Url};
use scraper::{Html, Selector};
use std::sync::{Arc, Mutex};

/// Main implementation for interacting with Last.fm's web interface.
///
/// This implementation provides methods for browsing user libraries and editing scrobble data
/// through web scraping. It requires a valid authenticated session to function.
///
#[derive(Clone)]
pub struct LastFmEditClientImpl {
    client: Arc<dyn HttpClient + Send + Sync>,
    session: Arc<Mutex<LastFmEditSession>>,
    rate_limit_patterns: Vec<String>,
    parser: LastFmParser,
    broadcaster: Arc<SharedEventBroadcaster>,
}

impl LastFmEditClientImpl {
    /// Create a new [`LastFmEditClient`] from an authenticated session.
    ///
    /// This is the primary constructor for creating a client. You must obtain a valid
    /// session first using the [`login`](crate::login::LoginManager::login) function.
    ///
    /// # Arguments
    ///
    /// * `client` - Any HTTP client implementation that implements [`HttpClient`]
    /// * `session` - A valid authenticated session
    ///
    pub fn from_session(
        client: Box<dyn HttpClient + Send + Sync>,
        session: LastFmEditSession,
    ) -> Self {
        Self::from_session_with_arc(Arc::from(client), session)
    }

    /// Create a new [`LastFmEditClient`] from an authenticated session with Arc client.
    ///
    /// Internal helper method to avoid Arc/Box conversion issues.
    fn from_session_with_arc(
        client: Arc<dyn HttpClient + Send + Sync>,
        session: LastFmEditSession,
    ) -> Self {
        Self::from_session_with_broadcaster_arc(
            client,
            session,
            Arc::new(SharedEventBroadcaster::new()),
        )
    }

    /// Create a new [`LastFmEditClient`] from an authenticated session with custom rate limit patterns.
    ///
    /// This is useful for testing or customizing rate limit detection.
    ///
    /// # Arguments
    ///
    /// * `client` - Any HTTP client implementation
    /// * `session` - A valid authenticated session
    /// * `rate_limit_patterns` - Text patterns that indicate rate limiting in responses
    pub fn from_session_with_rate_limit_patterns(
        client: Box<dyn HttpClient + Send + Sync>,
        session: LastFmEditSession,
        rate_limit_patterns: Vec<String>,
    ) -> Self {
        Self {
            client: Arc::from(client),
            session: Arc::new(Mutex::new(session)),
            rate_limit_patterns,
            parser: LastFmParser::new(),
            broadcaster: Arc::new(SharedEventBroadcaster::new()),
        }
    }

    /// Create a new authenticated [`LastFmEditClient`] by logging in with username and password.
    ///
    /// This is a convenience method that combines login and client creation into one step.
    ///
    /// # Arguments
    ///
    /// * `client` - Any HTTP client implementation
    /// * `username` - Last.fm username or email
    /// * `password` - Last.fm password
    ///
    /// # Returns
    ///
    /// Returns an authenticated client on success, or [`LastFmError::Auth`] on failure.
    ///
    pub async fn login_with_credentials(
        client: Box<dyn HttpClient + Send + Sync>,
        username: &str,
        password: &str,
    ) -> Result<Self> {
        let client_arc: Arc<dyn HttpClient + Send + Sync> = Arc::from(client);
        let login_manager =
            crate::login::LoginManager::new(client_arc.clone(), "https://www.last.fm".to_string());
        let session = login_manager.login(username, password).await?;
        Ok(Self::from_session_with_arc(client_arc, session))
    }

    /// Create a new [`LastFmEditClient`] from a session with a shared broadcaster.
    ///
    /// This allows you to create multiple clients that share the same event broadcasting system.
    /// When any client encounters rate limiting, all clients sharing the broadcaster will see the event.
    ///
    /// # Arguments
    ///
    /// * `client` - Any HTTP client implementation
    /// * `session` - A valid authenticated session
    /// * `broadcaster` - Shared broadcaster from another client
    ///
    /// # Returns
    ///
    /// Returns a client with the session and shared broadcaster.
    fn from_session_with_broadcaster(
        client: Box<dyn HttpClient + Send + Sync>,
        session: LastFmEditSession,
        broadcaster: Arc<SharedEventBroadcaster>,
    ) -> Self {
        Self::from_session_with_broadcaster_arc(Arc::from(client), session, broadcaster)
    }

    /// Internal helper for creating client with Arc and broadcaster
    fn from_session_with_broadcaster_arc(
        client: Arc<dyn HttpClient + Send + Sync>,
        session: LastFmEditSession,
        broadcaster: Arc<SharedEventBroadcaster>,
    ) -> Self {
        Self {
            client,
            session: Arc::new(Mutex::new(session)),
            rate_limit_patterns: vec![
                "you've tried to log in too many times".to_string(),
                "you're requesting too many pages".to_string(),
                "slow down".to_string(),
                "too fast".to_string(),
                "rate limit".to_string(),
                "throttled".to_string(),
                "temporarily blocked".to_string(),
                "temporarily restricted".to_string(),
                "captcha".to_string(),
                "verify you're human".to_string(),
                "prove you're not a robot".to_string(),
                "security check".to_string(),
                "service temporarily unavailable".to_string(),
                "quota exceeded".to_string(),
                "limit exceeded".to_string(),
                "daily limit".to_string(),
            ],
            parser: LastFmParser::new(),
            broadcaster,
        }
    }

    /// Extract the current session state for persistence.
    pub fn get_session(&self) -> LastFmEditSession {
        self.session.lock().unwrap().clone()
    }

    /// Restore session state from a previously saved session.
    pub fn restore_session(&self, session: LastFmEditSession) {
        *self.session.lock().unwrap() = session;
    }

    /// Create a new client that shares the same session and event broadcaster.
    ///
    /// This is useful when you want multiple HTTP client instances but want them to
    /// share the same authentication state and event broadcasting system.
    ///
    /// # Arguments
    ///
    /// * `client` - A new HTTP client implementation
    ///
    /// # Returns
    ///
    /// Returns a new client that shares the session and broadcaster with this client.
    ///
    pub fn with_shared_broadcaster(&self, client: Box<dyn HttpClient + Send + Sync>) -> Self {
        let session = self.get_session();
        Self::from_session_with_broadcaster(client, session, self.broadcaster.clone())
    }

    /// Get the currently authenticated username.
    ///
    /// Returns an empty string if not logged in.
    pub fn username(&self) -> String {
        self.session.lock().unwrap().username.clone()
    }

    pub async fn validate_session(&self) -> bool {
        let test_url = {
            let session = self.session.lock().unwrap();
            format!(
                "{}/settings/subscription/automatic-edits/tracks",
                session.base_url
            )
        };

        let mut request = Request::new(Method::Get, test_url.parse::<Url>().unwrap());

        {
            let session = self.session.lock().unwrap();
            headers::add_cookies(&mut request, &session.cookies);
        }

        headers::add_get_headers(&mut request, false, None);

        match self.client.send(request).await {
            Ok(response) => {
                // Check if we got redirected to login
                if response.status() == 302 || response.status() == 301 {
                    if let Some(location) = response.header("location") {
                        if let Some(redirect_url) = location.get(0) {
                            let redirect_url_str = redirect_url.as_str();
                            let is_valid = !redirect_url_str.contains("/login");

                            return is_valid;
                        }
                    }
                }
                true
            }
            Err(_e) => false,
        }
    }

    /// Delete a scrobble by its identifying information.
    pub async fn delete_scrobble(
        &self,
        artist_name: &str,
        track_name: &str,
        timestamp: u64,
    ) -> Result<bool> {
        let config = RetryConfig {
            max_retries: 3,
            base_delay: 5,
            max_delay: 300,
        };

        let artist_name = artist_name.to_string();
        let track_name = track_name.to_string();
        let client = self.clone();

        match retry::retry_with_backoff(
            config,
            "Delete scrobble",
            || client.delete_scrobble_impl(&artist_name, &track_name, timestamp),
            |delay, operation_name| {
                self.broadcast_event(ClientEvent::RateLimited {
                    delay_seconds: delay,
                    request: None,
                    rate_limit_type: RateLimitType::ResponsePattern,
                });
                log::debug!("{operation_name} rate limited, waiting {delay} seconds");
            },
        )
        .await
        {
            Ok(retry_result) => Ok(retry_result.result),
            Err(_) => Ok(false),
        }
    }

    async fn delete_scrobble_impl(
        &self,
        artist_name: &str,
        track_name: &str,
        timestamp: u64,
    ) -> Result<bool> {
        let delete_url = {
            let session = self.session.lock().unwrap();
            format!(
                "{}/user/{}/library/delete",
                session.base_url, session.username
            )
        };

        log::debug!("Getting fresh CSRF token for delete");

        // Get fresh CSRF token from any page that has it (we'll use the library page)
        let library_url = {
            let session = self.session.lock().unwrap();
            format!("{}/user/{}/library", session.base_url, session.username)
        };

        let mut response = self.get(&library_url).await?;
        let content = response
            .body_string()
            .await
            .map_err(|e| LastFmError::Http(e.to_string()))?;

        let document = Html::parse_document(&content);
        let fresh_csrf_token = self.extract_csrf_token(&document)?;

        log::debug!("Submitting delete request with fresh token");

        let mut request = Request::new(Method::Post, delete_url.parse::<Url>().unwrap());

        // Add session cookies and set up headers
        let referer_url = {
            let session = self.session.lock().unwrap();
            headers::add_cookies(&mut request, &session.cookies);
            format!("{}/user/{}", session.base_url, session.username)
        };

        // Add standard headers for AJAX delete requests
        headers::add_edit_headers(&mut request, &referer_url);

        // Build form data
        let form_data = [
            ("csrfmiddlewaretoken", fresh_csrf_token.as_str()),
            ("artist_name", artist_name),
            ("track_name", track_name),
            ("timestamp", &timestamp.to_string()),
            ("ajax", "1"),
        ];

        // Convert form data to URL-encoded string
        let form_string: String = form_data
            .iter()
            .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
            .collect::<Vec<_>>()
            .join("&");

        request.set_body(form_string);

        log::debug!(
            "Deleting scrobble: '{track_name}' by '{artist_name}' with timestamp {timestamp}"
        );

        // Create request info for event broadcasting
        let request_info = RequestInfo::from_url_and_method(&delete_url, "POST");
        let request_start = std::time::Instant::now();

        // Broadcast request started event
        self.broadcast_event(ClientEvent::RequestStarted {
            request: request_info.clone(),
        });

        let mut response = self
            .client
            .send(request)
            .await
            .map_err(|e| LastFmError::Http(e.to_string()))?;

        // Broadcast request completed event
        self.broadcast_event(ClientEvent::RequestCompleted {
            request: request_info.clone(),
            status_code: response.status().into(),
            duration_ms: request_start.elapsed().as_millis() as u64,
        });

        log::debug!("Delete response status: {}", response.status());

        let response_text = response
            .body_string()
            .await
            .map_err(|e| LastFmError::Http(e.to_string()))?;

        // Check if the delete was successful
        // A successful delete typically returns a 200 status with empty or minimal content
        let success = response.status().is_success();

        if success {
            log::debug!("Successfully deleted scrobble");
        } else {
            log::debug!("Delete failed with response: {response_text}");
        }

        Ok(success)
    }

    /// Subscribe to internal client events.
    pub fn subscribe(&self) -> ClientEventReceiver {
        self.broadcaster.subscribe()
    }

    /// Get the latest client event without subscribing to future events.
    pub fn latest_event(&self) -> Option<ClientEvent> {
        self.broadcaster.latest_event()
    }

    /// Broadcast an internal event to all subscribers.
    ///
    /// This is used internally by the client to notify observers of important events.
    fn broadcast_event(&self, event: ClientEvent) {
        self.broadcaster.broadcast_event(event);
    }

    /// Fetch recent scrobbles from the user's listening history
    /// This gives us real scrobble data with timestamps for editing
    pub async fn get_recent_scrobbles(&self, page: u32) -> Result<Vec<Track>> {
        let url = {
            let session = self.session.lock().unwrap();
            format!(
                "{}/user/{}/library?page={}",
                session.base_url, session.username, page
            )
        };

        log::debug!("Fetching recent scrobbles page {page}");
        let mut response = self.get(&url).await?;
        let content = response
            .body_string()
            .await
            .map_err(|e| LastFmError::Http(e.to_string()))?;

        log::debug!(
            "Recent scrobbles response: {} status, {} chars",
            response.status(),
            content.len()
        );

        let document = Html::parse_document(&content);
        self.parser.parse_recent_scrobbles(&document)
    }

    /// Get a page of tracks from the user's recent listening history.
    pub async fn get_recent_tracks_page(&self, page: u32) -> Result<TrackPage> {
        let tracks = self.get_recent_scrobbles(page).await?;

        // For now, we'll create a basic TrackPage from the tracks
        // In a real implementation, we might need to parse pagination info from the HTML
        let has_next_page = !tracks.is_empty(); // Simple heuristic

        Ok(TrackPage {
            tracks,
            page_number: page,
            has_next_page,
            total_pages: None, // Recent tracks don't have a definite total
        })
    }

    /// Find the most recent scrobble for a specific track
    /// This searches through recent listening history to find real scrobble data
    pub async fn find_recent_scrobble_for_track(
        &self,
        track_name: &str,
        artist_name: &str,
        max_pages: u32,
    ) -> Result<Option<Track>> {
        log::debug!("Searching for recent scrobble: '{track_name}' by '{artist_name}'");

        for page in 1..=max_pages {
            let scrobbles = self.get_recent_scrobbles(page).await?;

            for scrobble in scrobbles {
                if scrobble.name == track_name && scrobble.artist == artist_name {
                    log::debug!(
                        "Found recent scrobble: '{}' with timestamp {:?}",
                        scrobble.name,
                        scrobble.timestamp
                    );
                    return Ok(Some(scrobble));
                }
            }
        }

        log::debug!(
            "No recent scrobble found for '{track_name}' by '{artist_name}' in {max_pages} pages"
        );
        Ok(None)
    }

    pub async fn edit_scrobble(&self, edit: &ScrobbleEdit) -> Result<EditResponse> {
        // Use the generalized discovery function to find all relevant scrobble instances
        let discovered_edits = self.discover_scrobble_edit_variations(edit).await?;

        if discovered_edits.is_empty() {
            let context = match (&edit.track_name_original, &edit.album_name_original) {
                (Some(track_name), _) => {
                    format!("track '{}' by '{}'", track_name, edit.artist_name_original)
                }
                (None, Some(album_name)) => {
                    format!("album '{}' by '{}'", album_name, edit.artist_name_original)
                }
                (None, None) => format!("artist '{}'", edit.artist_name_original),
            };
            return Err(LastFmError::Parse(format!(
                "No scrobbles found for {context}. Make sure the names are correct and that you have scrobbled recently."
            )));
        }

        log::info!(
            "Discovered {} scrobble instances to edit",
            discovered_edits.len()
        );

        let mut all_results = Vec::new();

        // For each discovered scrobble instance, apply the user's desired changes and edit it
        for (index, discovered_edit) in discovered_edits.iter().enumerate() {
            log::debug!(
                "Processing scrobble {}/{}: '{}' from '{}'",
                index + 1,
                discovered_edits.len(),
                discovered_edit.track_name_original,
                discovered_edit.album_name_original
            );

            // Apply the user's desired changes to the discovered exact edit
            let mut modified_exact_edit = discovered_edit.clone();

            // Apply user's changes or keep original values
            if let Some(new_track_name) = &edit.track_name {
                modified_exact_edit.track_name = new_track_name.clone();
            }
            if let Some(new_album_name) = &edit.album_name {
                modified_exact_edit.album_name = new_album_name.clone();
            }
            modified_exact_edit.artist_name = edit.artist_name.clone();
            if let Some(new_album_artist_name) = &edit.album_artist_name {
                modified_exact_edit.album_artist_name = new_album_artist_name.clone();
            }
            modified_exact_edit.edit_all = edit.edit_all;

            let album_info = format!(
                "{} by {}",
                modified_exact_edit.album_name_original,
                modified_exact_edit.album_artist_name_original
            );

            let single_response = self.edit_scrobble_single(&modified_exact_edit, 3).await?;
            let success = single_response.success();
            let message = single_response.message();

            all_results.push(SingleEditResponse {
                success,
                message,
                album_info: Some(album_info),
                exact_scrobble_edit: modified_exact_edit.clone(),
            });

            // Add delay between edits to be respectful to the server
            if index < discovered_edits.len() - 1 {
                tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
            }
        }

        Ok(EditResponse::from_results(all_results))
    }

    /// Edit a single scrobble with retry logic, returning a single-result EditResponse.
    ///
    /// This method takes a fully-specified `ExactScrobbleEdit` and performs a single
    /// edit operation. Unlike `edit_scrobble`, this method does not perform enrichment
    /// or multiple edits - it edits exactly one scrobble instance.
    ///
    /// # Arguments
    /// * `exact_edit` - A fully-specified edit with all required fields populated
    /// * `max_retries` - Maximum number of retry attempts for rate limiting
    ///
    pub async fn edit_scrobble_single(
        &self,
        exact_edit: &ExactScrobbleEdit,
        max_retries: u32,
    ) -> Result<EditResponse> {
        let config = RetryConfig {
            max_retries,
            base_delay: 5,
            max_delay: 300,
        };

        let edit_clone = exact_edit.clone();
        let client = self.clone();

        match retry::retry_with_backoff(
            config,
            "Edit scrobble",
            || client.edit_scrobble_impl(&edit_clone),
            |delay, operation_name| {
                self.broadcast_event(ClientEvent::RateLimited {
                    delay_seconds: delay,
                    request: None, // No specific request context in retry callback
                    rate_limit_type: RateLimitType::ResponsePattern,
                });
                log::debug!("{operation_name} rate limited, waiting {delay} seconds");
            },
        )
        .await
        {
            Ok(retry_result) => Ok(EditResponse::single(
                retry_result.result,
                None,
                None,
                exact_edit.clone(),
            )),
            Err(LastFmError::RateLimit { .. }) => Ok(EditResponse::single(
                false,
                Some(format!("Rate limit exceeded after {max_retries} retries")),
                None,
                exact_edit.clone(),
            )),
            Err(other_error) => Ok(EditResponse::single(
                false,
                Some(other_error.to_string()),
                None,
                exact_edit.clone(),
            )),
        }
    }

    async fn edit_scrobble_impl(&self, exact_edit: &ExactScrobbleEdit) -> Result<bool> {
        let start_time = std::time::Instant::now();
        let result = self.edit_scrobble_impl_internal(exact_edit).await;
        let duration_ms = start_time.elapsed().as_millis() as u64;

        // Broadcast edit attempt event
        match &result {
            Ok(success) => {
                self.broadcast_event(ClientEvent::EditAttempted {
                    edit: exact_edit.clone(),
                    success: *success,
                    error_message: None,
                    duration_ms,
                });
            }
            Err(error) => {
                self.broadcast_event(ClientEvent::EditAttempted {
                    edit: exact_edit.clone(),
                    success: false,
                    error_message: Some(error.to_string()),
                    duration_ms,
                });
            }
        }

        result
    }

    async fn edit_scrobble_impl_internal(&self, exact_edit: &ExactScrobbleEdit) -> Result<bool> {
        let edit_url = {
            let session = self.session.lock().unwrap();
            format!(
                "{}/user/{}/library/edit?edited-variation=library-track-scrobble",
                session.base_url, session.username
            )
        };

        log::debug!("Getting fresh CSRF token for edit");

        // First request: Get the edit form to extract fresh CSRF token
        let form_html = self.get_edit_form_html(&edit_url).await?;

        // Parse HTML to get fresh CSRF token - do parsing synchronously
        let form_document = Html::parse_document(&form_html);
        let fresh_csrf_token = self.extract_csrf_token(&form_document)?;

        log::debug!("Submitting edit with fresh token");

        let form_data = exact_edit.build_form_data(&fresh_csrf_token);

        log::debug!(
            "Editing scrobble: '{}' -> '{}'",
            exact_edit.track_name_original,
            exact_edit.track_name
        );
        {
            let session = self.session.lock().unwrap();
            log::trace!("Session cookies count: {}", session.cookies.len());
        }

        let mut request = Request::new(Method::Post, edit_url.parse::<Url>().unwrap());

        // Add session cookies and set up headers
        let referer_url = {
            let session = self.session.lock().unwrap();
            headers::add_cookies(&mut request, &session.cookies);
            format!("{}/user/{}/library", session.base_url, session.username)
        };

        headers::add_edit_headers(&mut request, &referer_url);

        // Convert form data to URL-encoded string
        let form_string: String = form_data
            .iter()
            .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
            .collect::<Vec<_>>()
            .join("&");

        request.set_body(form_string);

        // Create request info for event broadcasting
        let request_info = RequestInfo::from_url_and_method(&edit_url, "POST");
        let request_start = std::time::Instant::now();

        // Broadcast request started event
        self.broadcast_event(ClientEvent::RequestStarted {
            request: request_info.clone(),
        });

        let mut response = self
            .client
            .send(request)
            .await
            .map_err(|e| LastFmError::Http(e.to_string()))?;

        // Broadcast request completed event
        self.broadcast_event(ClientEvent::RequestCompleted {
            request: request_info.clone(),
            status_code: response.status().into(),
            duration_ms: request_start.elapsed().as_millis() as u64,
        });

        log::debug!("Edit response status: {}", response.status());

        let response_text = response
            .body_string()
            .await
            .map_err(|e| LastFmError::Http(e.to_string()))?;

        // Analyze the edit response to determine success/failure
        let analysis = edit_analysis::analyze_edit_response(&response_text, response.status());

        Ok(analysis.success)
    }

    /// Fetch raw HTML content for edit form page
    /// This separates HTTP fetching from parsing to avoid Send/Sync issues
    async fn get_edit_form_html(&self, edit_url: &str) -> Result<String> {
        let mut form_response = self.get(edit_url).await?;
        let form_html = form_response
            .body_string()
            .await
            .map_err(|e| LastFmError::Http(e.to_string()))?;

        log::debug!("Edit form response status: {}", form_response.status());
        Ok(form_html)
    }

    /// Load prepopulated form values for editing a specific track
    /// This extracts scrobble data directly from the track page forms
    pub async fn load_edit_form_values_internal(
        &self,
        track_name: &str,
        artist_name: &str,
    ) -> Result<Vec<ExactScrobbleEdit>> {
        log::debug!("Loading edit form values for '{track_name}' by '{artist_name}'");

        // Get the specific track page to find scrobble forms
        // Add +noredirect to avoid redirects as per lastfm-bulk-edit approach
        // Use the correct URL format with underscore: artist/_/track
        let base_track_url = {
            let session = self.session.lock().unwrap();
            format!(
                "{}/user/{}/library/music/+noredirect/{}/_/{}",
                session.base_url,
                session.username,
                urlencoding::encode(artist_name),
                urlencoding::encode(track_name)
            )
        };

        log::debug!("Fetching track page: {base_track_url}");

        let mut response = self.get(&base_track_url).await?;
        let html = response
            .body_string()
            .await
            .map_err(|e| crate::LastFmError::Http(e.to_string()))?;

        let document = Html::parse_document(&html);

        // Handle pagination with a loop
        let mut all_scrobble_edits = Vec::new();
        let mut unique_albums = std::collections::HashSet::new();
        let max_pages = 5;

        // Start with the current page (page 1)
        let page_edits = self.extract_scrobble_edits_from_page(
            &document,
            track_name,
            artist_name,
            &mut unique_albums,
        )?;
        all_scrobble_edits.extend(page_edits);

        log::debug!(
            "Page 1: found {} unique album variations",
            all_scrobble_edits.len()
        );

        // Check for additional pages
        let pagination_selector = Selector::parse(".pagination .pagination-next").unwrap();
        let mut has_next_page = document.select(&pagination_selector).next().is_some();
        let mut page = 2;

        while has_next_page && page <= max_pages {
            // For pagination, we need to remove +noredirect and add page parameter
            let page_url = {
                let session = self.session.lock().unwrap();
                format!(
                    "{}/user/{}/library/music/{}/_/{}?page={page}",
                    session.base_url,
                    session.username,
                    urlencoding::encode(artist_name),
                    urlencoding::encode(track_name)
                )
            };

            log::debug!("Fetching page {page} for additional album variations");

            let mut response = self.get(&page_url).await?;
            let html = response
                .body_string()
                .await
                .map_err(|e| crate::LastFmError::Http(e.to_string()))?;

            let document = Html::parse_document(&html);

            let page_edits = self.extract_scrobble_edits_from_page(
                &document,
                track_name,
                artist_name,
                &mut unique_albums,
            )?;

            let initial_count = all_scrobble_edits.len();
            all_scrobble_edits.extend(page_edits);
            let found_new_unique_albums = all_scrobble_edits.len() > initial_count;

            // Check if there's another next page
            has_next_page = document.select(&pagination_selector).next().is_some();

            log::debug!(
                "Page {page}: found {} total unique albums ({})",
                all_scrobble_edits.len(),
                if found_new_unique_albums {
                    "new albums found"
                } else {
                    "no new unique albums"
                }
            );

            // Continue to next page even if no new unique albums found on this page,
            // as long as there are more pages available
            page += 1;
        }

        if all_scrobble_edits.is_empty() {
            return Err(crate::LastFmError::Parse(format!(
                "No scrobble forms found for track '{track_name}' by '{artist_name}'"
            )));
        }

        log::debug!(
            "Final result: found {} unique album variations for '{track_name}' by '{artist_name}'",
            all_scrobble_edits.len(),
        );

        Ok(all_scrobble_edits)
    }

    /// Extract scrobble edit data directly from track page forms. Based on the
    /// approach used in lastfm-bulk-edit
    fn extract_scrobble_edits_from_page(
        &self,
        document: &Html,
        expected_track: &str,
        expected_artist: &str,
        unique_albums: &mut std::collections::HashSet<(String, String)>,
    ) -> Result<Vec<ExactScrobbleEdit>> {
        let mut scrobble_edits = Vec::new();
        // Look for the chartlist table that contains scrobbles
        let table_selector =
            Selector::parse("table.chartlist:not(.chartlist__placeholder)").unwrap();
        let table = document.select(&table_selector).next().ok_or_else(|| {
            crate::LastFmError::Parse("No chartlist table found on track page".to_string())
        })?;

        // Look for table rows that contain scrobble edit forms
        let row_selector = Selector::parse("tr").unwrap();
        for row in table.select(&row_selector) {
            // Check if this row has a count bar link (means it's an aggregation, not individual scrobbles)
            let count_bar_link_selector = Selector::parse(".chartlist-count-bar-link").unwrap();
            if row.select(&count_bar_link_selector).next().is_some() {
                log::debug!("Found count bar link, skipping aggregated row");
                continue;
            }

            // Look for scrobble edit form in this row
            let form_selector = Selector::parse("form[data-edit-scrobble]").unwrap();
            if let Some(form) = row.select(&form_selector).next() {
                // Extract all form values directly
                let extract_form_value = |name: &str| -> Option<String> {
                    let selector = Selector::parse(&format!("input[name='{name}']")).unwrap();
                    form.select(&selector)
                        .next()
                        .and_then(|input| input.value().attr("value"))
                        .map(|s| s.to_string())
                };

                // Get the track and artist from this form
                let form_track = extract_form_value("track_name").unwrap_or_default();
                let form_artist = extract_form_value("artist_name").unwrap_or_default();
                let form_album = extract_form_value("album_name").unwrap_or_default();
                let form_album_artist =
                    extract_form_value("album_artist_name").unwrap_or_else(|| form_artist.clone());
                let form_timestamp = extract_form_value("timestamp").unwrap_or_default();

                log::debug!(
                    "Found scrobble form - Track: '{form_track}', Artist: '{form_artist}', Album: '{form_album}', Timestamp: {form_timestamp}"
                );

                // Check if this form matches the expected track and artist
                if form_track == expected_track && form_artist == expected_artist {
                    // Create a unique key for this album/album_artist combination
                    let album_key = (form_album.clone(), form_album_artist.clone());
                    if unique_albums.insert(album_key) {
                        // Parse timestamp - skip entries without valid timestamps for ExactScrobbleEdit
                        let timestamp = if form_timestamp.is_empty() {
                            None
                        } else {
                            form_timestamp.parse::<u64>().ok()
                        };

                        if let Some(timestamp) = timestamp {
                            log::debug!(
                                "✅ Found unique album variation: '{form_album}' by '{form_album_artist}' for '{expected_track}' by '{expected_artist}'"
                            );

                            // Create ExactScrobbleEdit with all fields specified
                            let scrobble_edit = ExactScrobbleEdit::new(
                                form_track.clone(),
                                form_album.clone(),
                                form_artist.clone(),
                                form_album_artist.clone(),
                                form_track,
                                form_album,
                                form_artist,
                                form_album_artist,
                                timestamp,
                                true,
                            );
                            scrobble_edits.push(scrobble_edit);
                        } else {
                            log::debug!(
                                "⚠️ Skipping album variation without valid timestamp: '{form_album}' by '{form_album_artist}'"
                            );
                        }
                    }
                }
            }
        }

        Ok(scrobble_edits)
    }

    pub async fn get_artist_tracks_page(&self, artist: &str, page: u32) -> Result<TrackPage> {
        // Use AJAX endpoint for page content
        let url = {
            let session = self.session.lock().unwrap();
            format!(
                "{}/user/{}/library/music/{}/+tracks?page={}&ajax=true",
                session.base_url,
                session.username,
                artist.replace(" ", "+"),
                page
            )
        };

        log::debug!("Fetching tracks page {page} for artist: {artist}");
        let mut response = self.get(&url).await?;
        let content = response
            .body_string()
            .await
            .map_err(|e| LastFmError::Http(e.to_string()))?;

        log::debug!(
            "AJAX response: {} status, {} chars",
            response.status(),
            content.len()
        );

        log::debug!("Parsing HTML response from AJAX endpoint");
        let document = Html::parse_document(&content);
        self.parser.parse_tracks_page(&document, page, artist, None)
    }

    /// Extract tracks from HTML document (delegates to parser)
    pub fn extract_tracks_from_document(
        &self,
        document: &Html,
        artist: &str,
        album: Option<&str>,
    ) -> Result<Vec<Track>> {
        self.parser
            .extract_tracks_from_document(document, artist, album)
    }

    /// Parse tracks page (delegates to parser)
    pub fn parse_tracks_page(
        &self,
        document: &Html,
        page_number: u32,
        artist: &str,
        album: Option<&str>,
    ) -> Result<TrackPage> {
        self.parser
            .parse_tracks_page(document, page_number, artist, album)
    }

    /// Parse recent scrobbles from HTML document (for testing)
    pub fn parse_recent_scrobbles(&self, document: &Html) -> Result<Vec<Track>> {
        self.parser.parse_recent_scrobbles(document)
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

    /// Make an HTTP GET request with authentication and retry logic
    pub async fn get(&self, url: &str) -> Result<Response> {
        self.get_with_retry(url, 3).await
    }

    /// Make an HTTP GET request with retry logic for rate limits
    async fn get_with_retry(&self, url: &str, max_retries: u32) -> Result<Response> {
        let config = RetryConfig {
            max_retries,
            base_delay: 30, // Longer base delay for GET requests
            max_delay: 300,
        };

        let url_string = url.to_string();
        let client = self.clone();

        let retry_result = retry::retry_with_backoff(
            config,
            &format!("GET {url}"),
            || async {
                let mut response = client.get_with_redirects(&url_string, 0).await?;

                // Extract body and save debug info if enabled
                let body = client
                    .extract_response_body(&url_string, &mut response)
                    .await?;

                // Check for rate limit patterns in successful responses
                if response.status().is_success() && client.is_rate_limit_response(&body) {
                    log::debug!("Response body contains rate limit patterns");
                    return Err(LastFmError::RateLimit { retry_after: 60 });
                }

                // Recreate response with the body we extracted
                let mut new_response = http_types::Response::new(response.status());
                for (name, values) in response.iter() {
                    for value in values {
                        let _ = new_response.insert_header(name.clone(), value.clone());
                    }
                }
                new_response.set_body(body);

                Ok(new_response)
            },
            |delay, operation_name| {
                self.broadcast_event(ClientEvent::RateLimited {
                    delay_seconds: delay,
                    request: None, // No specific request context in retry callback
                    rate_limit_type: RateLimitType::ResponsePattern,
                });
                log::debug!("{operation_name} rate limited, waiting {delay} seconds");
            },
        )
        .await?;

        Ok(retry_result.result)
    }

    async fn get_with_redirects(&self, url: &str, redirect_count: u32) -> Result<Response> {
        if redirect_count > 5 {
            return Err(LastFmError::Http("Too many redirects".to_string()));
        }

        let mut request = Request::new(Method::Get, url.parse::<Url>().unwrap());

        // Add session cookies for all authenticated requests
        {
            let session = self.session.lock().unwrap();
            headers::add_cookies(&mut request, &session.cookies);
            if session.cookies.is_empty() && url.contains("page=") {
                log::debug!("No cookies available for paginated request!");
            }
        }

        let is_ajax = url.contains("ajax=true");
        let referer_url = if url.contains("page=") {
            Some(url.split('?').next().unwrap_or(url))
        } else {
            None
        };

        headers::add_get_headers(&mut request, is_ajax, referer_url);

        let request_info = RequestInfo::from_url_and_method(url, "GET");
        let request_start = std::time::Instant::now();

        self.broadcast_event(ClientEvent::RequestStarted {
            request: request_info.clone(),
        });

        let response = self
            .client
            .send(request)
            .await
            .map_err(|e| LastFmError::Http(e.to_string()))?;

        self.broadcast_event(ClientEvent::RequestCompleted {
            request: request_info.clone(),
            status_code: response.status().into(),
            duration_ms: request_start.elapsed().as_millis() as u64,
        });

        // Extract any new cookies from the response
        self.extract_cookies(&response);

        // Handle redirects manually
        if response.status() == 302 || response.status() == 301 {
            if let Some(location) = response.header("location") {
                if let Some(redirect_url) = location.get(0) {
                    let redirect_url_str = redirect_url.as_str();
                    if url.contains("page=") {
                        log::debug!("Following redirect from {url} to {redirect_url_str}");

                        // Check if this is a redirect to login - authentication issue
                        if redirect_url_str.contains("/login") {
                            log::debug!("Redirect to login page - authentication failed for paginated request");
                            return Err(LastFmError::Auth(
                                "Session expired or invalid for paginated request".to_string(),
                            ));
                        }
                    }

                    // Handle relative URLs
                    let full_redirect_url = if redirect_url_str.starts_with('/') {
                        let base_url = self.session.lock().unwrap().base_url.clone();
                        format!("{base_url}{redirect_url_str}")
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
                        format!("{base_url}/{redirect_url_str}")
                    };

                    // Make a new request to the redirect URL
                    return Box::pin(
                        self.get_with_redirects(&full_redirect_url, redirect_count + 1),
                    )
                    .await;
                }
            }
        }

        // Handle explicit rate limit responses
        if response.status() == 429 {
            let retry_after = response
                .header("retry-after")
                .and_then(|h| h.get(0))
                .and_then(|v| v.as_str().parse::<u64>().ok())
                .unwrap_or(60);
            self.broadcast_event(ClientEvent::RateLimited {
                delay_seconds: retry_after,
                request: Some(request_info.clone()),
                rate_limit_type: RateLimitType::Http429,
            });
            return Err(LastFmError::RateLimit { retry_after });
        }

        // Check for 403 responses that might be rate limits
        if response.status() == 403 {
            log::debug!("Got 403 response, checking if it's a rate limit");
            // For now, treat 403s from authenticated endpoints as potential rate limits
            {
                let session = self.session.lock().unwrap();
                if !session.cookies.is_empty() {
                    log::debug!("403 on authenticated request - likely rate limit");
                    self.broadcast_event(ClientEvent::RateLimited {
                        delay_seconds: 60,
                        request: Some(request_info.clone()),
                        rate_limit_type: RateLimitType::Http403,
                    });
                    return Err(LastFmError::RateLimit { retry_after: 60 });
                }
            }
        }

        Ok(response)
    }

    /// Check if a response body indicates rate limiting
    fn is_rate_limit_response(&self, response_body: &str) -> bool {
        let body_lower = response_body.to_lowercase();

        // Check against configured rate limit patterns
        for pattern in &self.rate_limit_patterns {
            if body_lower.contains(&pattern.to_lowercase()) {
                return true;
            }
        }

        false
    }

    fn extract_cookies(&self, response: &Response) {
        let mut session = self.session.lock().unwrap();
        extract_cookies_from_response(response, &mut session.cookies);
    }

    /// Extract response body, optionally saving debug info
    async fn extract_response_body(&self, _url: &str, response: &mut Response) -> Result<String> {
        let body = response
            .body_string()
            .await
            .map_err(|e| LastFmError::Http(e.to_string()))?;

        Ok(body)
    }

    pub async fn get_artist_albums_page(&self, artist: &str, page: u32) -> Result<AlbumPage> {
        // Use AJAX endpoint for page content
        let url = {
            let session = self.session.lock().unwrap();
            format!(
                "{}/user/{}/library/music/{}/+albums?page={}&ajax=true",
                session.base_url,
                session.username,
                artist.replace(" ", "+"),
                page
            )
        };

        log::debug!("Fetching albums page {page} for artist: {artist}");
        let mut response = self.get(&url).await?;
        let content = response
            .body_string()
            .await
            .map_err(|e| LastFmError::Http(e.to_string()))?;

        log::debug!(
            "AJAX response: {} status, {} chars",
            response.status(),
            content.len()
        );

        log::debug!("Parsing HTML response from AJAX endpoint");
        let document = Html::parse_document(&content);
        self.parser.parse_albums_page(&document, page, artist)
    }
}

#[async_trait(?Send)]
impl LastFmEditClient for LastFmEditClientImpl {
    fn username(&self) -> String {
        self.username()
    }

    async fn get_recent_scrobbles(&self, page: u32) -> Result<Vec<Track>> {
        self.get_recent_scrobbles(page).await
    }

    async fn find_recent_scrobble_for_track(
        &self,
        track_name: &str,
        artist_name: &str,
        max_pages: u32,
    ) -> Result<Option<Track>> {
        self.find_recent_scrobble_for_track(track_name, artist_name, max_pages)
            .await
    }

    async fn edit_scrobble(&self, edit: &ScrobbleEdit) -> Result<EditResponse> {
        self.edit_scrobble(edit).await
    }

    async fn edit_scrobble_single(
        &self,
        exact_edit: &ExactScrobbleEdit,
        max_retries: u32,
    ) -> Result<EditResponse> {
        self.edit_scrobble_single(exact_edit, max_retries).await
    }

    fn get_session(&self) -> LastFmEditSession {
        self.get_session()
    }

    fn restore_session(&self, session: LastFmEditSession) {
        self.restore_session(session)
    }

    fn subscribe(&self) -> ClientEventReceiver {
        self.subscribe()
    }

    fn latest_event(&self) -> Option<ClientEvent> {
        self.latest_event()
    }

    fn discover_scrobbles(
        &self,
        edit: ScrobbleEdit,
    ) -> Box<dyn crate::AsyncDiscoveryIterator<crate::ExactScrobbleEdit>> {
        let track_name = edit.track_name_original.clone();
        let album_name = edit.album_name_original.clone();

        match (&track_name, &album_name) {
            // Case 1: Track+Album specified - exact match lookup
            (Some(track_name), Some(album_name)) => Box::new(crate::ExactMatchDiscovery::new(
                self.clone(),
                edit,
                track_name.clone(),
                album_name.clone(),
            )),

            // Case 2: Track-specific discovery (discover all album variations of a specific track)
            (Some(track_name), None) => Box::new(crate::TrackVariationsDiscovery::new(
                self.clone(),
                edit,
                track_name.clone(),
            )),

            // Case 3: Album-specific discovery (discover all tracks in a specific album)
            (None, Some(album_name)) => Box::new(crate::AlbumTracksDiscovery::new(
                self.clone(),
                edit,
                album_name.clone(),
            )),

            // Case 4: Artist-specific discovery (discover all tracks by an artist)
            (None, None) => Box::new(crate::ArtistTracksDiscovery::new(self.clone(), edit)),
        }
    }

    async fn get_artist_tracks_page(&self, artist: &str, page: u32) -> Result<TrackPage> {
        self.get_artist_tracks_page(artist, page).await
    }

    async fn get_artist_albums_page(&self, artist: &str, page: u32) -> Result<AlbumPage> {
        self.get_artist_albums_page(artist, page).await
    }

    fn artist_tracks(&self, artist: &str) -> crate::ArtistTracksIterator {
        crate::ArtistTracksIterator::new(self.clone(), artist.to_string())
    }

    fn artist_albums(&self, artist: &str) -> crate::ArtistAlbumsIterator {
        crate::ArtistAlbumsIterator::new(self.clone(), artist.to_string())
    }

    fn album_tracks(&self, album_name: &str, artist_name: &str) -> crate::AlbumTracksIterator {
        crate::AlbumTracksIterator::new(
            self.clone(),
            album_name.to_string(),
            artist_name.to_string(),
        )
    }

    fn recent_tracks(&self) -> crate::RecentTracksIterator {
        crate::RecentTracksIterator::new(self.clone())
    }

    fn recent_tracks_from_page(&self, starting_page: u32) -> crate::RecentTracksIterator {
        crate::RecentTracksIterator::with_starting_page(self.clone(), starting_page)
    }

    async fn validate_session(&self) -> bool {
        self.validate_session().await
    }

    async fn delete_scrobble(
        &self,
        artist_name: &str,
        track_name: &str,
        timestamp: u64,
    ) -> Result<bool> {
        self.delete_scrobble(artist_name, track_name, timestamp)
            .await
    }
}
