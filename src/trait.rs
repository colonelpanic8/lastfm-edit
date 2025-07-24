use crate::edit::ExactScrobbleEdit;
use crate::session::LastFmEditSession;
use crate::{AlbumPage, EditResponse, LastFmError, Result, ScrobbleEdit, Track, TrackPage};
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
    ) -> Result<Vec<ExactScrobbleEdit>>;

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

    /// Get a page of tracks from the user's library for the specified artist.
    async fn get_artist_tracks_page(&self, artist: &str, page: u32) -> Result<TrackPage>;

    /// Get a page of albums from the user's library for the specified artist.
    async fn get_artist_albums_page(&self, artist: &str, page: u32) -> Result<AlbumPage>;

    /// Get a page of tracks from the user's recent listening history.
    async fn get_recent_tracks_page(&self, page: u32) -> Result<TrackPage>;

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
