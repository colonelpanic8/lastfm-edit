use crate::edit::{ExactScrobbleEdit, SingleEditResponse};
use crate::parsing::LastFmParser;
use crate::session::LastFmEditSession;
use crate::{AlbumPage, EditResponse, LastFmError, Result, ScrobbleEdit, Track, TrackPage};
use async_trait::async_trait;
use http_client::{HttpClient, Request, Response};
use http_types::{Method, Url};
use scraper::{Html, Selector};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Trait for Last.fm client operations that can be mocked for testing.
///
/// This trait abstracts the core functionality needed for Last.fm scrobble editing
/// to enable easy mocking and testing. All methods that perform network operations or
/// state changes are included to support comprehensive test coverage.
///
/// # Mocking Support
///
/// When the `mock` feature is enabled, this crate provides `MockLastFmEditClient`
/// that implements this trait using the `mockall` library.
///
/// # Examples
///
/// ```rust,ignore
/// use lastfm_edit::{LastFmEditClient, MockLastFmEditClient, Result};
///
/// #[cfg(feature = "mock")]
/// async fn test_example() -> Result<()> {
///     let mut mock_client = MockLastFmEditClient::new();
///
///     mock_client
///         .expect_login()
///         .with(eq("user"), eq("pass"))
///         .returning(|_, _| Ok(()));
///
///     mock_client
///         .expect_is_logged_in()
///         .returning(|| true);
///
///     // Use mock_client as &dyn LastFmEditClient
///     let client: &dyn LastFmEditClient = &mock_client;
///     client.login("user", "pass").await?;
///     assert!(client.is_logged_in());
///     Ok(())
/// }
/// ```
#[cfg_attr(feature = "mock", mockall::automock)]
#[async_trait(?Send)]
pub trait LastFmEditClient {
    /// Authenticate with Last.fm using username and password.
    async fn login(&self, username: &str, password: &str) -> Result<()>;

    /// Get the currently authenticated username.
    fn username(&self) -> String;

    /// Check if the client is currently authenticated.
    fn is_logged_in(&self) -> bool;

    /// Fetch recent scrobbles from the user's listening history.
    async fn get_recent_scrobbles(&self, page: u32) -> Result<Vec<Track>>;

    /// Find a scrobble by its timestamp in recent scrobbles.
    async fn find_scrobble_by_timestamp(&self, timestamp: u64) -> Result<Track> {
        log::debug!("Searching for scrobble with timestamp {timestamp}");

        // Search through recent scrobbles to find the one with matching timestamp
        for page in 1..=10 {
            // Search up to 10 pages of recent scrobbles
            let scrobbles = self.get_recent_scrobbles(page).await?;

            for scrobble in scrobbles {
                if let Some(scrobble_timestamp) = scrobble.timestamp {
                    if scrobble_timestamp == timestamp {
                        log::debug!(
                            "Found scrobble: '{}' by '{}' with album: '{:?}', album_artist: '{:?}'",
                            scrobble.name,
                            scrobble.artist,
                            scrobble.album,
                            scrobble.album_artist
                        );
                        return Ok(scrobble);
                    }
                }
            }
        }

        Err(LastFmError::Parse(format!(
            "Could not find scrobble with timestamp {timestamp}"
        )))
    }

    /// Find the most recent scrobble for a specific track.
    async fn find_recent_scrobble_for_track(
        &self,
        track_name: &str,
        artist_name: &str,
        max_pages: u32,
    ) -> Result<Option<Track>>;

    /// Edit a scrobble with the given edit parameters.
    async fn edit_scrobble(&self, edit: &ScrobbleEdit) -> Result<EditResponse>;

    /// Edit a single scrobble with complete information.
    ///
    /// This method performs a single edit operation on a fully-specified scrobble.
    /// Unlike `edit_scrobble`, it does not perform enrichment or multiple edits.
    async fn edit_scrobble_single(
        &self,
        exact_edit: &ExactScrobbleEdit,
        max_retries: u32,
    ) -> Result<EditResponse>;

    /// Discover all unique album variations for a track from the user's library.
    ///
    /// This method scrapes the user's library to find all unique album/album_artist
    /// combinations for the given track and artist, returning fully populated
    /// ScrobbleEdit objects for each variation found.
    async fn discover_album_variations(
        &self,
        track_name: &str,
        artist_name: &str,
    ) -> Result<Vec<ScrobbleEdit>>;

    /// Get tracks from a specific album page.
    async fn get_album_tracks(&self, album_name: &str, artist_name: &str) -> Result<Vec<Track>>;

    /// Edit album metadata by updating scrobbles with new album name.
    async fn edit_album(
        &self,
        old_album_name: &str,
        new_album_name: &str,
        artist_name: &str,
    ) -> Result<EditResponse>;

    /// Edit artist metadata by updating scrobbles with new artist name.
    async fn edit_artist(
        &self,
        old_artist_name: &str,
        new_artist_name: &str,
    ) -> Result<EditResponse>;

    /// Edit artist metadata for a specific track only.
    async fn edit_artist_for_track(
        &self,
        track_name: &str,
        old_artist_name: &str,
        new_artist_name: &str,
    ) -> Result<EditResponse>;

    /// Edit artist metadata for all tracks in a specific album.
    async fn edit_artist_for_album(
        &self,
        album_name: &str,
        old_artist_name: &str,
        new_artist_name: &str,
    ) -> Result<EditResponse>;

    /// Get a page of tracks for a specific artist.
    async fn get_artist_tracks_page(&self, artist: &str, page: u32) -> Result<TrackPage>;

    /// Get a page of albums for a specific artist.
    async fn get_artist_albums_page(&self, artist: &str, page: u32) -> Result<AlbumPage>;

    /// Extract the current session state for persistence.
    fn get_session(&self) -> LastFmEditSession;

    /// Restore session state from a previously saved session.
    fn restore_session(&self, session: LastFmEditSession);

    /// Create an iterator for browsing an artist's tracks from the user's library.
    fn artist_tracks(&self, artist: &str) -> crate::ArtistTracksIterator;

    /// Create an iterator for browsing an artist's albums from the user's library.
    fn artist_albums(&self, artist: &str) -> crate::ArtistAlbumsIterator;

    /// Create an iterator for browsing the user's recent tracks/scrobbles.
    fn recent_tracks(&self) -> crate::RecentTracksIterator;

    /// Create an iterator for browsing the user's recent tracks starting from a specific page.
    fn recent_tracks_from_page(&self, starting_page: u32) -> crate::RecentTracksIterator;
}

