use crate::edit::ExactScrobbleEdit;
use crate::events::{ClientEvent, ClientEventReceiver};
use crate::iterator::AsyncPaginatedIterator;
use crate::session::LastFmEditSession;
use crate::{Album, EditResponse, LastFmError, Result, ScrobbleEdit, Track};
use async_trait::async_trait;

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
#[cfg_attr(feature = "mock", mockall::automock)]
#[async_trait(?Send)]
pub trait LastFmEditClient {
    // =============================================================================
    // CORE EDITING METHODS - Most important functionality
    // =============================================================================

    /// Edit scrobbles by discovering and updating all matching instances.
    ///
    /// This is the main editing method that automatically discovers all scrobble instances
    /// that match the provided criteria and applies the specified changes to each one.
    ///
    /// # How it works
    ///
    /// 1. **Discovery**: Analyzes the `ScrobbleEdit` to determine what to search for:
    ///    - If `track_name_original` is specified: finds all album variations of that track
    ///    - If only `album_name_original` is specified: finds all tracks in that album
    ///    - If neither is specified: finds all tracks by that artist
    ///
    /// 2. **Enrichment**: For each discovered scrobble, extracts complete metadata
    ///    including album artist information from the user's library
    ///
    /// 3. **Editing**: Applies the requested changes to each discovered instance
    ///
    /// # Arguments
    ///
    /// * `edit` - A `ScrobbleEdit` specifying what to find and how to change it
    ///
    /// # Returns
    ///
    /// Returns an `EditResponse` containing results for all edited scrobbles, including:
    /// - Overall success status
    /// - Individual results for each scrobble instance
    /// - Detailed error messages if any edits fail
    ///
    /// # Errors
    ///
    /// Returns `LastFmError::Parse` if no matching scrobbles are found, or other errors
    /// for network/authentication issues.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use lastfm_edit::{LastFmEditClient, ScrobbleEdit, Result};
    /// # async fn example(client: &dyn LastFmEditClient) -> Result<()> {
    /// // Change track name for all instances of a track
    /// let edit = ScrobbleEdit::from_track_and_artist("Old Track Name", "Artist")
    ///     .with_track_name("New Track Name");
    ///
    /// let response = client.edit_scrobble(&edit).await?;
    /// if response.success() {
    ///     println!("Successfully edited {} scrobbles", response.total_edits());
    /// }
    /// # Ok(())
    /// # }
    /// ```
    async fn edit_scrobble(&self, edit: &ScrobbleEdit) -> Result<EditResponse>;

    /// Edit a single scrobble with complete information and retry logic.
    ///
    /// This method performs a single edit operation on a fully-specified scrobble.
    /// Unlike [`edit_scrobble`], this method does not perform discovery, enrichment,
    /// or multiple edits - it edits exactly one scrobble instance.
    ///
    /// # Key Differences from `edit_scrobble`
    ///
    /// - **No discovery**: Requires a fully-specified `ExactScrobbleEdit`
    /// - **Single edit**: Only edits one scrobble instance
    /// - **No enrichment**: All fields must be provided upfront
    /// - **Retry logic**: Automatically retries on rate limiting
    ///
    /// # Arguments
    ///
    /// * `exact_edit` - A fully-specified edit with all required fields populated,
    ///   including original metadata and timestamps
    /// * `max_retries` - Maximum number of retry attempts for rate limiting.
    ///   The method will wait with exponential backoff between retries.
    ///
    /// # Returns
    ///
    /// Returns an `EditResponse` with a single result indicating success or failure.
    /// If max retries are exceeded due to rate limiting, returns a failed response
    /// rather than an error.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use lastfm_edit::{LastFmEditClient, ExactScrobbleEdit, Result};
    /// # async fn example(client: &dyn LastFmEditClient) -> Result<()> {
    /// let exact_edit = ExactScrobbleEdit::new(
    ///     "Original Track".to_string(),
    ///     "Original Album".to_string(),
    ///     "Artist".to_string(),
    ///     "Artist".to_string(),
    ///     "New Track Name".to_string(),
    ///     "Original Album".to_string(),
    ///     "Artist".to_string(),
    ///     "Artist".to_string(),
    ///     1640995200, // timestamp
    ///     false
    /// );
    ///
    /// let response = client.edit_scrobble_single(&exact_edit, 3).await?;
    /// # Ok(())
    /// # }
    /// ```
    async fn edit_scrobble_single(
        &self,
        exact_edit: &ExactScrobbleEdit,
        max_retries: u32,
    ) -> Result<EditResponse>;

