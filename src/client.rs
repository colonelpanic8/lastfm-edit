use crate::edit_analysis;
use crate::headers;
use crate::login::extract_cookies_from_response;
use crate::parsing::LastFmParser;
use crate::r#trait::LastFmEditClient;
use crate::retry;
use crate::types::{
    AlbumPage, ClientConfig, ClientEvent, ClientEventReceiver, EditResponse, ExactScrobbleEdit,
    LastFmEditSession, LastFmError, OperationalDelayConfig, RateLimitConfig, RateLimitType,
    RequestInfo, RetryConfig, ScrobbleEdit, SharedEventBroadcaster, SingleEditResponse, Track,
    TrackPage,
};
use crate::Result;
use async_trait::async_trait;
use http_client::{HttpClient, Request, Response};
use http_types::{Method, Url};
use scraper::{Html, Selector};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct LastFmEditClientImpl {
    client: Arc<dyn HttpClient + Send + Sync>,
    session: Arc<Mutex<LastFmEditSession>>,
    parser: LastFmParser,
    broadcaster: Arc<SharedEventBroadcaster>,
    config: ClientConfig,
}

impl LastFmEditClientImpl {
    /// Custom URL encoding for Last.fm paths
    fn lastfm_encode(&self, input: &str) -> String {
        urlencoding::encode(input).to_string()
    }

    /// Detect if the response content indicates a login redirect
    fn is_login_redirect(&self, content: &str) -> bool {
        // Check for common login redirect indicators
        content.contains("login")
            || content.contains("sign in")
            || content.contains("signin")
            || content.contains("Log in to Last.fm")
            || content.contains("Please sign in")
            // Check for login form elements
            || (content.contains("<form") && content.contains("password"))
            // Check for authentication-related classes or IDs
            || content.contains("auth-form")
            || content.contains("login-form")
    }

    /// Check if a specific endpoint requires authentication that our session doesn't provide
    pub async fn validate_endpoint_access(&self, url: &str) -> Result<bool> {
        let mut response = self.get(url).await?;
        let content = response
            .body_string()
            .await
            .map_err(|e| LastFmError::Http(e.to_string()))?;

        Ok(!self.is_login_redirect(&content))
    }
    pub fn from_session(
        client: Box<dyn HttpClient + Send + Sync>,
        session: LastFmEditSession,
    ) -> Self {
        Self::from_session_with_arc(Arc::from(client), session)
    }

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

    pub fn from_session_with_rate_limit_patterns(
        client: Box<dyn HttpClient + Send + Sync>,
        session: LastFmEditSession,
        rate_limit_patterns: Vec<String>,
    ) -> Self {
        let config = ClientConfig::default()
            .with_rate_limit_config(RateLimitConfig::default().with_patterns(rate_limit_patterns));
        Self::from_session_with_client_config(client, session, config)
    }

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

    pub fn from_session_with_client_config(
        client: Box<dyn HttpClient + Send + Sync>,
        session: LastFmEditSession,
        config: ClientConfig,
    ) -> Self {
        Self::from_session_with_client_config_arc(Arc::from(client), session, config)
    }

    pub async fn login_with_credentials_and_client_config(
        client: Box<dyn HttpClient + Send + Sync>,
        username: &str,
        password: &str,
        config: ClientConfig,
    ) -> Result<Self> {
        let client_arc: Arc<dyn HttpClient + Send + Sync> = Arc::from(client);
        let login_manager =
            crate::login::LoginManager::new(client_arc.clone(), "https://www.last.fm".to_string());
        let session = login_manager.login(username, password).await?;
        Ok(Self::from_session_with_client_config_arc(
            client_arc, session, config,
        ))
    }

    pub fn from_session_with_config(
        client: Box<dyn HttpClient + Send + Sync>,
        session: LastFmEditSession,
        retry_config: RetryConfig,
        rate_limit_config: RateLimitConfig,
    ) -> Self {
        Self::from_session_with_config_arc(
            Arc::from(client),
            session,
            retry_config,
            rate_limit_config,
        )
    }