/// Main implementation for interacting with Last.fm's web interface.
///
/// This implementation handles authentication, session management, and provides methods for
/// browsing user libraries and editing scrobble data through web scraping.
///
/// # Examples
///
/// ```rust,no_run
/// use lastfm_edit::{LastFmEditClient, LastFmEditClientImpl, Result};
///
/// #[tokio::main]
/// async fn main() -> Result<()> {
///     // Create client with any HTTP implementation
///     let http_client = http_client::native::NativeClient::new();
///     let mut client = LastFmEditClientImpl::new(Box::new(http_client));
///
///     // Login to Last.fm
///     client.login("username", "password").await?;
///
///     // Check if authenticated
///     assert!(client.is_logged_in());
///
///     Ok(())
/// }
/// ```
#[derive(Clone)]
pub struct LastFmEditClientImpl {
    client: Arc<dyn HttpClient + Send + Sync>,
    session: Arc<Mutex<LastFmEditSession>>,
    rate_limit_patterns: Vec<String>,
    debug_save_responses: bool,
    parser: LastFmParser,
}

impl LastFmEditClientImpl {
    /// Create a new [`LastFmEditClient`] with the default Last.fm URL.
    ///
    /// **Note:** This creates an unauthenticated client. You must call [`login`](Self::login)
    /// or [`restore_session`](Self::restore_session) before using most functionality.
    ///
    /// # Arguments
    ///
    /// * `client` - Any HTTP client implementation that implements [`HttpClient`]
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use lastfm_edit::{LastFmEditClient, LastFmEditClientImpl, Result};
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<()> {
    ///     let http_client = http_client::native::NativeClient::new();
    ///     let mut client = LastFmEditClientImpl::new(Box::new(http_client));
    ///     client.login("username", "password").await?;
    ///     Ok(())
    /// }
    /// ```
    pub fn new(client: Box<dyn HttpClient + Send + Sync>) -> Self {
        Self::with_base_url(client, "https://www.last.fm".to_string())
    }

    /// Create a new [`LastFmEditClient`] with a custom base URL.
    ///
    /// **Note:** This creates an unauthenticated client. You must call [`login`](Self::login)
    /// or [`restore_session`](Self::restore_session) before using most functionality.
    ///
    /// This is useful for testing or if Last.fm changes their domain.
    ///
    /// # Arguments
    ///
    /// * `client` - Any HTTP client implementation
    /// * `base_url` - The base URL for Last.fm (e.g., <https://www.last.fm>)
    pub fn with_base_url(client: Box<dyn HttpClient + Send + Sync>, base_url: String) -> Self {
        Self::with_rate_limit_patterns(
            client,
            base_url,
            vec![
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
        )
    }

    /// Create a new [`LastFmEditClient`] with custom rate limit detection patterns.
    ///
    /// # Arguments
    ///
    /// * `client` - Any HTTP client implementation
    /// * `base_url` - The base URL for Last.fm
    /// * `rate_limit_patterns` - Text patterns that indicate rate limiting in responses
    pub fn with_rate_limit_patterns(
        client: Box<dyn HttpClient + Send + Sync>,
        base_url: String,
        rate_limit_patterns: Vec<String>,
    ) -> Self {
        Self {
            client: Arc::from(client),
            session: Arc::new(Mutex::new(LastFmEditSession::new(
                String::new(),
                Vec::new(),
                None,
                base_url,
            ))),
            rate_limit_patterns,
            debug_save_responses: std::env::var("LASTFM_DEBUG_SAVE_RESPONSES").is_ok(),
            parser: LastFmParser::new(),
        }
    }

    /// Create a new authenticated [`LastFmEditClient`] by logging in with username and password.
    ///
    /// This is a convenience method that combines client creation and login into one step.
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
    /// # Examples
    ///
    /// ```rust,no_run
    /// use lastfm_edit::{LastFmEditClient, LastFmEditClientImpl, Result};
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<()> {
    ///     let client = LastFmEditClientImpl::login_with_credentials(
    ///         Box::new(http_client::native::NativeClient::new()),
    ///         "username",
    ///         "password"
    ///     ).await?;
    ///     assert!(client.is_logged_in());
    ///     Ok(())
    /// }
    /// ```
    pub async fn login_with_credentials(
        client: Box<dyn HttpClient + Send + Sync>,
        username: &str,
        password: &str,
    ) -> Result<Self> {
        let new_client = Self::new(client);
        new_client.login(username, password).await?;
        Ok(new_client)
    }

    /// Create a new [`LastFmEditClient`] by restoring a previously saved session.
    ///
    /// This allows you to resume a Last.fm session without requiring the user to log in again.
    ///
    /// # Arguments
    ///
    /// * `client` - Any HTTP client implementation
    /// * `session` - Previously saved [`LastFmEditSession`]
    ///
    /// # Returns
    ///
    /// Returns a client with the restored session.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use lastfm_edit::{LastFmEditClient, LastFmEditClientImpl, LastFmEditSession};
    ///
    /// fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    ///     // Assume we have a saved session
    ///     let session_json = std::fs::read_to_string("session.json")?;
    ///     let session = LastFmEditSession::from_json(&session_json)?;
    ///
    ///     let client = LastFmEditClientImpl::from_session(
    ///         Box::new(http_client::native::NativeClient::new()),
    ///         session
    ///     );
    ///     assert!(client.is_logged_in());
    ///     Ok(())
    /// }
    /// ```
    pub fn from_session(
        client: Box<dyn HttpClient + Send + Sync>,
        session: LastFmEditSession,
    ) -> Self {
        Self {
            client: Arc::from(client),
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
            debug_save_responses: std::env::var("LASTFM_DEBUG_SAVE_RESPONSES").is_ok(),
            parser: LastFmParser::new(),
        }
    }

    /// Extract the current session state for persistence.
    ///
    /// This allows you to save the authentication state and restore it later
    /// without requiring the user to log in again.
    ///
    /// # Returns
    ///
    /// Returns a [`LastFmEditSession`] that can be serialized and saved.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use lastfm_edit::{LastFmEditClient, LastFmEditClientImpl, Result};
    ///
    /// #[tokio::main]
    /// async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    ///     let mut client = LastFmEditClientImpl::new(Box::new(http_client::native::NativeClient::new()));
    ///     client.login("username", "password").await?;
    ///
    ///     // Save session for later use
    ///     let session = client.get_session();
    ///     let session_json = session.to_json()?;
    ///     std::fs::write("session.json", session_json)?;
    ///     Ok(())
    /// }
    /// ```
    pub fn get_session(&self) -> LastFmEditSession {
        self.session.lock().unwrap().clone()
    }

    /// Restore session state from a previously saved [`LastFmEditSession`].
    ///
    /// This allows you to restore authentication state without logging in again.
    ///
    /// # Arguments
    ///
    /// * `session` - Previously saved session state
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use lastfm_edit::{LastFmEditClient, LastFmEditClientImpl, LastFmEditSession};
    ///
    /// fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    ///     let mut client = LastFmEditClientImpl::new(Box::new(http_client::native::NativeClient::new()));
    ///
    ///     // Restore from saved session
    ///     let session_json = std::fs::read_to_string("session.json")?;
    ///     let session = LastFmEditSession::from_json(&session_json)?;
    ///     client.restore_session(session);
    ///
    ///     assert!(client.is_logged_in());
    ///     Ok(())
    /// }
    /// ```
    pub fn restore_session(&self, session: LastFmEditSession) {
        *self.session.lock().unwrap() = session;
    }

    /// Authenticate with Last.fm using username and password.
    ///
    /// This method:
    /// 1. Fetches the login page to extract CSRF tokens
    /// 2. Submits the login form with credentials
    /// 3. Validates the authentication by checking for session cookies
    /// 4. Stores session data for subsequent requests
    ///
    /// # Arguments
    ///
    /// * `username` - Last.fm username or email
    /// * `password` - Last.fm password
    ///
    /// # Returns
    ///
    /// Returns [`Ok(())`] on successful authentication, or [`LastFmError::Auth`] on failure.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # use lastfm_edit::{LastFmEditClient, LastFmEditClientImpl, Result};
    /// # tokio_test::block_on(async {
    /// let mut client = LastFmEditClientImpl::new(Box::new(http_client::native::NativeClient::new()));
    /// client.login("username", "password").await?;
    /// assert!(client.is_logged_in());
    /// # Ok::<(), lastfm_edit::LastFmError>(())
    /// # });
    /// ```
    pub async fn login(&self, username: &str, password: &str) -> Result<()> {
        // Get login page to extract CSRF token
        let login_url = {
            let session = self.session.lock().unwrap();
            format!("{}/login", session.base_url)
        };
        let mut response = self.get(&login_url).await?;

        // Extract any initial cookies from the login page
        self.extract_cookies(&response);

        let html = response
            .body_string()
            .await
            .map_err(|e| LastFmError::Http(e.to_string()))?;

        // Parse HTML synchronously to avoid holding parser state across await boundaries
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
        {
            let session = self.session.lock().unwrap();
            let _ = request.insert_header("Origin", &session.base_url);
        }
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
        {
            let session = self.session.lock().unwrap();
            if !session.cookies.is_empty() {
                let cookie_header = session.cookies.join("; ");
                let _ = request.insert_header("Cookie", &cookie_header);
            }
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

        log::debug!("Login response status: {}", response.status());

        // If we get a 403, it might be rate limiting or auth failure
        if response.status() == 403 {
            // Get the response body to check if it's rate limiting
            let response_html = response
                .body_string()
                .await
                .map_err(|e| LastFmError::Http(e.to_string()))?;

            // Look for rate limit indicators in the response
            if self.is_rate_limit_response(&response_html) {
                log::debug!("403 response appears to be rate limiting");
                return Err(LastFmError::RateLimit { retry_after: 60 });
            }
            log::debug!("403 response appears to be authentication failure");

            // Continue with the normal auth failure handling using the response_html
            let login_error = self.parse_login_error(&response_html);
            return Err(LastFmError::Auth(login_error));
        }

        // Check if we got a new sessionid that looks like a real Last.fm session
        let has_real_session = {
            let session = self.session.lock().unwrap();
            session
                .cookies
                .iter()
                .any(|cookie| cookie.starts_with("sessionid=.") && cookie.len() > 50)
        };

        if has_real_session && (response.status() == 302 || response.status() == 200) {
            // We got a real session ID, login was successful
            {
                let mut session = self.session.lock().unwrap();
                session.username = username.to_string();
                session.csrf_token = Some(csrf_token);
            }
            log::debug!("Login successful - authenticated session established");
            return Ok(());
        }

        // At this point, we didn't get a 403, so read the response body for other cases
        let response_html = response
            .body_string()
            .await
            .map_err(|e| LastFmError::Http(e.to_string()))?;

        // Check if we were redirected away from login page (success) by parsing synchronously
        let has_login_form = self.check_for_login_form(&response_html);

        if !has_login_form && response.status() == 200 {
            {
                let mut session = self.session.lock().unwrap();
                session.username = username.to_string();
                session.csrf_token = Some(csrf_token);
            }
            Ok(())
        } else {
            // Parse error messages synchronously
            let error_msg = self.parse_login_error(&response_html);
            Err(LastFmError::Auth(error_msg))
        }
    }

    /// Get the currently authenticated username.
    ///
    /// Returns an empty string if not logged in.
    pub fn username(&self) -> String {
        self.session.lock().unwrap().username.clone()
    }

    /// Check if the client is currently authenticated.
    ///
    /// Returns `true` if [`login`](Self::login) was successful and session is active.
    pub fn is_logged_in(&self) -> bool {
        self.session.lock().unwrap().is_valid()
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
        // First, try to enrich the edit with complete metadata from library page
        let enriched_edits = self.enrich_edit_metadata(edit).await?;

        if enriched_edits.is_empty() {
            return Err(LastFmError::Parse(format!(
                "No scrobbles found for track '{}' by '{}' in your library. Make sure the track and artist names are correct and that you have scrobbled this track recently.",
                edit.track_name_original, edit.artist_name_original
            )));
        }

        // Perform all enriched edits and collect results
        let mut results = Vec::new();

        for (i, exact_edit) in enriched_edits.iter().enumerate() {
            let album_info = format!(
                "{} by {}",
                exact_edit.album_name_original, exact_edit.album_artist_name_original
            );

            log::debug!(
                "Performing edit {}/{} for album '{}'",
                i + 1,
                enriched_edits.len(),
                album_info
            );

            let single_response = self.edit_scrobble_single(exact_edit, 3).await?;
            let success = single_response.success();
            let message = single_response.message();

            results.push(SingleEditResponse {
                success,
                message,
                album_info: Some(album_info),
            });

            // Add a small delay between edits to be respectful
            if i < enriched_edits.len() - 1 {
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            }
        }

        Ok(EditResponse::from_results(results))
    }

    /// Enrich a ScrobbleEdit with complete metadata by scraping the track library page
    /// This returns a collection of ExactScrobbleEdits, one for each unique album/album_artist combination
    async fn enrich_edit_metadata(&self, edit: &ScrobbleEdit) -> Result<Vec<ExactScrobbleEdit>> {
        log::debug!(
            "Enriching metadata for track '{}' by '{}'",
            edit.track_name_original,
            edit.artist_name_original
        );

        // Scrape the track library page to get all album variations
        self.load_edit_form_values_internal(&edit.track_name_original, &edit.artist_name_original)
            .await
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
    /// # Examples
    /// ```rust,ignore
    /// use lastfm_edit::{LastFmEditClientImpl, ExactScrobbleEdit};
    ///
    /// let exact_edit = ExactScrobbleEdit::new(
    ///     "Original Track".to_string(),
    ///     "Original Album".to_string(),
    ///     "Original Artist".to_string(),
    ///     "Original Artist".to_string(),
    ///     "New Track".to_string(),
    ///     "New Album".to_string(),
    ///     "New Artist".to_string(),
    ///     "New Artist".to_string(),
    ///     1640995200,
    ///     false,
    /// );
    ///
    /// let response = client.edit_scrobble_single(&exact_edit, 3).await?;
    /// ```
    pub async fn edit_scrobble_single(
        &self,
        exact_edit: &ExactScrobbleEdit,
        max_retries: u32,
    ) -> Result<EditResponse> {
        let edit = exact_edit.to_scrobble_edit();
        let mut retries = 0;

        loop {
            match self.edit_scrobble_impl(&edit).await {
                Ok(success) => {
                    return Ok(EditResponse::single(success, None, None));
                }
                Err(LastFmError::RateLimit { retry_after }) => {
                    if retries >= max_retries {
                        log::warn!("Max retries ({max_retries}) exceeded for edit operation");
                        return Ok(EditResponse::single(
                            false,
                            Some(format!("Rate limit exceeded after {max_retries} retries")),
                            None,
                        ));
                    }

                    let delay = std::cmp::min(retry_after, 2_u64.pow(retries + 1) * 5);
                    log::info!(
                        "Edit rate limited. Waiting {} seconds before retry {} of {}",
                        delay,
                        retries + 1,
                        max_retries
                    );
                    // Rate limit delay would go here
                    retries += 1;
                }
                Err(other_error) => {
                    return Ok(EditResponse::single(
                        false,
                        Some(other_error.to_string()),
                        None,
                    ));
                }
            }
        }
    }

    async fn edit_scrobble_impl(&self, edit: &ScrobbleEdit) -> Result<bool> {
        if !self.is_logged_in() {
            return Err(LastFmError::Auth(
                "Must be logged in to edit scrobbles".to_string(),
            ));
        }

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

        let mut form_data = HashMap::new();

        // Add fresh CSRF token (required)
        form_data.insert("csrfmiddlewaretoken", fresh_csrf_token.as_str());

        // Include ALL form fields
        form_data.insert("track_name_original", &edit.track_name_original);
        form_data.insert("track_name", &edit.track_name);
        form_data.insert("artist_name_original", &edit.artist_name_original);
        form_data.insert("artist_name", &edit.artist_name);

        let album_name_original = edit.album_name_original.as_deref().unwrap_or("");
        let album_artist_name_original = edit.album_artist_name_original.as_deref().unwrap_or("");
        form_data.insert("album_name_original", album_name_original);
        form_data.insert("album_name", &edit.album_name);
        form_data.insert("album_artist_name_original", album_artist_name_original);
        form_data.insert("album_artist_name", &edit.album_artist_name);

        // Include timestamp if available
        let timestamp_str;
        if let Some(timestamp) = edit.timestamp {
            timestamp_str = timestamp.to_string();
            form_data.insert("timestamp", &timestamp_str);
        }

        // Edit flags
        if edit.edit_all {
            form_data.insert("edit_all", "1");
        }
        form_data.insert("submit", "edit-scrobble");
        form_data.insert("ajax", "1");

        log::debug!(
            "Editing scrobble: '{}' -> '{}'",
            edit.track_name_original,
            edit.track_name
        );
        {
            let session = self.session.lock().unwrap();
            log::trace!("Session cookies count: {}", session.cookies.len());
        }

        let mut request = Request::new(Method::Post, edit_url.parse::<Url>().unwrap());

        // Add comprehensive headers matching your browser request
        let _ = request.insert_header("Accept", "*/*");
        let _ = request.insert_header("Accept-Language", "en-US,en;q=0.9");
        let _ = request.insert_header(
            "Content-Type",
            "application/x-www-form-urlencoded;charset=UTF-8",
        );
        let _ = request.insert_header("Priority", "u=1, i");
        let _ = request.insert_header("User-Agent", "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/138.0.0.0 Safari/537.36");
        let _ = request.insert_header("X-Requested-With", "XMLHttpRequest");
        let _ = request.insert_header("Sec-Fetch-Dest", "empty");
        let _ = request.insert_header("Sec-Fetch-Mode", "cors");
        let _ = request.insert_header("Sec-Fetch-Site", "same-origin");
        let _ = request.insert_header(
            "sec-ch-ua",
            "\"Not)A;Brand\";v=\"8\", \"Chromium\";v=\"138\", \"Google Chrome\";v=\"138\"",
        );
        let _ = request.insert_header("sec-ch-ua-mobile", "?0");
        let _ = request.insert_header("sec-ch-ua-platform", "\"Linux\"");

        // Add session cookies
        {
            let session = self.session.lock().unwrap();
            if !session.cookies.is_empty() {
                let cookie_header = session.cookies.join("; ");
                let _ = request.insert_header("Cookie", &cookie_header);
            }
        }

        // Add referer header - use the current artist being edited
        {
            let session = self.session.lock().unwrap();
            let _ = request.insert_header(
                "Referer",
                format!("{}/user/{}/library", session.base_url, session.username),
            );
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

        log::debug!("Edit response status: {}", response.status());

        let response_text = response
            .body_string()
            .await
            .map_err(|e| LastFmError::Http(e.to_string()))?;

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

        // If not found, try extracting from the raw response text using generic patterns
        if actual_track_name.is_none() || actual_album_name.is_none() {
            // Look for track name in href="/music/{artist}/_/{track}"
            // Use regex to find track URLs
            let track_pattern = regex::Regex::new(r#"href="/music/[^"]+/_/([^"]+)""#).unwrap();
            if let Some(captures) = track_pattern.captures(&response_text) {
                if let Some(track_match) = captures.get(1) {
                    let raw_track = track_match.as_str();
                    // URL decode the track name
                    let decoded_track = urlencoding::decode(raw_track)
                        .unwrap_or_else(|_| raw_track.into())
                        .replace("+", " ");
                    actual_track_name = Some(decoded_track);
                }
            }

            // Look for album name in href="/music/{artist}/{album}"
            // Find album links that are not track links (don't contain /_/)
            let album_pattern =
                regex::Regex::new(r#"href="/music/[^"]+/([^"/_]+)"[^>]*>[^<]*</a>"#).unwrap();
            if let Some(captures) = album_pattern.captures(&response_text) {
                if let Some(album_match) = captures.get(1) {
                    let raw_album = album_match.as_str();
                    // URL decode the album name
                    let decoded_album = urlencoding::decode(raw_album)
                        .unwrap_or_else(|_| raw_album.into())
                        .replace("+", " ");
                    actual_album_name = Some(decoded_album);
                }
            }
        }

        log::debug!(
            "Response analysis: success_alert={}, error_alert={}, track='{}', album='{}'",
            has_success_alert,
            has_error_alert,
            actual_track_name.as_deref().unwrap_or("not found"),
            actual_album_name.as_deref().unwrap_or("not found")
        );

        // Determine if edit was truly successful
        let final_success = response.status().is_success() && has_success_alert && !has_error_alert;

        // Create detailed message
        let _message = if has_error_alert {
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
        } else {
            Some(format!("Edit failed with status: {}", response.status()))
        };

        Ok(final_success)
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
    async fn load_edit_form_values_internal(
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

    /// Extract scrobble edit data directly from track page forms
    /// Based on the approach used in lastfm-bulk-edit
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

    /// Discover all unique album variations for a track from the user's library (public API)
    pub async fn discover_album_variations(
        &self,
        track_name: &str,
        artist_name: &str,
    ) -> Result<Vec<ScrobbleEdit>> {
        let exact_edits = self
            .load_edit_form_values_internal(track_name, artist_name)
            .await?;
        Ok(exact_edits
            .iter()
            .map(|edit| edit.to_scrobble_edit())
            .collect())
    }

    /// Get tracks from a specific album page
    /// This makes a single request to the album page and extracts track data
    pub async fn get_album_tracks(
        &self,
        album_name: &str,
        artist_name: &str,
    ) -> Result<Vec<Track>> {
        log::debug!("Getting tracks from album '{album_name}' by '{artist_name}'");

        // Get the album page directly - this should contain track listings
        let album_url = {
            let session = self.session.lock().unwrap();
            format!(
                "{}/user/{}/library/music/{}/{}",
                session.base_url,
                session.username,
                urlencoding::encode(artist_name),
                urlencoding::encode(album_name)
            )
        };

        log::debug!("Fetching album page: {album_url}");

        let mut response = self.get(&album_url).await?;
        let html = response
            .body_string()
            .await
            .map_err(|e| LastFmError::Http(e.to_string()))?;

        let document = Html::parse_document(&html);

        // Use the shared track extraction function
        let tracks =
            self.parser
                .extract_tracks_from_document(&document, artist_name, Some(album_name))?;

        log::debug!(
            "Successfully parsed {} tracks from album page",
            tracks.len()
        );
        Ok(tracks)
    }

    /// Edit album metadata by updating scrobbles with new album name
    /// This edits ALL tracks from the album that are found in recent scrobbles
    pub async fn edit_album(
        &self,
        old_album_name: &str,
        new_album_name: &str,
        artist_name: &str,
    ) -> Result<EditResponse> {
        log::debug!("Editing album '{old_album_name}' -> '{new_album_name}' by '{artist_name}'");

        // Get all tracks from the album page
        let tracks = self.get_album_tracks(old_album_name, artist_name).await?;

        if tracks.is_empty() {
            return Ok(EditResponse::single(
                false,
                Some(format!(
                    "No tracks found for album '{old_album_name}' by '{artist_name}'. Make sure the album name matches exactly."
                )),
                None
            ));
        }

        log::info!(
            "Found {} tracks in album '{}'",
            tracks.len(),
            old_album_name
        );

        let mut all_results = Vec::new();

        // For each track, create a simple edit and let edit_scrobble handle the complexity
        for (index, track) in tracks.iter().enumerate() {
            log::debug!(
                "Processing track {}/{}: '{}'",
                index + 1,
                tracks.len(),
                track.name
            );

            // Create a simple ScrobbleEdit for this track with the new album name
            let edit = ScrobbleEdit::from_track_and_artist(&track.name, artist_name)
                .with_album_name(new_album_name);

            // Let edit_scrobble handle all the enrichment and multiple album variations
            match self.edit_scrobble(&edit).await {
                Ok(response) => {
                    let total_edits = response.total_edits();
                    let successful_edits = response.successful_edits();

                    // Add all individual results to our collection
                    all_results.extend(response.individual_results);

                    log::info!(
                        "Processed track '{}': {} edits ({} successful)",
                        track.name,
                        total_edits,
                        successful_edits
                    );
                }
                Err(e) => {
                    // If we can't edit this track, add a failure result
                    all_results.push(SingleEditResponse {
                        success: false,
                        message: Some(format!("Error editing track '{}': {}", track.name, e)),
                        album_info: Some(format!("track from album '{old_album_name}'")),
                    });
                    log::debug!("❌ Error editing track '{}': {}", track.name, e);
                }
            }

            // Add delay between track edits to be respectful to the server
            tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
        }

        Ok(EditResponse::from_results(all_results))
    }

    /// Edit artist metadata by updating scrobbles with new artist name
    /// This edits ALL tracks from the artist that are found in recent scrobbles
    pub async fn edit_artist(
        &self,
        old_artist_name: &str,
        new_artist_name: &str,
    ) -> Result<EditResponse> {
        log::debug!("Editing artist '{old_artist_name}' -> '{new_artist_name}'");

        // Get all tracks from the artist using pagination
        let mut tracks = Vec::new();
        let mut page = 1;
        let max_pages = 10; // Limit to reasonable number to avoid infinite processing

        loop {
            if page > max_pages || tracks.len() >= 200 {
                break;
            }

            match self.get_artist_tracks_page(old_artist_name, page).await {
                Ok(track_page) => {
                    if track_page.tracks.is_empty() {
                        break;
                    }
                    tracks.extend(track_page.tracks);
                    if !track_page.has_next_page {
                        break;
                    }
                    page += 1;
                }
                Err(e) => {
                    log::warn!("Error fetching artist tracks page {page}: {e}");
                    break;
                }
            }
        }

        if tracks.is_empty() {
            return Ok(EditResponse::single(
                false,
                Some(format!(
                    "No tracks found for artist '{old_artist_name}'. Make sure the artist name matches exactly."
                )),
                None
            ));
        }

        log::info!(
            "Found {} tracks for artist '{}'",
            tracks.len(),
            old_artist_name
        );

        let mut successful_edits = 0;
        let mut failed_edits = 0;
        let mut error_messages = Vec::new();
        let mut skipped_tracks = 0;

        // For each track, try to load and edit it
        for (index, track) in tracks.iter().enumerate() {
            log::debug!(
                "Processing track {}/{}: '{}'",
                index + 1,
                tracks.len(),
                track.name
            );

            match self
                .load_edit_form_values_internal(&track.name, old_artist_name)
                .await
            {
                Ok(edit_data_list) => {
                    // Use the first (most common) album variation
                    if let Some(edit_data) = edit_data_list.into_iter().next() {
                        let mut edit_data = edit_data.to_scrobble_edit();
                        // Update the artist name and album artist name
                        edit_data.artist_name = new_artist_name.to_string();
                        edit_data.album_artist_name = new_artist_name.to_string();

                        // Perform the edit
                        match self.edit_scrobble(&edit_data).await {
                            Ok(response) => {
                                if response.success() {
                                    successful_edits += 1;
                                    log::info!("✅ Successfully edited track '{}'", track.name);
                                } else {
                                    failed_edits += 1;
                                    let error_msg = format!(
                                        "Failed to edit track '{}': {}",
                                        track.name,
                                        response
                                            .message()
                                            .unwrap_or_else(|| "Unknown error".to_string())
                                    );
                                    error_messages.push(error_msg);
                                    log::debug!("❌ {}", error_messages.last().unwrap());
                                }
                            }
                            Err(e) => {
                                failed_edits += 1;
                                let error_msg =
                                    format!("Error editing track '{}': {}", track.name, e);
                                error_messages.push(error_msg);
                                log::info!("❌ {}", error_messages.last().unwrap());
                            }
                        }
                    } else {
                        skipped_tracks += 1;
                        log::debug!("No edit data found for track '{}'", track.name);
                    }
                }
                Err(e) => {
                    skipped_tracks += 1;
                    log::debug!("Could not load edit form for track '{}': {e}", track.name);
                    // Continue to next track - some tracks might not be in recent scrobbles
                }
            }

            // Add delay between edits to be respectful to the server
        }

        let total_processed = successful_edits + failed_edits;
        let success = successful_edits > 0 && failed_edits == 0;

        let message = if success {
            Some(format!(
                "Successfully renamed artist '{old_artist_name}' to '{new_artist_name}' for all {successful_edits} editable tracks ({skipped_tracks} tracks were not in recent scrobbles)"
            ))
        } else if successful_edits > 0 {
            Some(format!(
                "Partially successful: {} of {} editable tracks renamed ({} skipped, {} failed). Errors: {}",
                successful_edits,
                total_processed,
                skipped_tracks,
                failed_edits,
                error_messages.join("; ")
            ))
        } else if total_processed == 0 {
            Some(format!(
                "No editable tracks found for artist '{}'. All {} tracks were skipped because they're not in recent scrobbles.",
                old_artist_name, tracks.len()
            ))
        } else {
            Some(format!(
                "Failed to rename any tracks. Errors: {}",
                error_messages.join("; ")
            ))
        };

        Ok(EditResponse::single(success, message, None))
    }

    /// Edit artist metadata for a specific track only
    /// This edits only the specified track if found in recent scrobbles
    pub async fn edit_artist_for_track(
        &self,
        track_name: &str,
        old_artist_name: &str,
        new_artist_name: &str,
    ) -> Result<EditResponse> {
        log::debug!("Editing artist for track '{track_name}' from '{old_artist_name}' -> '{new_artist_name}'");

        match self.load_edit_form_values_internal(track_name, old_artist_name).await {
            Ok(edit_data_list) => {
                // Use the first (most common) album variation
                if let Some(edit_data) = edit_data_list.into_iter().next() {
                    let mut edit_data = edit_data.to_scrobble_edit();
                    // Update the artist name and album artist name
                    edit_data.artist_name = new_artist_name.to_string();
                    edit_data.album_artist_name = new_artist_name.to_string();

                    log::info!("Updating artist for track '{track_name}' from '{old_artist_name}' to '{new_artist_name}'");

                    // Perform the edit
                    match self.edit_scrobble(&edit_data).await {
                    Ok(response) => {
                        if response.success() {
                            Ok(EditResponse::single(
                                true,
                                Some(format!(
                                    "Successfully renamed artist for track '{track_name}' from '{old_artist_name}' to '{new_artist_name}'"
                                )),
                                None
                            ))
                        } else {
                            Ok(EditResponse::single(
                                false,
                                Some(format!(
                                    "Failed to rename artist for track '{track_name}': {}",
                                    response.message().unwrap_or_else(|| "Unknown error".to_string())
                                )),
                                None
                            ))
                        }
                    }
                    Err(e) => Ok(EditResponse::single(
                        false,
                        Some(format!("Error editing track '{track_name}': {e}")),
                        None
                    )),
                    }
                } else {
                    Ok(EditResponse::single(
                        false,
                        Some(format!(
                            "No edit data found for track '{track_name}' by '{old_artist_name}'. The track may not be in your recent scrobbles."
                        )),
                        None
                    ))
                }
            }
            Err(e) => Ok(EditResponse::single(
                false,
                Some(format!(
                    "Could not load edit form for track '{track_name}' by '{old_artist_name}': {e}. The track may not be in your recent scrobbles."
                )),
                None
            )),
        }
    }

    /// Edit artist metadata for all tracks in a specific album
    /// This edits ALL tracks from the specified album that are found in recent scrobbles
    pub async fn edit_artist_for_album(
        &self,
        album_name: &str,
        old_artist_name: &str,
        new_artist_name: &str,
    ) -> Result<EditResponse> {
        log::debug!("Editing artist for album '{album_name}' from '{old_artist_name}' -> '{new_artist_name}'");

        // Get all tracks from the album page
        let tracks = self.get_album_tracks(album_name, old_artist_name).await?;

        if tracks.is_empty() {
            return Ok(EditResponse::single(
                false,
                Some(format!(
                    "No tracks found for album '{album_name}' by '{old_artist_name}'. Make sure the album name matches exactly."
                )),
                None
            ));
        }

        log::info!(
            "Found {} tracks in album '{}' by '{}'",
            tracks.len(),
            album_name,
            old_artist_name
        );

        let mut successful_edits = 0;
        let mut failed_edits = 0;
        let mut error_messages = Vec::new();
        let mut skipped_tracks = 0;

        // For each track, try to load and edit it
        for (index, track) in tracks.iter().enumerate() {
            log::debug!(
                "Processing track {}/{}: '{}'",
                index + 1,
                tracks.len(),
                track.name
            );

            match self
                .load_edit_form_values_internal(&track.name, old_artist_name)
                .await
            {
                Ok(edit_data_list) => {
                    // Use the first (most common) album variation
                    if let Some(edit_data) = edit_data_list.into_iter().next() {
                        let mut edit_data = edit_data.to_scrobble_edit();
                        // Update the artist name and album artist name
                        edit_data.artist_name = new_artist_name.to_string();
                        edit_data.album_artist_name = new_artist_name.to_string();

                        // Perform the edit
                        match self.edit_scrobble(&edit_data).await {
                            Ok(response) => {
                                if response.success() {
                                    successful_edits += 1;
                                    log::info!("✅ Successfully edited track '{}'", track.name);
                                } else {
                                    failed_edits += 1;
                                    let error_msg = format!(
                                        "Failed to edit track '{}': {}",
                                        track.name,
                                        response
                                            .message()
                                            .unwrap_or_else(|| "Unknown error".to_string())
                                    );
                                    error_messages.push(error_msg);
                                    log::debug!("❌ {}", error_messages.last().unwrap());
                                }
                            }
                            Err(e) => {
                                failed_edits += 1;
                                let error_msg =
                                    format!("Error editing track '{}': {}", track.name, e);
                                error_messages.push(error_msg);
                                log::info!("❌ {}", error_messages.last().unwrap());
                            }
                        }
                    } else {
                        skipped_tracks += 1;
                        log::debug!("No edit data found for track '{}'", track.name);
                    }
                }
                Err(e) => {
                    skipped_tracks += 1;
                    log::debug!("Could not load edit form for track '{}': {e}", track.name);
                    // Continue to next track - some tracks might not be in recent scrobbles
                }
            }

            // Add delay between edits to be respectful to the server
        }

        let total_processed = successful_edits + failed_edits;
        let success = successful_edits > 0 && failed_edits == 0;

        let message = if success {
            Some(format!(
                "Successfully renamed artist for album '{album_name}' from '{old_artist_name}' to '{new_artist_name}' for all {successful_edits} editable tracks ({skipped_tracks} tracks were not in recent scrobbles)"
            ))
        } else if successful_edits > 0 {
            Some(format!(
                "Partially successful: {} of {} editable tracks renamed ({} skipped, {} failed). Errors: {}",
                successful_edits,
                total_processed,
                skipped_tracks,
                failed_edits,
                error_messages.join("; ")
            ))
        } else if total_processed == 0 {
            Some(format!(
                "No editable tracks found for album '{album_name}' by '{old_artist_name}'. All {} tracks were skipped because they're not in recent scrobbles.",
                tracks.len()
            ))
        } else {
            Some(format!(
                "Failed to rename any tracks. Errors: {}",
                error_messages.join("; ")
            ))
        };

        Ok(EditResponse::single(success, message, None))
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

        // Check if we got JSON or HTML
        if content.trim_start().starts_with("{") || content.trim_start().starts_with("[") {
            log::debug!("Parsing JSON response from AJAX endpoint");
            self.parse_json_tracks_page(&content, page, artist)
        } else {
            log::debug!("Parsing HTML response from AJAX endpoint");
            let document = Html::parse_document(&content);
            self.parser.parse_tracks_page(&document, page, artist, None)
        }
    }

    /// Parse JSON tracks page (delegates to parser)
    fn parse_json_tracks_page(
        &self,
        _json_content: &str,
        page_number: u32,
        _artist: &str,
    ) -> Result<TrackPage> {
        // JSON parsing not yet implemented - fallback to empty page
        log::debug!("JSON parsing not implemented, returning empty page");
        Ok(TrackPage {
            tracks: Vec::new(),
            page_number,
            has_next_page: false,
            total_pages: Some(1),
        })
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

    /// Extract login form data (CSRF token and next field) - synchronous parsing helper
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

    /// Parse login error messages from HTML - synchronous parsing helper
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

    /// Check if HTML contains a login form - synchronous parsing helper
    fn check_for_login_form(&self, html: &str) -> bool {
        let document = Html::parse_document(html);
        let login_form_selector =
            Selector::parse("form[action*=\"login\"], input[name=\"username_or_email\"]").unwrap();
        document.select(&login_form_selector).next().is_some()
    }

    /// Make an HTTP GET request with authentication and retry logic
    pub async fn get(&self, url: &str) -> Result<Response> {
        self.get_with_retry(url, 3).await
    }

    /// Make an HTTP GET request with retry logic for rate limits
    async fn get_with_retry(&self, url: &str, max_retries: u32) -> Result<Response> {
        let mut retries = 0;

        loop {
            match self.get_with_redirects(url, 0).await {
                Ok(mut response) => {
                    // Extract body and save debug info if enabled
                    let body = self.extract_response_body(url, &mut response).await?;

                    // Check for rate limit patterns in successful responses
                    if response.status().is_success() && self.is_rate_limit_response(&body) {
                        log::debug!("Response body contains rate limit patterns");
                        if retries < max_retries {
                            let delay = 60 + (retries as u64 * 30); // Exponential backoff
                            log::info!("Rate limit detected in response body, retrying in {delay}s (attempt {}/{max_retries})", retries + 1);
                            // Rate limit delay would go here
                            retries += 1;
                            continue;
                        }
                        return Err(crate::LastFmError::RateLimit { retry_after: 60 });
                    }

                    // Recreate response with the body we extracted
                    let mut new_response = http_types::Response::new(response.status());
                    for (name, values) in response.iter() {
                        for value in values {
                            let _ = new_response.insert_header(name.clone(), value.clone());
                        }
                    }
                    new_response.set_body(body);

                    return Ok(new_response);
                }
                Err(crate::LastFmError::RateLimit { retry_after }) => {
                    if retries < max_retries {
                        let delay = retry_after + (retries as u64 * 30); // Exponential backoff
                        log::info!(
                            "Rate limit detected, retrying in {delay}s (attempt {}/{max_retries})",
                            retries + 1
                        );
                        // Rate limit delay would go here
                        retries += 1;
                    } else {
                        return Err(crate::LastFmError::RateLimit { retry_after });
                    }
                }
                Err(e) => return Err(e),
            }
        }
    }

    async fn get_with_redirects(&self, url: &str, redirect_count: u32) -> Result<Response> {
        if redirect_count > 5 {
            return Err(LastFmError::Http("Too many redirects".to_string()));
        }

        let mut request = Request::new(Method::Get, url.parse::<Url>().unwrap());
        let _ = request.insert_header("User-Agent", "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/138.0.0.0 Safari/537.36");

        // Add session cookies for all authenticated requests
        {
            let session = self.session.lock().unwrap();
            if !session.cookies.is_empty() {
                let cookie_header = session.cookies.join("; ");
                let _ = request.insert_header("Cookie", &cookie_header);
            } else if url.contains("page=") {
                log::debug!("No cookies available for paginated request!");
            }
        }

        // Add browser-like headers for all requests
        if url.contains("ajax=true") {
            // AJAX request headers
            let _ = request.insert_header("Accept", "*/*");
            let _ = request.insert_header("X-Requested-With", "XMLHttpRequest");
        } else {
            // Regular page request headers
            let _ = request.insert_header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.7");
        }
        let _ = request.insert_header("Accept-Language", "en-US,en;q=0.9");
        let _ = request.insert_header("Accept-Encoding", "gzip, deflate, br");
        let _ = request.insert_header("DNT", "1");
        let _ = request.insert_header("Connection", "keep-alive");
        let _ = request.insert_header("Upgrade-Insecure-Requests", "1");

        // Add referer for paginated requests
        if url.contains("page=") {
            let base_url = url.split('?').next().unwrap_or(url);
            let _ = request.insert_header("Referer", base_url);
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
        // Extract Set-Cookie headers and store them (avoiding duplicates)
        if let Some(cookie_headers) = response.header("set-cookie") {
            let mut new_cookies = 0;
            for cookie_header in cookie_headers {
                let cookie_str = cookie_header.as_str();
                // Extract just the cookie name=value part (before any semicolon)
                if let Some(cookie_value) = cookie_str.split(';').next() {
                    let cookie_name = cookie_value.split('=').next().unwrap_or("");

                    // Remove any existing cookie with the same name
                    {
                        let mut session = self.session.lock().unwrap();
                        session
                            .cookies
                            .retain(|existing| !existing.starts_with(&format!("{cookie_name}=")));
                        session.cookies.push(cookie_value.to_string());
                    }
                    new_cookies += 1;
                }
            }
            if new_cookies > 0 {
                {
                    let session = self.session.lock().unwrap();
                    log::trace!(
                        "Extracted {} new cookies, total: {}",
                        new_cookies,
                        session.cookies.len()
                    );
                    log::trace!("Updated cookies: {:?}", &session.cookies);

                    // Check if sessionid changed
                    for cookie in &session.cookies {
                        if cookie.starts_with("sessionid=") {
                            log::trace!("Current sessionid: {}", &cookie[10..50.min(cookie.len())]);
                            break;
                        }
                    }
                }
            }
        }
    }

    /// Extract response body, optionally saving debug info
    async fn extract_response_body(&self, url: &str, response: &mut Response) -> Result<String> {
        let body = response
            .body_string()
            .await
            .map_err(|e| LastFmError::Http(e.to_string()))?;

        if self.debug_save_responses {
            self.save_debug_response(url, response.status().into(), &body);
        }

        Ok(body)
    }

    /// Save response to debug directory (optional debug feature)
    fn save_debug_response(&self, url: &str, status_code: u16, body: &str) {
        if let Err(e) = self.try_save_debug_response(url, status_code, body) {
            log::warn!("Failed to save debug response: {e}");
        }
    }

    /// Internal debug response saving implementation
    fn try_save_debug_response(&self, url: &str, status_code: u16, body: &str) -> Result<()> {
        // Create debug directory if it doesn't exist
        let debug_dir = Path::new("debug_responses");
        if !debug_dir.exists() {
            fs::create_dir_all(debug_dir)
                .map_err(|e| LastFmError::Http(format!("Failed to create debug directory: {e}")))?;
        }

        // Extract the path part of the URL (after base_url)
        let url_path = {
            let session = self.session.lock().unwrap();
            if url.starts_with(&session.base_url) {
                &url[session.base_url.len()..]
            } else {
                url
            }
        };

        // Create safe filename from URL path and add timestamp
        let now = chrono::Utc::now();
        let timestamp = now.format("%Y%m%d_%H%M%S_%3f");
        let safe_path = url_path.replace(['/', '?', '&', '=', '%', '+'], "_");

        let filename = format!("{timestamp}_{safe_path}_status{status_code}.html");
        let file_path = debug_dir.join(filename);

        // Write response to file
        fs::write(&file_path, body)
            .map_err(|e| LastFmError::Http(format!("Failed to write debug file: {e}")))?;

        log::debug!(
            "Saved HTTP response to {file_path:?} (status: {status_code}, url: {url_path})"
        );

        Ok(())
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

        // Check if we got JSON or HTML
        if content.trim_start().starts_with("{") || content.trim_start().starts_with("[") {
            log::debug!("Parsing JSON response from AJAX endpoint");
            self.parse_json_albums_page(&content, page, artist)
        } else {
            log::debug!("Parsing HTML response from AJAX endpoint");
            let document = Html::parse_document(&content);
            self.parser.parse_albums_page(&document, page, artist)
        }
    }

    fn parse_json_albums_page(
        &self,
        _json_content: &str,
        page_number: u32,
        _artist: &str,
    ) -> Result<AlbumPage> {
        // JSON parsing not yet implemented - fallback to empty page
        log::debug!("JSON parsing not implemented, returning empty page");
        Ok(AlbumPage {
            albums: Vec::new(),
            page_number,
            has_next_page: false,
            total_pages: Some(1),
        })
    }
}

#[async_trait(?Send)]
impl LastFmEditClient for LastFmEditClientImpl {
    async fn login(&self, username: &str, password: &str) -> Result<()> {
        self.login(username, password).await
    }

    fn username(&self) -> String {
        self.username()
    }

    fn is_logged_in(&self) -> bool {
        self.is_logged_in()
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

    async fn discover_album_variations(
        &self,
        track_name: &str,
        artist_name: &str,
    ) -> Result<Vec<ScrobbleEdit>> {
        self.discover_album_variations(track_name, artist_name)
            .await
    }

    async fn get_album_tracks(&self, album_name: &str, artist_name: &str) -> Result<Vec<Track>> {
        self.get_album_tracks(album_name, artist_name).await
    }

    async fn edit_album(
        &self,
        old_album_name: &str,
        new_album_name: &str,
        artist_name: &str,
    ) -> Result<EditResponse> {
        self.edit_album(old_album_name, new_album_name, artist_name)
            .await
    }

    async fn edit_artist(
        &self,
        old_artist_name: &str,
        new_artist_name: &str,
    ) -> Result<EditResponse> {
        self.edit_artist(old_artist_name, new_artist_name).await
    }

    async fn edit_artist_for_track(
        &self,
        track_name: &str,
        old_artist_name: &str,
        new_artist_name: &str,
    ) -> Result<EditResponse> {
        self.edit_artist_for_track(track_name, old_artist_name, new_artist_name)
            .await
    }

    async fn edit_artist_for_album(
        &self,
        album_name: &str,
        old_artist_name: &str,
        new_artist_name: &str,
    ) -> Result<EditResponse> {
        self.edit_artist_for_album(album_name, old_artist_name, new_artist_name)
            .await
    }

    async fn get_artist_tracks_page(&self, artist: &str, page: u32) -> Result<TrackPage> {
        self.get_artist_tracks_page(artist, page).await
    }

    async fn get_artist_albums_page(&self, artist: &str, page: u32) -> Result<AlbumPage> {
        self.get_artist_albums_page(artist, page).await
    }

    fn get_session(&self) -> LastFmEditSession {
        self.get_session()
    }

    fn restore_session(&self, session: LastFmEditSession) {
        self.restore_session(session)
    }

    fn artist_tracks(&self, artist: &str) -> crate::ArtistTracksIterator {
        crate::ArtistTracksIterator::new(self.clone(), artist.to_string())
    }

    fn artist_albums(&self, artist: &str) -> crate::ArtistAlbumsIterator {
        crate::ArtistAlbumsIterator::new(self.clone(), artist.to_string())
    }

    fn recent_tracks(&self) -> crate::RecentTracksIterator {
        crate::RecentTracksIterator::new(self.clone())
    }

    fn recent_tracks_from_page(&self, starting_page: u32) -> crate::RecentTracksIterator {
        crate::RecentTracksIterator::with_starting_page(self.clone(), starting_page)
    }
}