    /// Delete a scrobble by its identifying information.
    ///
    /// This method deletes a specific scrobble from the user's library using the
    /// artist name, track name, and timestamp to uniquely identify it.
    ///
    /// # Arguments
    ///
    /// * `artist_name` - The artist name of the scrobble to delete
    /// * `track_name` - The track name of the scrobble to delete
    /// * `timestamp` - The unix timestamp of the scrobble to delete
    ///
    /// # Returns
    ///
    /// Returns `true` if the deletion was successful, `false` otherwise.
    async fn delete_scrobble(
        &self,
        artist_name: &str,
        track_name: &str,
        timestamp: u64,
    ) -> Result<bool>;

    /// Create an incremental discovery iterator for scrobble editing.
    ///
    /// This returns the appropriate discovery iterator based on what fields are specified
    /// in the ScrobbleEdit. The iterator yields `ExactScrobbleEdit` results incrementally,
    /// which helps avoid rate limiting issues when discovering many scrobbles.
    ///
    /// Returns a `Box<dyn AsyncDiscoveryIterator<ExactScrobbleEdit>>` to handle the different
    /// discovery strategies uniformly.
    fn discover_scrobbles(
        &self,
        edit: ScrobbleEdit,
    ) -> Box<dyn crate::AsyncDiscoveryIterator<crate::ExactScrobbleEdit>>;

    // =============================================================================
    // ITERATOR METHODS - Core library browsing functionality
    // =============================================================================

    /// Create an iterator for browsing an artist's tracks from the user's library.
    fn artist_tracks(&self, artist: &str) -> Box<dyn AsyncPaginatedIterator<Track>>;

    /// Create an iterator for browsing an artist's albums from the user's library.
    fn artist_albums(&self, artist: &str) -> Box<dyn AsyncPaginatedIterator<Album>>;

    /// Create an iterator for browsing tracks from a specific album.
    fn album_tracks(
        &self,
        album_name: &str,
        artist_name: &str,
    ) -> Box<dyn AsyncPaginatedIterator<Track>>;

    /// Create an iterator for browsing the user's recent tracks/scrobbles.
    fn recent_tracks(&self) -> Box<dyn AsyncPaginatedIterator<Track>>;

    /// Create an iterator for browsing the user's recent tracks starting from a specific page.
    fn recent_tracks_from_page(&self, starting_page: u32)
        -> Box<dyn AsyncPaginatedIterator<Track>>;

    /// Create an iterator for searching tracks in the user's library.
    ///
    /// This returns an iterator that uses Last.fm's library search functionality
    /// to find tracks matching the provided query string. The iterator handles
    /// pagination automatically.
    ///
    /// # Arguments
    ///
    /// * `query` - The search query (e.g., "remaster", "live", artist name, etc.)
    ///
    /// # Returns
    ///
    /// Returns a `SearchTracksIterator` for streaming search results.
    fn search_tracks(&self, query: &str) -> Box<dyn AsyncPaginatedIterator<Track>>;

    /// Create an iterator for searching albums in the user's library.
    ///
    /// This returns an iterator that uses Last.fm's library search functionality
    /// to find albums matching the provided query string. The iterator handles
    /// pagination automatically.
    ///
    /// # Arguments
    ///
    /// * `query` - The search query (e.g., "remaster", "deluxe", artist name, etc.)
    ///
    /// # Returns
    ///
    /// Returns a `SearchAlbumsIterator` for streaming search results.
    fn search_albums(&self, query: &str) -> Box<dyn AsyncPaginatedIterator<Album>>;

    // =============================================================================
    // SEARCH METHODS - Library search functionality
    // =============================================================================

    /// Get a single page of track search results from the user's library.
    ///
    /// This performs a search using Last.fm's library search functionality,
    /// returning one page of tracks that match the provided query string.
    /// For iterator-based access, use [`search_tracks`](Self::search_tracks) instead.
    ///
    /// # Arguments
    ///
    /// * `query` - The search query (e.g., "remaster", "live", artist name, etc.)
    /// * `page` - The page number to retrieve (1-based)
    ///
    /// # Returns
    ///
    /// Returns a `TrackPage` containing the search results with pagination information.
    async fn search_tracks_page(&self, query: &str, page: u32) -> Result<crate::TrackPage>;

    /// Get a single page of album search results from the user's library.
    ///
    /// This performs a search using Last.fm's library search functionality,
    /// returning one page of albums that match the provided query string.
    /// For iterator-based access, use [`search_albums`](Self::search_albums) instead.
    ///
    /// # Arguments
    ///
    /// * `query` - The search query (e.g., "remaster", "deluxe", artist name, etc.)
    /// * `page` - The page number to retrieve (1-based)
    ///
    /// # Returns
    ///
    /// Returns an `AlbumPage` containing the search results with pagination information.
    async fn search_albums_page(&self, query: &str, page: u32) -> Result<crate::AlbumPage>;