    pub async fn login_with_credentials_and_config(
        client: Box<dyn HttpClient + Send + Sync>,
        username: &str,
        password: &str,
        retry_config: RetryConfig,
        rate_limit_config: RateLimitConfig,
    ) -> Result<Self> {
        let client_arc: Arc<dyn HttpClient + Send + Sync> = Arc::from(client);
        let login_manager =
            crate::login::LoginManager::new(client_arc.clone(), "https://www.last.fm".to_string());
        let session = login_manager.login(username, password).await?;
        Ok(Self::from_session_with_config_arc(
            client_arc,
            session,
            retry_config,
            rate_limit_config,
        ))
    }

    fn from_session_with_broadcaster(
        client: Box<dyn HttpClient + Send + Sync>,
        session: LastFmEditSession,
        broadcaster: Arc<SharedEventBroadcaster>,
    ) -> Self {
        Self::from_session_with_broadcaster_arc(Arc::from(client), session, broadcaster)
    }

    fn from_session_with_client_config_arc(
        client: Arc<dyn HttpClient + Send + Sync>,
        session: LastFmEditSession,
        config: ClientConfig,
    ) -> Self {
        Self::from_session_with_client_config_and_broadcaster_arc(
            client,
            session,
            config,
            Arc::new(SharedEventBroadcaster::new()),
        )
    }

    fn from_session_with_config_arc(
        client: Arc<dyn HttpClient + Send + Sync>,
        session: LastFmEditSession,
        retry_config: RetryConfig,
        rate_limit_config: RateLimitConfig,
    ) -> Self {
        let config = ClientConfig {
            retry: retry_config,
            rate_limit: rate_limit_config,
            operational_delays: OperationalDelayConfig::default(),
        };
        Self::from_session_with_client_config_arc(client, session, config)
    }

    fn from_session_with_broadcaster_arc(
        client: Arc<dyn HttpClient + Send + Sync>,
        session: LastFmEditSession,
        broadcaster: Arc<SharedEventBroadcaster>,
    ) -> Self {
        Self::from_session_with_client_config_and_broadcaster_arc(
            client,
            session,
            ClientConfig::default(),
            broadcaster,
        )
    }

    fn from_session_with_client_config_and_broadcaster_arc(
        client: Arc<dyn HttpClient + Send + Sync>,
        session: LastFmEditSession,
        config: ClientConfig,
        broadcaster: Arc<SharedEventBroadcaster>,
    ) -> Self {
        Self {
            client,
            session: Arc::new(Mutex::new(session)),
            parser: LastFmParser::new(),
            broadcaster,
            config,
        }
    }

    pub fn get_session(&self) -> LastFmEditSession {
        self.session.lock().unwrap().clone()
    }

    pub fn restore_session(&self, session: LastFmEditSession) {
        *self.session.lock().unwrap() = session;
    }