    // =============================================================================
    // CORE DATA METHODS - Essential data access
    // =============================================================================

    /// Get the currently authenticated username.
    fn username(&self) -> String;

    /// Fetch recent scrobbles from the user's listening history.
    async fn get_recent_scrobbles(&self, page: u32) -> Result<Vec<Track>>;

    /// Find the most recent scrobble for a specific track.
    async fn find_recent_scrobble_for_track(
        &self,
        track_name: &str,
        artist_name: &str,
        max_pages: u32,
    ) -> Result<Option<Track>>;

    /// Get a page of tracks from the user's library for the specified artist.
    async fn get_artist_tracks_page(&self, artist: &str, page: u32) -> Result<crate::TrackPage>;

    /// Get a page of albums from the user's library for the specified artist.
    async fn get_artist_albums_page(&self, artist: &str, page: u32) -> Result<crate::AlbumPage>;

    /// Get a page of tracks from a specific album in the user's library.
    async fn get_album_tracks_page(
        &self,
        album_name: &str,
        artist_name: &str,
        page: u32,
    ) -> Result<crate::TrackPage>;

    /// Get a page of tracks from the user's recent listening history.
    async fn get_recent_tracks_page(&self, page: u32) -> Result<crate::TrackPage> {
        let tracks = self.get_recent_scrobbles(page).await?;
        let has_next_page = !tracks.is_empty();
        Ok(crate::TrackPage {
            tracks,
            page_number: page,
            has_next_page,
            total_pages: None,
        })
    }

    // =============================================================================
    // CONVENIENCE METHODS - Higher-level helpers and shortcuts
    // =============================================================================

    /// Discover all scrobble edit variations based on the provided ScrobbleEdit template.
    ///
    /// This method analyzes what fields are specified in the input ScrobbleEdit and discovers
    /// all relevant scrobble instances that match the criteria:
    /// - If track_name_original is specified: discovers all album variations of that track
    /// - If only album_name_original is specified: discovers all tracks in that album
    /// - If neither is specified: discovers all tracks by that artist
    ///
    /// Returns fully-specified ExactScrobbleEdit instances with all metadata populated
    /// from the user's library, ready for editing operations.
    async fn discover_scrobble_edit_variations(
        &self,
        edit: &ScrobbleEdit,
    ) -> Result<Vec<ExactScrobbleEdit>> {
        // Use the incremental iterator and collect all results
        let mut discovery_iterator = self.discover_scrobbles(edit.clone());
        discovery_iterator.collect_all().await
    }

    /// Get tracks from a specific album page.
    async fn get_album_tracks(&self, album_name: &str, artist_name: &str) -> Result<Vec<Track>> {
        let mut tracks_iterator = self.album_tracks(album_name, artist_name);
        tracks_iterator.collect_all().await
    }

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

    /// Edit album metadata by updating scrobbles with new album name.
    async fn edit_album(
        &self,
        old_album_name: &str,
        new_album_name: &str,
        artist_name: &str,
    ) -> Result<EditResponse> {
        log::debug!("Editing album '{old_album_name}' -> '{new_album_name}' by '{artist_name}'");

        let edit = ScrobbleEdit::for_album(old_album_name, artist_name, artist_name)
            .with_album_name(new_album_name);

        self.edit_scrobble(&edit).await
    }

    /// Edit artist metadata by updating scrobbles with new artist name.
    ///
    /// This edits ALL tracks from the artist that are found in recent scrobbles.
    async fn edit_artist(
        &self,
        old_artist_name: &str,
        new_artist_name: &str,
    ) -> Result<EditResponse> {
        log::debug!("Editing artist '{old_artist_name}' -> '{new_artist_name}'");

        let edit = ScrobbleEdit::for_artist(old_artist_name, new_artist_name);

        self.edit_scrobble(&edit).await
    }

    /// Edit artist metadata for a specific track only.
    ///
    /// This edits only the specified track if found in recent scrobbles.
    async fn edit_artist_for_track(
        &self,
        track_name: &str,
        old_artist_name: &str,
        new_artist_name: &str,
    ) -> Result<EditResponse> {
        log::debug!("Editing artist for track '{track_name}' from '{old_artist_name}' -> '{new_artist_name}'");

        let edit = ScrobbleEdit::from_track_and_artist(track_name, old_artist_name)
            .with_artist_name(new_artist_name);

        self.edit_scrobble(&edit).await
    }

    /// Edit artist metadata for all tracks in a specific album.
    ///
    /// This edits ALL tracks from the specified album that are found in recent scrobbles.
    async fn edit_artist_for_album(
        &self,
        album_name: &str,
        old_artist_name: &str,
        new_artist_name: &str,
    ) -> Result<EditResponse> {
        log::debug!("Editing artist for album '{album_name}' from '{old_artist_name}' -> '{new_artist_name}'");

        let edit = ScrobbleEdit::for_album(album_name, old_artist_name, old_artist_name)
            .with_artist_name(new_artist_name);

        self.edit_scrobble(&edit).await
    }

    // =============================================================================
    // SESSION & EVENT MANAGEMENT - Authentication and monitoring
    // =============================================================================

    /// Extract the current session state for persistence.
    ///
    /// This allows you to save the authentication state and restore it later
    /// without requiring the user to log in again.
    ///
    /// # Returns
    ///
    /// Returns a [`LastFmEditSession`] that can be serialized and saved.
    fn get_session(&self) -> LastFmEditSession;

    /// Restore session state from a previously saved session.
    ///
    /// This allows you to restore authentication state without logging in again.
    ///
    /// # Arguments
    ///
    /// * `session` - Previously saved session state
    fn restore_session(&self, session: LastFmEditSession);

    /// Subscribe to internal client events.
    ///
    /// Returns a broadcast receiver that can be used to listen to events like rate limiting.
    /// Multiple subscribers can listen simultaneously.
    ///
    /// # Example
    /// ```rust,no_run
    /// use lastfm_edit::{LastFmEditClientImpl, LastFmEditSession, ClientEvent};
    ///
    /// let http_client = http_client::native::NativeClient::new();
    /// let test_session = LastFmEditSession::new("test".to_string(), vec!["sessionid=.test123".to_string()], Some("csrf".to_string()), "https://www.last.fm".to_string());
    /// let client = LastFmEditClientImpl::from_session(Box::new(http_client), test_session);
    /// let mut events = client.subscribe();
    ///
    /// // Listen for events in a background task
    /// tokio::spawn(async move {
    ///     while let Ok(event) = events.recv().await {
    ///         match event {
    ///             ClientEvent::RequestStarted { request } => {
    ///                 println!("Request started: {}", request.short_description());
    ///             }
    ///             ClientEvent::RequestCompleted { request, status_code, duration_ms } => {
    ///                 println!("Request completed: {} - {} ({} ms)", request.short_description(), status_code, duration_ms);
    ///             }
    ///             ClientEvent::RateLimited { delay_seconds, .. } => {
    ///                 println!("Rate limited! Waiting {} seconds", delay_seconds);
    ///             }
    ///             ClientEvent::EditAttempted { edit, success, .. } => {
    ///                 println!("Edit attempt: '{}' -> '{}' - {}",
    ///                          edit.track_name_original, edit.track_name,
    ///                          if success { "Success" } else { "Failed" });
    ///             }
    ///         }
    ///     }
    /// });
    /// ```
    fn subscribe(&self) -> ClientEventReceiver;

    /// Get the latest client event without subscribing to future events.
    ///
    /// This returns the most recent event that occurred, or `None` if no events have occurred yet.
    /// Unlike `subscribe()`, this provides instant access to the current state without waiting.
    ///
    /// # Example
    /// ```rust,no_run
    /// use lastfm_edit::{LastFmEditClientImpl, LastFmEditSession, ClientEvent};
    ///
    /// let http_client = http_client::native::NativeClient::new();
    /// let test_session = LastFmEditSession::new("test".to_string(), vec!["sessionid=.test123".to_string()], Some("csrf".to_string()), "https://www.last.fm".to_string());
    /// let client = LastFmEditClientImpl::from_session(Box::new(http_client), test_session);
    ///
    /// if let Some(ClientEvent::RateLimited { delay_seconds, .. }) = client.latest_event() {
    ///     println!("Currently rate limited for {} seconds", delay_seconds);
    /// }
    /// ```
    fn latest_event(&self) -> Option<ClientEvent>;

    /// Validate if the current session is still working.
    ///
    /// This method makes a test request to a protected Last.fm settings page to verify
    /// that the current session is still valid. If the session has expired or become
    /// invalid, Last.fm will redirect to the login page.
    ///
    /// This is useful for checking session validity before attempting operations that
    /// require authentication, especially after loading a previously saved session.
    ///
    /// # Returns
    ///
    /// Returns `true` if the session is valid and can be used for authenticated operations,
    /// `false` if the session is invalid or expired.
    async fn validate_session(&self) -> bool;
}