    pub fn with_shared_broadcaster(&self, client: Box<dyn HttpClient + Send + Sync>) -> Self {
        let session = self.get_session();
        Self::from_session_with_broadcaster(client, session, self.broadcaster.clone())
    }

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
            enabled: true,
        };

        let artist_name = artist_name.to_string();
        let track_name = track_name.to_string();
        let client = self.clone();

        match retry::retry_with_backoff(
            config,
            "Delete scrobble",
            || client.delete_scrobble_impl(&artist_name, &track_name, timestamp),
            |delay, rate_limit_timestamp, operation_name| {
                self.broadcast_event(ClientEvent::RateLimited {
                    delay_seconds: delay,
                    request: None,
                    rate_limit_type: RateLimitType::ResponsePattern,
                    rate_limit_timestamp,
                });
                log::debug!("{operation_name} rate limited, waiting {delay} seconds");
            },
            |total_duration, _operation_name| {
                self.broadcast_event(ClientEvent::RateLimitEnded {
                    request: crate::types::RequestInfo::from_url_and_method(
                        &format!("delete_scrobble/{artist_name}/{track_name}/{timestamp}"),
                        "POST",
                    ),
                    rate_limit_type: RateLimitType::ResponsePattern,
                    total_rate_limit_duration_seconds: total_duration,
                });
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

        let referer_url = {
            let session = self.session.lock().unwrap();
            headers::add_cookies(&mut request, &session.cookies);
            format!("{}/user/{}", session.base_url, session.username)
        };

        headers::add_edit_headers(&mut request, &referer_url);

        let form_data = [
            ("csrfmiddlewaretoken", fresh_csrf_token.as_str()),
            ("artist_name", artist_name),
            ("track_name", track_name),
            ("timestamp", &timestamp.to_string()),
            ("ajax", "1"),
        ];

        let form_string: String = form_data
            .iter()
            .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
            .collect::<Vec<_>>()
            .join("&");

        request.set_body(form_string);

        log::debug!(
            "Deleting scrobble: '{track_name}' by '{artist_name}' with timestamp {timestamp}"
        );

        let request_info = RequestInfo::from_url_and_method(&delete_url, "POST");
        let request_start = std::time::Instant::now();

        self.broadcast_event(ClientEvent::RequestStarted {
            request: request_info.clone(),
        });

        let mut response = self
            .client
            .send(request)
            .await
            .map_err(|e| LastFmError::Http(e.to_string()))?;

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

        let success = response.status().is_success();

        if success {
            log::debug!("Successfully deleted scrobble");
        } else {
            log::debug!("Delete failed with response: {response_text}");
        }

        Ok(success)
    }

    pub fn subscribe(&self) -> ClientEventReceiver {
        self.broadcaster.subscribe()
    }

    pub fn latest_event(&self) -> Option<ClientEvent> {
        self.broadcaster.latest_event()
    }

    fn broadcast_event(&self, event: ClientEvent) {
        self.broadcaster.broadcast_event(event);
    }

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

    pub async fn get_recent_tracks_page(&self, page: u32) -> Result<TrackPage> {
        let tracks = self.get_recent_scrobbles(page).await?;

        let has_next_page = !tracks.is_empty();

        Ok(TrackPage {
            tracks,
            page_number: page,
            has_next_page,
            total_pages: None,
        })
    }

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

        for (index, discovered_edit) in discovered_edits.iter().enumerate() {
            log::debug!(
                "Processing scrobble {}/{}: '{}' from '{}'",
                index + 1,
                discovered_edits.len(),
                discovered_edit.track_name_original,
                discovered_edit.album_name_original
            );

            let mut modified_exact_edit = discovered_edit.clone();

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

            if index < discovered_edits.len() - 1
                && self.config.operational_delays.edit_delay_ms > 0
            {
                tokio::time::sleep(std::time::Duration::from_millis(
                    self.config.operational_delays.edit_delay_ms,
                ))
                .await;
            }
        }

        Ok(EditResponse::from_results(all_results))
    }

    pub async fn edit_scrobble_single(
        &self,
        exact_edit: &ExactScrobbleEdit,
        max_retries: u32,
    ) -> Result<EditResponse> {
        let config = RetryConfig {
            max_retries,
            base_delay: 5,
            max_delay: 300,
            enabled: true,
        };

        let edit_clone = exact_edit.clone();
        let client = self.clone();

        match retry::retry_with_backoff(
            config,
            "Edit scrobble",
            || client.edit_scrobble_impl(&edit_clone),
            |delay, rate_limit_timestamp, operation_name| {
                self.broadcast_event(ClientEvent::RateLimited {
                    delay_seconds: delay,
                    request: None, // No specific request context in retry callback
                    rate_limit_type: RateLimitType::ResponsePattern,
                    rate_limit_timestamp,
                });
                log::debug!("{operation_name} rate limited, waiting {delay} seconds");
            },
            |total_duration, _operation_name| {
                self.broadcast_event(ClientEvent::RateLimitEnded {
                    request: crate::types::RequestInfo::from_url_and_method(
                        &format!(
                            "edit_scrobble/{}/{}",
                            edit_clone.artist_name, edit_clone.track_name
                        ),
                        "POST",
                    ),
                    rate_limit_type: RateLimitType::ResponsePattern,
                    total_rate_limit_duration_seconds: total_duration,
                });
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
        let form_html = self.get_edit_form_html(&edit_url).await?;

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

        let referer_url = {
            let session = self.session.lock().unwrap();
            headers::add_cookies(&mut request, &session.cookies);
            format!("{}/user/{}/library", session.base_url, session.username)
        };

        headers::add_edit_headers(&mut request, &referer_url);

        let form_string: String = form_data
            .iter()
            .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
            .collect::<Vec<_>>()
            .join("&");

        request.set_body(form_string);

        let request_info = RequestInfo::from_url_and_method(&edit_url, "POST");
        let request_start = std::time::Instant::now();

        self.broadcast_event(ClientEvent::RequestStarted {
            request: request_info.clone(),
        });

        let mut response = self
            .client
            .send(request)
            .await
            .map_err(|e| LastFmError::Http(e.to_string()))?;

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

        let analysis = edit_analysis::analyze_edit_response(&response_text, response.status());

        Ok(analysis.success)
    }

    async fn get_edit_form_html(&self, edit_url: &str) -> Result<String> {
        let mut form_response = self.get(edit_url).await?;
        let form_html = form_response
            .body_string()
            .await
            .map_err(|e| LastFmError::Http(e.to_string()))?;

        log::debug!("Edit form response status: {}", form_response.status());
        Ok(form_html)
    }

    pub async fn load_edit_form_values_internal(
        &self,
        track_name: &str,
        artist_name: &str,
    ) -> Result<Vec<ExactScrobbleEdit>> {
        log::debug!("Loading edit form values for '{track_name}' by '{artist_name}'");

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

        let mut all_scrobble_edits = Vec::new();
        let mut unique_albums = std::collections::HashSet::new();
        let max_pages = 5;

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

        let pagination_selector = Selector::parse(".pagination .pagination-next").unwrap();
        let mut has_next_page = document.select(&pagination_selector).next().is_some();
        let mut page = 2;

        while has_next_page && page <= max_pages {
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

    fn extract_scrobble_edits_from_page(
        &self,
        document: &Html,
        expected_track: &str,
        expected_artist: &str,
        unique_albums: &mut std::collections::HashSet<(String, String)>,
    ) -> Result<Vec<ExactScrobbleEdit>> {
        let mut scrobble_edits = Vec::new();
        let table_selector =
            Selector::parse("table.chartlist:not(.chartlist__placeholder)").unwrap();
        let table = document.select(&table_selector).next().ok_or_else(|| {
            crate::LastFmError::Parse("No chartlist table found on track page".to_string())
        })?;

        let row_selector = Selector::parse("tr").unwrap();
        for row in table.select(&row_selector) {
            let count_bar_link_selector = Selector::parse(".chartlist-count-bar-link").unwrap();
            if row.select(&count_bar_link_selector).next().is_some() {
                log::debug!("Found count bar link, skipping aggregated row");
                continue;
            }

            let form_selector = Selector::parse("form[data-edit-scrobble]").unwrap();
            if let Some(form) = row.select(&form_selector).next() {
                let extract_form_value = |name: &str| -> Option<String> {
                    let selector = Selector::parse(&format!("input[name='{name}']")).unwrap();
                    form.select(&selector)
                        .next()
                        .and_then(|input| input.value().attr("value"))
                        .map(|s| s.to_string())
                };

                let form_track = extract_form_value("track_name").unwrap_or_default();
                let form_artist = extract_form_value("artist_name").unwrap_or_default();
                let form_album = extract_form_value("album_name").unwrap_or_default();
                let form_album_artist =
                    extract_form_value("album_artist_name").unwrap_or_else(|| form_artist.clone());
                let form_timestamp = extract_form_value("timestamp").unwrap_or_default();

                if form_track == expected_track && form_artist == expected_artist {
                    let album_key = (form_album.clone(), form_album_artist.clone());
                    if unique_albums.insert(album_key) {
                        let timestamp = if form_timestamp.is_empty() {
                            None
                        } else {
                            form_timestamp.parse::<u64>().ok()
                        };

                        if let Some(timestamp) = timestamp {
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
                            log::warn!(
                                "⚠️ Skipping form without valid timestamp: '{form_album}' by '{form_album_artist}'"
                            );
                        }
                    }
                }
            }
        }

        Ok(scrobble_edits)
    }

    pub async fn get_artist_tracks_page(&self, artist: &str, page: u32) -> Result<TrackPage> {
        let url = {
            let session = self.session.lock().unwrap();
            format!(
                "{}/user/{}/library/music/{}/+tracks?page={}&ajax=true",
                session.base_url,
                session.username,
                urlencoding::encode(artist),
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

    pub fn extract_tracks_from_document(
        &self,
        document: &Html,
        artist: &str,
        album: Option<&str>,
    ) -> Result<Vec<Track>> {
        self.parser
            .extract_tracks_from_document(document, artist, album)
    }

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

    pub async fn get(&self, url: &str) -> Result<Response> {
        self.get_with_retry(url).await
    }

    async fn get_with_retry(&self, url: &str) -> Result<Response> {
        let config = self.config.retry.clone();

        let url_string = url.to_string();
        let client = self.clone();

        let retry_result = retry::retry_with_backoff(
            config,
            &format!("GET {url}"),
            || async {
                let mut response = client.get_with_redirects(&url_string, 0).await?;

                let body = client
                    .extract_response_body(&url_string, &mut response)
                    .await?;

                if response.status().is_success() && client.is_rate_limit_response(&body) {
                    log::debug!("Response body contains rate limit patterns");
                    return Err(LastFmError::RateLimit { retry_after: 60 });
                }

                let mut new_response = http_types::Response::new(response.status());
                for (name, values) in response.iter() {
                    for value in values {
                        let _ = new_response.insert_header(name.clone(), value.clone());
                    }
                }
                new_response.set_body(body);

                Ok(new_response)
            },
            |delay, rate_limit_timestamp, operation_name| {
                self.broadcast_event(ClientEvent::RateLimited {
                    delay_seconds: delay,
                    request: None, // No specific request context in retry callback
                    rate_limit_type: RateLimitType::ResponsePattern,
                    rate_limit_timestamp,
                });
                log::debug!("{operation_name} rate limited, waiting {delay} seconds");
            },
            |total_duration, _operation_name| {
                self.broadcast_event(ClientEvent::RateLimitEnded {
                    request: crate::types::RequestInfo::from_url_and_method(&url_string, "GET"),
                    rate_limit_type: RateLimitType::ResponsePattern,
                    total_rate_limit_duration_seconds: total_duration,
                });
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

        self.extract_cookies(&response);

        if response.status() == 302 || response.status() == 301 {
            if let Some(location) = response.header("location") {
                if let Some(redirect_url) = location.get(0) {
                    let redirect_url_str = redirect_url.as_str();
                    if url.contains("page=") {
                        log::debug!("Following redirect from {url} to {redirect_url_str}");

                        if redirect_url_str.contains("/login") {
                            log::debug!("Redirect to login page - authentication failed for paginated request");
                            return Err(LastFmError::Auth(
                                "Session expired or invalid for paginated request".to_string(),
                            ));
                        }
                    }

                    let full_redirect_url = if redirect_url_str.starts_with('/') {
                        let base_url = self.session.lock().unwrap().base_url.clone();
                        format!("{base_url}{redirect_url_str}")
                    } else if redirect_url_str.starts_with("http") {
                        redirect_url_str.to_string()
                    } else {
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

                    return Box::pin(
                        self.get_with_redirects(&full_redirect_url, redirect_count + 1),
                    )
                    .await;
                }
            }
        }

        if self.config.rate_limit.detect_by_status && response.status() == 429 {
            let retry_after = response
                .header("retry-after")
                .and_then(|h| h.get(0))
                .and_then(|v| v.as_str().parse::<u64>().ok())
                .unwrap_or(60);
            self.broadcast_event(ClientEvent::RateLimited {
                delay_seconds: retry_after,
                request: Some(request_info.clone()),
                rate_limit_type: RateLimitType::Http429,
                rate_limit_timestamp: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            });
            return Err(LastFmError::RateLimit { retry_after });
        }

        if self.config.rate_limit.detect_by_status && response.status() == 403 {
            log::debug!("Got 403 response, checking if it's a rate limit");
            {
                let session = self.session.lock().unwrap();
                if !session.cookies.is_empty() {
                    log::debug!("403 on authenticated request - likely rate limit");
                    self.broadcast_event(ClientEvent::RateLimited {
                        delay_seconds: 60,
                        request: Some(request_info.clone()),
                        rate_limit_type: RateLimitType::Http403,
                        rate_limit_timestamp: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs(),
                    });
                    return Err(LastFmError::RateLimit { retry_after: 60 });
                }
            }
        }

        Ok(response)
    }

    fn is_rate_limit_response(&self, response_body: &str) -> bool {
        let rate_limit_config = &self.config.rate_limit;

        if !rate_limit_config.detect_by_patterns && rate_limit_config.custom_patterns.is_empty() {
            return false;
        }

        let body_lower = response_body.to_lowercase();

        for pattern in &rate_limit_config.custom_patterns {
            if body_lower.contains(&pattern.to_lowercase()) {
                log::debug!("Rate limit detected (custom pattern: '{pattern}')");
                return true;
            }
        }

        if rate_limit_config.detect_by_patterns {
            for pattern in &rate_limit_config.patterns {
                let pattern_lower = pattern.to_lowercase();
                if body_lower.contains(&pattern_lower) {
                    log::debug!("Rate limit detected (pattern: '{pattern}')");
                    return true;
                }
            }
        }

        false
    }

    fn extract_cookies(&self, response: &Response) {
        let mut session = self.session.lock().unwrap();
        extract_cookies_from_response(response, &mut session.cookies);
    }

    async fn extract_response_body(&self, _url: &str, response: &mut Response) -> Result<String> {
        let body = response
            .body_string()
            .await
            .map_err(|e| LastFmError::Http(e.to_string()))?;

        Ok(body)
    }

    pub async fn get_artists_page(&self, page: u32) -> Result<crate::ArtistPage> {
        let url = {
            let session = self.session.lock().unwrap();
            format!(
                "{}/user/{}/library/artists?page={}",
                session.base_url, session.username, page
            )
        };

        log::debug!("Fetching artists page {page}");
        let mut response = self.get(&url).await?;
        let content = response
            .body_string()
            .await
            .map_err(|e| LastFmError::Http(e.to_string()))?;

        log::debug!(
            "Artist library response: {} status, {} chars",
            response.status(),
            content.len()
        );

        log::debug!("Parsing HTML response from artist library endpoint");
        let document = Html::parse_document(&content);
        self.parser.parse_artists_page(&document, page)
    }

    pub async fn get_artist_albums_page(&self, artist: &str, page: u32) -> Result<AlbumPage> {
        let url = {
            let session = self.session.lock().unwrap();
            format!(
                "{}/user/{}/library/music/{}/+albums?page={}&ajax=true",
                session.base_url,
                session.username,
                urlencoding::encode(artist),
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

    pub async fn get_album_tracks_page(
        &self,
        album_name: &str,
        artist_name: &str,
        page: u32,
    ) -> Result<TrackPage> {
        let url = {
            let session = self.session.lock().unwrap();
            format!(
                "{}/user/{}/library/music/{}/{}?page={}&ajax=true",
                session.base_url,
                session.username,
                self.lastfm_encode(artist_name),
                self.lastfm_encode(album_name),
                page
            )
        };

        log::debug!("Fetching tracks page {page} for album '{album_name}' by '{artist_name}'");
        log::debug!("🔗 Album URL: {url}");

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
        let result =
            self.parser
                .parse_tracks_page(&document, page, artist_name, Some(album_name))?;

        // Debug logging for albums that return 0 tracks
        if result.tracks.is_empty() {
            if content.contains("404") || content.contains("Not Found") {
                log::warn!("🚨 404 ERROR for album '{album_name}' by '{artist_name}': {url}");
            } else if content.contains("no tracks") || content.contains("no music") {
                log::debug!("ℹ️  Album '{album_name}' by '{artist_name}' explicitly has no tracks in user's library");
            } else {
                log::warn!(
                    "🚨 UNKNOWN EMPTY RESPONSE for album '{album_name}' by '{artist_name}': {url}"
                );
                log::debug!("🔍 Response length: {} chars", content.len());
                log::debug!(
                    "🔍 Response preview (first 200 chars): {}",
                    &content.chars().take(200).collect::<String>()
                );
            }
        } else {
            log::debug!(
                "✅ SUCCESS: Album '{album_name}' by '{artist_name}' returned {} tracks",
                result.tracks.len()
            );
        }

        Ok(result)
    }

    pub async fn search_tracks_page(&self, query: &str, page: u32) -> Result<TrackPage> {
        let url = {
            let session = self.session.lock().unwrap();
            format!(
                "{}/user/{}/library/tracks/search?page={}&query={}&ajax=1",
                session.base_url,
                session.username,
                page,
                urlencoding::encode(query)
            )
        };

        log::debug!("Searching tracks for query '{query}' on page {page}");
        let mut response = self.get(&url).await?;
        let content = response
            .body_string()
            .await
            .map_err(|e| LastFmError::Http(e.to_string()))?;

        log::debug!(
            "Track search response: {} status, {} chars",
            response.status(),
            content.len()
        );

        let document = Html::parse_document(&content);
        let tracks = self.parser.parse_track_search_results(&document)?;

        // For search results, we need to determine pagination differently
        // since we don't have the same pagination structure as regular library pages
        let (has_next_page, total_pages) = self.parser.parse_pagination(&document, page)?;

        Ok(TrackPage {
            tracks,
            page_number: page,
            has_next_page,
            total_pages,
        })
    }

    pub async fn search_albums_page(&self, query: &str, page: u32) -> Result<AlbumPage> {
        let url = {
            let session = self.session.lock().unwrap();
            format!(
                "{}/user/{}/library/albums/search?page={}&query={}&ajax=1",
                session.base_url,
                session.username,
                page,
                urlencoding::encode(query)
            )
        };

        log::debug!("Searching albums for query '{query}' on page {page}");
        let mut response = self.get(&url).await?;
        let content = response
            .body_string()
            .await
            .map_err(|e| LastFmError::Http(e.to_string()))?;

        log::debug!(
            "Album search response: {} status, {} chars",
            response.status(),
            content.len()
        );

        let document = Html::parse_document(&content);
        let albums = self.parser.parse_album_search_results(&document)?;

        // For search results, we need to determine pagination differently
        let (has_next_page, total_pages) = self.parser.parse_pagination(&document, page)?;

        Ok(AlbumPage {
            albums,
            page_number: page,
            has_next_page,
            total_pages,
        })
    }

    /// Expose the inner HTTP client for advanced use cases like VCR cassette management
    pub fn inner_client(&self) -> Arc<dyn HttpClient + Send + Sync> {
        self.client.clone()
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
            (Some(track_name), Some(album_name)) => Box::new(crate::ExactMatchDiscovery::new(
                self.clone(),
                edit,
                track_name.clone(),
                album_name.clone(),
            )),

            (Some(track_name), None) => Box::new(crate::TrackVariationsDiscovery::new(
                self.clone(),
                edit,
                track_name.clone(),
            )),

            (None, Some(album_name)) => Box::new(crate::AlbumTracksDiscovery::new(
                self.clone(),
                edit,
                album_name.clone(),
            )),

            (None, None) => Box::new(crate::ArtistTracksDiscovery::new(self.clone(), edit)),
        }
    }

    async fn get_artists_page(&self, page: u32) -> Result<crate::ArtistPage> {
        self.get_artists_page(page).await
    }

    async fn get_artist_tracks_page(&self, artist: &str, page: u32) -> Result<TrackPage> {
        self.get_artist_tracks_page(artist, page).await
    }

    async fn get_artist_albums_page(&self, artist: &str, page: u32) -> Result<AlbumPage> {
        self.get_artist_albums_page(artist, page).await
    }

    async fn get_album_tracks_page(
        &self,
        album_name: &str,
        artist_name: &str,
        page: u32,
    ) -> Result<TrackPage> {
        self.get_album_tracks_page(album_name, artist_name, page)
            .await
    }

    fn artists(&self) -> Box<dyn crate::AsyncPaginatedIterator<crate::Artist>> {
        Box::new(crate::iterator::ArtistsIterator::new(self.clone()))
    }

    fn artist_tracks(&self, artist: &str) -> Box<dyn crate::AsyncPaginatedIterator<Track>> {
        Box::new(crate::ArtistTracksIterator::new(
            self.clone(),
            artist.to_string(),
        ))
    }

    fn artist_tracks_direct(&self, artist: &str) -> Box<dyn crate::AsyncPaginatedIterator<Track>> {
        Box::new(crate::iterator::ArtistTracksDirectIterator::new(
            self.clone(),
            artist.to_string(),
        ))
    }

    fn artist_albums(&self, artist: &str) -> Box<dyn crate::AsyncPaginatedIterator<crate::Album>> {
        Box::new(crate::ArtistAlbumsIterator::new(
            self.clone(),
            artist.to_string(),
        ))
    }

    fn album_tracks(
        &self,
        album_name: &str,
        artist_name: &str,
    ) -> Box<dyn crate::AsyncPaginatedIterator<Track>> {
        Box::new(crate::AlbumTracksIterator::new(
            self.clone(),
            album_name.to_string(),
            artist_name.to_string(),
        ))
    }

    fn recent_tracks(&self) -> Box<dyn crate::AsyncPaginatedIterator<Track>> {
        Box::new(crate::RecentTracksIterator::new(self.clone()))
    }

    fn recent_tracks_from_page(
        &self,
        starting_page: u32,
    ) -> Box<dyn crate::AsyncPaginatedIterator<Track>> {
        Box::new(crate::RecentTracksIterator::with_starting_page(
            self.clone(),
            starting_page,
        ))
    }

    fn search_tracks(&self, query: &str) -> Box<dyn crate::AsyncPaginatedIterator<Track>> {
        Box::new(crate::SearchTracksIterator::new(
            self.clone(),
            query.to_string(),
        ))
    }

    fn search_albums(&self, query: &str) -> Box<dyn crate::AsyncPaginatedIterator<crate::Album>> {
        Box::new(crate::SearchAlbumsIterator::new(
            self.clone(),
            query.to_string(),
        ))
    }

    async fn search_tracks_page(&self, query: &str, page: u32) -> Result<crate::TrackPage> {
        self.search_tracks_page(query, page).await
    }

    async fn search_albums_page(&self, query: &str, page: u32) -> Result<crate::AlbumPage> {
        self.search_albums_page(query, page).await
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
