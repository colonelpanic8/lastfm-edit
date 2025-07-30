//! Data types for Last.fm music metadata and operations.
//!
//! This module contains all the core data structures used throughout the crate,
//! including track and album metadata, edit operations, error types, session state,
//! configuration, and event handling.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;
use tokio::sync::{broadcast, watch};

// ================================================================================================
// TRACK AND ALBUM METADATA
// ================================================================================================

/// Represents a music track with associated metadata.
///
/// This structure contains track information as parsed from Last.fm pages,
/// including play count and optional timestamp data for scrobbles.
///
/// # Examples
///
/// ```rust
/// use lastfm_edit::Track;
///
/// let track = Track {
///     name: "Paranoid Android".to_string(),
///     artist: "Radiohead".to_string(),
///     playcount: 42,
///     timestamp: Some(1640995200), // Unix timestamp
///     album: Some("OK Computer".to_string()),
///     album_artist: Some("Radiohead".to_string()),
/// };
///
/// println!("{} by {} (played {} times)", track.name, track.artist, track.playcount);
/// if let Some(album) = &track.album {
///     println!("From album: {}", album);
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Track {
    /// The track name/title
    pub name: String,
    /// The artist name
    pub artist: String,
    /// Number of times this track has been played/scrobbled
    pub playcount: u32,
    /// Unix timestamp of when this track was scrobbled (if available)
    ///
    /// This field is populated when tracks are retrieved from recent scrobbles
    /// or individual scrobble data, but may be `None` for aggregate track listings.
    pub timestamp: Option<u64>,
    /// The album name (if available)
    ///
    /// This field is populated when tracks are retrieved from recent scrobbles
    /// where album information is available in the edit forms. May be `None`
    /// for aggregate track listings or when album information is not available.
    pub album: Option<String>,
    /// The album artist name (if available and different from track artist)
    ///
    /// This field is populated when tracks are retrieved from recent scrobbles
    /// where album artist information is available. May be `None` for tracks
    /// where the album artist is the same as the track artist, or when this
    /// information is not available.
    pub album_artist: Option<String>,
}

/// Represents a paginated collection of tracks.
///
/// This structure is returned by track listing methods and provides
/// information about the current page and pagination state.
///
/// # Examples
///
/// ```rust
/// use lastfm_edit::{Track, TrackPage};
///
/// let page = TrackPage {
///     tracks: vec![
///         Track {
///             name: "Song 1".to_string(),
///             artist: "Artist".to_string(),
///             playcount: 10,
///             timestamp: None,
///             album: None,
///             album_artist: None,
///         }
///     ],
///     page_number: 1,
///     has_next_page: true,
///     total_pages: Some(5),
/// };
///
/// println!("Page {} of {:?}, {} tracks",
///          page.page_number,
///          page.total_pages,
///          page.tracks.len());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TrackPage {
    /// The tracks on this page
    pub tracks: Vec<Track>,
    /// Current page number (1-indexed)
    pub page_number: u32,
    /// Whether there are more pages available
    pub has_next_page: bool,
    /// Total number of pages, if known
    ///
    /// This may be `None` if the total page count cannot be determined
    /// from the Last.fm response.
    pub total_pages: Option<u32>,
}

/// Represents a music album with associated metadata.
///
/// This structure contains album information as parsed from Last.fm pages,
/// including play count and optional timestamp data for scrobbles.
///
/// # Examples
///
/// ```rust
/// use lastfm_edit::Album;
///
/// let album = Album {
///     name: "OK Computer".to_string(),
///     artist: "Radiohead".to_string(),
///     playcount: 156,
///     timestamp: Some(1640995200), // Unix timestamp
/// };
///
/// println!("{} by {} (played {} times)", album.name, album.artist, album.playcount);
///
/// // Convert timestamp to human-readable date
/// if let Some(date) = album.scrobbled_at() {
///     println!("Last scrobbled: {}", date.format("%Y-%m-%d %H:%M:%S UTC"));
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Album {
    /// The album name/title
    pub name: String,
    /// The artist name
    pub artist: String,
    /// Number of times this album has been played/scrobbled
    pub playcount: u32,
    /// Unix timestamp of when this album was last scrobbled (if available)
    ///
    /// This field is populated when albums are retrieved from recent scrobbles
    /// or individual scrobble data, but may be `None` for aggregate album listings.
    pub timestamp: Option<u64>,
}

/// Represents a paginated collection of albums.
///
/// This structure is returned by album listing methods and provides
/// information about the current page and pagination state.
///
/// # Examples
///
/// ```rust
/// use lastfm_edit::{Album, AlbumPage};
///
/// let page = AlbumPage {
///     albums: vec![
///         Album {
///             name: "Album 1".to_string(),
///             artist: "Artist".to_string(),
///             playcount: 25,
///             timestamp: None,
///         }
///     ],
///     page_number: 1,
///     has_next_page: false,
///     total_pages: Some(1),
/// };
///
/// println!("Page {} of {:?}, {} albums",
///          page.page_number,
///          page.total_pages,
///          page.albums.len());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AlbumPage {
    /// The albums on this page
    pub albums: Vec<Album>,
    /// Current page number (1-indexed)
    pub page_number: u32,
    /// Whether there are more pages available
    pub has_next_page: bool,
    /// Total number of pages, if known
    ///
    /// This may be `None` if the total page count cannot be determined
    /// from the Last.fm response.
    pub total_pages: Option<u32>,
}

impl Album {
    /// Convert the Unix timestamp to a human-readable datetime.
    ///
    /// Returns `None` if no timestamp is available or if the timestamp is invalid.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use lastfm_edit::Album;
    ///
    /// let album = Album {
    ///     name: "Abbey Road".to_string(),
    ///     artist: "The Beatles".to_string(),
    ///     playcount: 42,
    ///     timestamp: Some(1640995200),
    /// };
    ///
    /// if let Some(datetime) = album.scrobbled_at() {
    ///     println!("Last played: {}", datetime.format("%Y-%m-%d %H:%M:%S UTC"));
    /// }
    /// ```
    #[must_use]
    pub fn scrobbled_at(&self) -> Option<DateTime<Utc>> {
        self.timestamp
            .and_then(|ts| DateTime::from_timestamp(i64::try_from(ts).ok()?, 0))
    }
}

// ================================================================================================
// EDIT OPERATIONS
// ================================================================================================

/// Represents a scrobble edit operation.
///
/// This structure contains all the information needed to edit a specific scrobble
/// on Last.fm, including both the original and new metadata values.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct ScrobbleEdit {
    /// Original track name as it appears in the scrobble (optional - if None, edits all tracks)
    pub track_name_original: Option<String>,
    /// Original album name as it appears in the scrobble (optional)
    pub album_name_original: Option<String>,
    /// Original artist name as it appears in the scrobble (required)
    pub artist_name_original: String,
    /// Original album artist name as it appears in the scrobble (optional)
    pub album_artist_name_original: Option<String>,

    /// New track name to set (optional - if None, keeps original track names)
    pub track_name: Option<String>,
    /// New album name to set (optional - if None, keeps original album names)
    pub album_name: Option<String>,
    /// New artist name to set
    pub artist_name: String,
    /// New album artist name to set (optional - if None, keeps original album artist names)
    pub album_artist_name: Option<String>,

    /// Unix timestamp of the scrobble to edit (optional)
    ///
    /// This identifies the specific scrobble instance to modify.
    /// If None, the client will attempt to find a representative timestamp.
    pub timestamp: Option<u64>,
    /// Whether to edit all instances or just this specific scrobble
    ///
    /// When `true`, Last.fm will update all scrobbles with matching metadata.
    /// When `false`, only this specific scrobble (identified by timestamp) is updated.
    pub edit_all: bool,
}

/// Response from a single scrobble edit operation.
///
/// This structure contains the result of attempting to edit a specific scrobble instance,
/// including success status and any error messages.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SingleEditResponse {
    /// Whether this individual edit operation was successful
    pub success: bool,
    /// Optional message describing the result or any errors
    pub message: Option<String>,
    /// Information about which album variation was edited
    pub album_info: Option<String>,
    /// The exact scrobble edit that was performed
    pub exact_scrobble_edit: ExactScrobbleEdit,
}

/// Response from a scrobble edit operation that may affect multiple album variations.
///
/// When editing a track that appears on multiple albums, this response contains
/// the results of all individual edit operations performed.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EditResponse {
    /// Results of individual edit operations
    pub individual_results: Vec<SingleEditResponse>,
}

/// Internal representation of a scrobble edit with all fields fully specified.
///
/// This type is used internally by the client after enriching metadata from
/// Last.fm. Unlike `ScrobbleEdit`, all fields are required and non-optional,
/// ensuring we have complete information before performing edit operations.
///
/// This type represents a fully-specified scrobble edit where all fields are known.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct ExactScrobbleEdit {
    /// Original track name as it appears in the scrobble
    pub track_name_original: String,
    /// Original album name as it appears in the scrobble
    pub album_name_original: String,
    /// Original artist name as it appears in the scrobble
    pub artist_name_original: String,
    /// Original album artist name as it appears in the scrobble
    pub album_artist_name_original: String,

    /// New track name to set
    pub track_name: String,
    /// New album name to set
    pub album_name: String,
    /// New artist name to set
    pub artist_name: String,
    /// New album artist name to set
    pub album_artist_name: String,

    /// Unix timestamp of the scrobble to edit
    pub timestamp: u64,
    /// Whether to edit all instances or just this specific scrobble
    pub edit_all: bool,
}

impl ScrobbleEdit {
    /// Create a new [`ScrobbleEdit`] with all required fields.
    ///
    /// This is the most general constructor that allows setting all fields.
    /// For convenience, consider using [`from_track_info`](Self::from_track_info) instead.
    ///
    /// # Arguments
    ///
    /// * `track_name_original` - The current track name in the scrobble
    /// * `album_name_original` - The current album name in the scrobble
    /// * `artist_name_original` - The current artist name in the scrobble
    /// * `album_artist_name_original` - The current album artist name in the scrobble
    /// * `track_name` - The new track name to set
    /// * `album_name` - The new album name to set
    /// * `artist_name` - The new artist name to set
    /// * `album_artist_name` - The new album artist name to set
    /// * `timestamp` - Unix timestamp identifying the scrobble
    /// * `edit_all` - Whether to edit all matching scrobbles or just this one
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        track_name_original: Option<String>,
        album_name_original: Option<String>,
        artist_name_original: String,
        album_artist_name_original: Option<String>,
        track_name: Option<String>,
        album_name: Option<String>,
        artist_name: String,
        album_artist_name: Option<String>,
        timestamp: Option<u64>,
        edit_all: bool,
    ) -> Self {
        Self {
            track_name_original,
            album_name_original,
            artist_name_original,
            album_artist_name_original,
            track_name,
            album_name,
            artist_name,
            album_artist_name,
            timestamp,
            edit_all,
        }
    }

    /// Create an edit request from track information (convenience constructor).
    ///
    /// This constructor creates a [`ScrobbleEdit`] with the new values initially
    /// set to the same as the original values. Use the builder methods like
    /// [`with_track_name`](Self::with_track_name) to specify what should be changed.
    ///
    /// # Arguments
    ///
    /// * `original_track` - The current track name
    /// * `original_album` - The current album name
    /// * `original_artist` - The current artist name
    /// * `timestamp` - Unix timestamp identifying the scrobble
    pub fn from_track_info(
        original_track: &str,
        original_album: &str,
        original_artist: &str,
        timestamp: u64,
    ) -> Self {
        Self::new(
            Some(original_track.to_string()),
            Some(original_album.to_string()),
            original_artist.to_string(),
            Some(original_artist.to_string()), // album_artist defaults to artist
            Some(original_track.to_string()),
            Some(original_album.to_string()),
            original_artist.to_string(),
            Some(original_artist.to_string()), // album_artist defaults to artist
            Some(timestamp),
            false, // edit_all defaults to false
        )
    }

    /// Set the new track name.
    pub fn with_track_name(mut self, track_name: &str) -> Self {
        self.track_name = Some(track_name.to_string());
        self
    }

    /// Set the new album name.
    pub fn with_album_name(mut self, album_name: &str) -> Self {
        self.album_name = Some(album_name.to_string());
        self
    }

    /// Set the new artist name.
    ///
    /// This also sets the album artist name to the same value.
    pub fn with_artist_name(mut self, artist_name: &str) -> Self {
        self.artist_name = artist_name.to_string();
        self.album_artist_name = Some(artist_name.to_string());
        self
    }

    /// Set whether to edit all instances of this track.
    ///
    /// When `true`, Last.fm will update all scrobbles with the same metadata.
    /// When `false` (default), only the specific scrobble is updated.
    pub fn with_edit_all(mut self, edit_all: bool) -> Self {
        self.edit_all = edit_all;
        self
    }

    /// Create an edit request with minimal information, letting the client look up missing metadata.
    ///
    /// This constructor is useful when you only know some of the original metadata and want
    /// the client to automatically fill in missing information by looking up the scrobble.
    ///
    /// # Arguments
    ///
    /// * `track_name` - The new track name to set
    /// * `artist_name` - The new artist name to set
    /// * `album_name` - The new album name to set
    /// * `timestamp` - Unix timestamp identifying the scrobble
    pub fn with_minimal_info(
        track_name: &str,
        artist_name: &str,
        album_name: &str,
        timestamp: u64,
    ) -> Self {
        Self::new(
            Some(track_name.to_string()),
            Some(album_name.to_string()),
            artist_name.to_string(),
            Some(artist_name.to_string()),
            Some(track_name.to_string()),
            Some(album_name.to_string()),
            artist_name.to_string(),
            Some(artist_name.to_string()),
            Some(timestamp),
            false,
        )
    }
    /// Create an edit request with just track and artist information.
    ///
    /// This constructor is useful when you only know the track and artist names.
    /// The client will use these as both original and new values, and will
    /// attempt to find a representative timestamp and album information.
    ///
    /// # Arguments
    ///
    /// * `track_name` - The track name (used as both original and new)
    /// * `artist_name` - The artist name (used as both original and new)
    pub fn from_track_and_artist(track_name: &str, artist_name: &str) -> Self {
        Self::new(
            Some(track_name.to_string()),
            None, // Client will look up original album name
            artist_name.to_string(),
            None, // Client will look up original album artist name
            Some(track_name.to_string()),
            None, // Will be filled by client or kept as original
            artist_name.to_string(),
            Some(artist_name.to_string()), // album_artist defaults to artist
            None,                          // Client will find representative timestamp
            false,
        )
    }

    /// Create an edit request for all tracks by an artist.
    ///
    /// This constructor creates a [`ScrobbleEdit`] that will edit all tracks
    /// by the specified artist, changing the artist name to the new value.
    ///
    /// # Arguments
    ///
    /// * `old_artist_name` - The current artist name to change from
    /// * `new_artist_name` - The new artist name to change to
    pub fn for_artist(old_artist_name: &str, new_artist_name: &str) -> Self {
        Self::new(
            None, // No specific track - edit all tracks
            None, // No specific album - edit all albums
            old_artist_name.to_string(),
            None, // Client will look up original album artist name
            None, // No track name change - keep original track names
            None, // Keep original album names (they can vary)
            new_artist_name.to_string(),
            Some(new_artist_name.to_string()), // album_artist also changes for global renames
            None,                              // Client will find representative timestamp
            true,                              // Edit all instances by default for artist changes
        )
    }

    /// Create an edit request for all tracks in a specific album.
    ///
    /// This constructor creates a [`ScrobbleEdit`] that will edit all tracks
    /// in the specified album by the specified artist.
    ///
    /// # Arguments
    ///
    /// * `album_name` - The album name containing tracks to edit
    /// * `artist_name` - The artist name for the album
    /// * `new_artist_name` - The new artist name to change to
    pub fn for_album(album_name: &str, old_artist_name: &str, new_artist_name: &str) -> Self {
        Self::new(
            None, // No specific track - edit all tracks in album
            Some(album_name.to_string()),
            old_artist_name.to_string(),
            Some(old_artist_name.to_string()),
            None,                         // No track name change - keep original track names
            Some(album_name.to_string()), // Keep same album name
            new_artist_name.to_string(),
            None, // Keep original album_artist names (they can vary)
            None, // Client will find representative timestamp
            true, // Edit all instances by default for album changes
        )
    }
}

impl ExactScrobbleEdit {
    /// Create a new [`ExactScrobbleEdit`] with all fields specified.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        track_name_original: String,
        album_name_original: String,
        artist_name_original: String,
        album_artist_name_original: String,
        track_name: String,
        album_name: String,
        artist_name: String,
        album_artist_name: String,
        timestamp: u64,
        edit_all: bool,
    ) -> Self {
        Self {
            track_name_original,
            album_name_original,
            artist_name_original,
            album_artist_name_original,
            track_name,
            album_name,
            artist_name,
            album_artist_name,
            timestamp,
            edit_all,
        }
    }

    /// Build the form data for submitting this scrobble edit.
    ///
    /// This creates a HashMap containing all the form fields needed to submit
    /// the edit request to Last.fm, including the CSRF token and all metadata fields.
    pub fn build_form_data(&self, csrf_token: &str) -> HashMap<&str, String> {
        let mut form_data = HashMap::new();

        // Add fresh CSRF token (required)
        form_data.insert("csrfmiddlewaretoken", csrf_token.to_string());

        // Include ALL form fields (using ExactScrobbleEdit which has all required fields)
        form_data.insert("track_name_original", self.track_name_original.clone());
        form_data.insert("track_name", self.track_name.clone());
        form_data.insert("artist_name_original", self.artist_name_original.clone());
        form_data.insert("artist_name", self.artist_name.clone());
        form_data.insert("album_name_original", self.album_name_original.clone());
        form_data.insert("album_name", self.album_name.clone());
        form_data.insert(
            "album_artist_name_original",
            self.album_artist_name_original.clone(),
        );
        form_data.insert("album_artist_name", self.album_artist_name.clone());

        // Include timestamp (ExactScrobbleEdit always has a timestamp)
        form_data.insert("timestamp", self.timestamp.to_string());

        // Edit flags
        if self.edit_all {
            form_data.insert("edit_all", "1".to_string());
        }
        form_data.insert("submit", "edit-scrobble".to_string());
        form_data.insert("ajax", "1".to_string());

        form_data
    }

    /// Convert this exact edit back to a public ScrobbleEdit.
    ///
    /// This is useful when you need to expose the edit data through the public API.
    pub fn to_scrobble_edit(&self) -> ScrobbleEdit {
        ScrobbleEdit::new(
            Some(self.track_name_original.clone()),
            Some(self.album_name_original.clone()),
            self.artist_name_original.clone(),
            Some(self.album_artist_name_original.clone()),
            Some(self.track_name.clone()),
            Some(self.album_name.clone()),
            self.artist_name.clone(),
            Some(self.album_artist_name.clone()),
            Some(self.timestamp),
            self.edit_all,
        )
    }
}

impl EditResponse {
    /// Create a new EditResponse from a single result.
    pub fn single(
        success: bool,
        message: Option<String>,
        album_info: Option<String>,
        exact_scrobble_edit: ExactScrobbleEdit,
    ) -> Self {
        Self {
            individual_results: vec![SingleEditResponse {
                success,
                message,
                album_info,
                exact_scrobble_edit,
            }],
        }
    }

    /// Create a new EditResponse from multiple results.
    pub fn from_results(results: Vec<SingleEditResponse>) -> Self {
        Self {
            individual_results: results,
        }
    }

    /// Check if all individual edit operations were successful.
    pub fn all_successful(&self) -> bool {
        !self.individual_results.is_empty() && self.individual_results.iter().all(|r| r.success)
    }

    /// Check if any individual edit operations were successful.
    pub fn any_successful(&self) -> bool {
        self.individual_results.iter().any(|r| r.success)
    }

    /// Get the total number of edit operations performed.
    pub fn total_edits(&self) -> usize {
        self.individual_results.len()
    }

    /// Get the number of successful edit operations.
    pub fn successful_edits(&self) -> usize {
        self.individual_results.iter().filter(|r| r.success).count()
    }

    /// Get the number of failed edit operations.
    pub fn failed_edits(&self) -> usize {
        self.individual_results
            .iter()
            .filter(|r| !r.success)
            .count()
    }

    /// Generate a summary message describing the overall result.
    pub fn summary_message(&self) -> String {
        let total = self.total_edits();
        let successful = self.successful_edits();
        let failed = self.failed_edits();

        if total == 0 {
            return "No edit operations performed".to_string();
        }

        if successful == total {
            if total == 1 {
                "Edit completed successfully".to_string()
            } else {
                format!("All {total} edits completed successfully")
            }
        } else if successful == 0 {
            if total == 1 {
                "Edit failed".to_string()
            } else {
                format!("All {total} edits failed")
            }
        } else {
            format!("{successful} of {total} edits succeeded, {failed} failed")
        }
    }

    /// Get detailed messages from all edit operations.
    pub fn detailed_messages(&self) -> Vec<String> {
        self.individual_results
            .iter()
            .enumerate()
            .map(|(i, result)| {
                let album_info = result
                    .album_info
                    .as_deref()
                    .map(|info| format!(" ({info})"))
                    .unwrap_or_default();

                match &result.message {
                    Some(msg) => format!("{}: {}{}", i + 1, msg, album_info),
                    None => {
                        if result.success {
                            format!("{}: Success{}", i + 1, album_info)
                        } else {
                            format!("{}: Failed{}", i + 1, album_info)
                        }
                    }
                }
            })
            .collect()
    }

    /// Check if this response represents a single edit (for backward compatibility).
    pub fn is_single_edit(&self) -> bool {
        self.individual_results.len() == 1
    }

    /// Check if all edits succeeded (for backward compatibility).
    pub fn success(&self) -> bool {
        self.all_successful()
    }

    /// Get a single message for backward compatibility.
    /// Returns the summary message.
    pub fn message(&self) -> Option<String> {
        Some(self.summary_message())
    }
}

// ================================================================================================
// ERROR TYPES
// ================================================================================================

/// Error types for Last.fm operations.
///
/// This enum covers all possible errors that can occur when interacting with Last.fm,
/// including network issues, authentication failures, parsing errors, and rate limiting.
///
/// # Error Handling Examples
///
/// ```rust,no_run
/// use lastfm_edit::{LastFmEditClient, LastFmEditClientImpl, LastFmError};
///
/// #[tokio::main]
/// async fn main() {
///     let http_client = http_client::native::NativeClient::new();
///
///     match LastFmEditClientImpl::login_with_credentials(Box::new(http_client), "username", "password").await {
///         Ok(client) => println!("Login successful"),
///         Err(LastFmError::Auth(msg)) => eprintln!("Authentication failed: {}", msg),
///         Err(LastFmError::RateLimit { retry_after }) => {
///             eprintln!("Rate limited, retry in {} seconds", retry_after);
///         }
///         Err(LastFmError::Http(msg)) => eprintln!("Network error: {}", msg),
///         Err(e) => eprintln!("Other error: {}", e),
///     }
/// }
/// ```
///
/// # Automatic Retry
///
/// Some operations like [`LastFmEditClient::edit_scrobble_single`](crate::LastFmEditClient::edit_scrobble_single)
/// automatically handle rate limiting errors by waiting and retrying:
///
/// ```rust,no_run
/// # use lastfm_edit::{LastFmEditClient, LastFmEditClientImpl, LastFmEditSession, ScrobbleEdit};
/// # tokio_test::block_on(async {
/// # let test_session = LastFmEditSession::new("test".to_string(), vec!["sessionid=.test123".to_string()], Some("csrf".to_string()), "https://www.last.fm".to_string());
/// let mut client = LastFmEditClientImpl::from_session(Box::new(http_client::native::NativeClient::new()), test_session);
///
/// let edit = ScrobbleEdit::from_track_info("Track", "Album", "Artist", 1640995200);
///
/// // Standard edit operation
/// match client.edit_scrobble(&edit).await {
///     Ok(response) => println!("Edit completed: {:?}", response),
///     Err(e) => eprintln!("Edit failed: {}", e),
/// }
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// # });
/// ```
#[derive(Error, Debug)]
pub enum LastFmError {
    /// HTTP/network related errors.
    ///
    /// This includes connection failures, timeouts, DNS errors, and other
    /// low-level networking issues.
    #[error("HTTP error: {0}")]
    Http(String),

    /// Authentication failures.
    ///
    /// This occurs when login credentials are invalid, sessions expire,
    /// or authentication is required but not provided.
    ///
    /// # Common Causes
    /// - Invalid username/password
    /// - Expired session cookies
    /// - Account locked or suspended
    /// - Two-factor authentication required
    #[error("Authentication failed: {0}")]
    Auth(String),

    /// CSRF token not found in response.
    ///
    /// This typically indicates that Last.fm's page structure has changed
    /// or that the request was blocked.
    #[error("CSRF token not found")]
    CsrfNotFound,

    /// Failed to parse Last.fm's response.
    ///
    /// This can happen when Last.fm changes their HTML structure or
    /// returns unexpected data formats.
    #[error("Failed to parse response: {0}")]
    Parse(String),

    /// Rate limiting from Last.fm.
    ///
    /// Last.fm has rate limits to prevent abuse. When hit, the client
    /// should wait before making more requests.
    ///
    /// The `retry_after` field indicates how many seconds to wait before
    /// the next request attempt.
    #[error("Rate limited, retry after {retry_after} seconds")]
    RateLimit {
        /// Number of seconds to wait before retrying
        retry_after: u64,
    },

    /// Scrobble edit operation failed.
    ///
    /// This is returned when an edit request is properly formatted and sent,
    /// but Last.fm rejects it for business logic reasons.
    #[error("Edit failed: {0}")]
    EditFailed(String),

    /// File system I/O errors.
    ///
    /// This can occur when saving debug responses or other file operations.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

// ================================================================================================
// SESSION MANAGEMENT
// ================================================================================================

/// Serializable client session state that can be persisted and restored.
///
/// This contains all the authentication state needed to resume a Last.fm session
/// without requiring the user to log in again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastFmEditSession {
    /// The authenticated username
    pub username: String,
    /// Session cookies required for authenticated requests
    pub cookies: Vec<String>,
    /// CSRF token for form submissions
    pub csrf_token: Option<String>,
    /// Base URL for the Last.fm instance
    pub base_url: String,
}

impl LastFmEditSession {
    /// Create a new client session with the provided state
    pub fn new(
        username: String,
        session_cookies: Vec<String>,
        csrf_token: Option<String>,
        base_url: String,
    ) -> Self {
        Self {
            username,
            cookies: session_cookies,
            csrf_token,
            base_url,
        }
    }

    /// Check if this session appears to be valid
    ///
    /// This performs basic validation but doesn't guarantee the session
    /// is still active on the server.
    pub fn is_valid(&self) -> bool {
        !self.username.is_empty()
            && !self.cookies.is_empty()
            && self.csrf_token.is_some()
            && self
                .cookies
                .iter()
                .any(|cookie| cookie.starts_with("sessionid=") && cookie.len() > 50)
    }

    /// Serialize session to JSON string
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Deserialize session from JSON string
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }
}

// ================================================================================================
// CLIENT CONFIGURATION
// ================================================================================================

/// Configuration for rate limit detection behavior
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateLimitConfig {
    /// Whether to detect rate limits by HTTP status codes (429, 403)
    pub detect_by_status: bool,
    /// Whether to detect rate limits by response body patterns
    pub detect_by_patterns: bool,
    /// Patterns to look for in response bodies (used when detect_by_patterns is true)
    pub patterns: Vec<String>,
    /// Additional custom patterns to look for in response bodies
    pub custom_patterns: Vec<String>,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            detect_by_status: true,
            detect_by_patterns: true,
            patterns: vec![
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
            custom_patterns: vec![],
        }
    }
}

impl RateLimitConfig {
    /// Create config with all detection disabled
    pub fn disabled() -> Self {
        Self {
            detect_by_status: false,
            detect_by_patterns: false,
            patterns: vec![],
            custom_patterns: vec![],
        }
    }

    /// Create config with only status code detection
    pub fn status_only() -> Self {
        Self {
            detect_by_status: true,
            detect_by_patterns: false,
            patterns: vec![],
            custom_patterns: vec![],
        }
    }

    /// Create config with only default pattern detection
    pub fn patterns_only() -> Self {
        Self {
            detect_by_status: false,
            detect_by_patterns: true,
            ..Default::default()
        }
    }

    /// Create config with custom patterns only (no default patterns)
    pub fn custom_patterns_only(patterns: Vec<String>) -> Self {
        Self {
            detect_by_status: false,
            detect_by_patterns: false,
            patterns: vec![],
            custom_patterns: patterns,
        }
    }

    /// Create config with both default and custom patterns
    pub fn with_custom_patterns(mut self, patterns: Vec<String>) -> Self {
        self.custom_patterns = patterns;
        self
    }

    /// Create config with custom patterns (replaces built-in patterns)
    pub fn with_patterns(mut self, patterns: Vec<String>) -> Self {
        self.patterns = patterns;
        self
    }
}

/// Unified configuration for retry behavior and rate limiting
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ClientConfig {
    /// Retry configuration
    pub retry: RetryConfig,
    /// Rate limit detection configuration
    pub rate_limit: RateLimitConfig,
}

impl ClientConfig {
    /// Create a new config with default settings
    pub fn new() -> Self {
        Self::default()
    }

    /// Create config with retries disabled
    pub fn with_retries_disabled() -> Self {
        Self {
            retry: RetryConfig::disabled(),
            rate_limit: RateLimitConfig::default(),
        }
    }

    /// Create config with rate limit detection disabled
    pub fn with_rate_limiting_disabled() -> Self {
        Self {
            retry: RetryConfig::default(),
            rate_limit: RateLimitConfig::disabled(),
        }
    }

    /// Create config with both retries and rate limiting disabled
    pub fn minimal() -> Self {
        Self {
            retry: RetryConfig::disabled(),
            rate_limit: RateLimitConfig::disabled(),
        }
    }

    /// Set custom retry configuration
    pub fn with_retry_config(mut self, retry_config: RetryConfig) -> Self {
        self.retry = retry_config;
        self
    }

    /// Set custom rate limit configuration
    pub fn with_rate_limit_config(mut self, rate_limit_config: RateLimitConfig) -> Self {
        self.rate_limit = rate_limit_config;
        self
    }

    /// Set custom retry count
    pub fn with_max_retries(mut self, max_retries: u32) -> Self {
        self.retry.max_retries = max_retries;
        self.retry.enabled = max_retries > 0;
        self
    }

    /// Set custom retry delays
    pub fn with_retry_delays(mut self, base_delay: u64, max_delay: u64) -> Self {
        self.retry.base_delay = base_delay;
        self.retry.max_delay = max_delay;
        self
    }

    /// Add custom rate limit patterns
    pub fn with_custom_rate_limit_patterns(mut self, patterns: Vec<String>) -> Self {
        self.rate_limit.custom_patterns = patterns;
        self
    }

    /// Enable/disable HTTP status code rate limit detection
    pub fn with_status_detection(mut self, enabled: bool) -> Self {
        self.rate_limit.detect_by_status = enabled;
        self
    }

    /// Enable/disable response pattern rate limit detection
    pub fn with_pattern_detection(mut self, enabled: bool) -> Self {
        self.rate_limit.detect_by_patterns = enabled;
        self
    }
}

/// Configuration for retry behavior
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryConfig {
    /// Maximum number of retry attempts (set to 0 to disable retries)
    pub max_retries: u32,
    /// Base delay for exponential backoff (in seconds)
    pub base_delay: u64,
    /// Maximum delay cap (in seconds)
    pub max_delay: u64,
    /// Whether retries are enabled at all
    pub enabled: bool,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay: 5,
            max_delay: 300, // 5 minutes
            enabled: true,
        }
    }
}

impl RetryConfig {
    /// Create a config with retries disabled
    pub fn disabled() -> Self {
        Self {
            max_retries: 0,
            base_delay: 5,
            max_delay: 300,
            enabled: false,
        }
    }

    /// Create a config with custom retry count
    pub fn with_retries(max_retries: u32) -> Self {
        Self {
            max_retries,
            enabled: max_retries > 0,
            ..Default::default()
        }
    }

    /// Create a config with custom delays
    pub fn with_delays(base_delay: u64, max_delay: u64) -> Self {
        Self {
            base_delay,
            max_delay,
            ..Default::default()
        }
    }
}

/// Result of a retry operation with context
#[derive(Debug)]
pub struct RetryResult<T> {
    /// The successful result
    pub result: T,
    /// Number of retry attempts made
    pub attempts_made: u32,
    /// Total time spent retrying (in seconds)
    pub total_retry_time: u64,
}

// ================================================================================================
// EVENT SYSTEM
// ================================================================================================

/// Request information for client events
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RequestInfo {
    /// The HTTP method (GET, POST, etc.)
    pub method: String,
    /// The full URI being requested
    pub uri: String,
    /// Query parameters as key-value pairs
    pub query_params: Vec<(String, String)>,
    /// Path without query parameters
    pub path: String,
}

impl RequestInfo {
    /// Create RequestInfo from a URL string and method
    pub fn from_url_and_method(url: &str, method: &str) -> Self {
        // Parse URL manually to avoid adding dependencies
        let (path, query_params) = if let Some(query_start) = url.find('?') {
            let path = url[..query_start].to_string();
            let query_string = &url[query_start + 1..];

            let query_params: Vec<(String, String)> = query_string
                .split('&')
                .filter_map(|pair| {
                    if let Some(eq_pos) = pair.find('=') {
                        let key = &pair[..eq_pos];
                        let value = &pair[eq_pos + 1..];
                        Some((key.to_string(), value.to_string()))
                    } else if !pair.is_empty() {
                        Some((pair.to_string(), String::new()))
                    } else {
                        None
                    }
                })
                .collect();

            (path, query_params)
        } else {
            (url.to_string(), Vec::new())
        };

        // Extract just the path part if it's a full URL
        let path = if path.starts_with("http://") || path.starts_with("https://") {
            if let Some(third_slash) = path[8..].find('/') {
                path[8 + third_slash..].to_string()
            } else {
                "/".to_string()
            }
        } else {
            path
        };

        Self {
            method: method.to_string(),
            uri: url.to_string(),
            query_params,
            path,
        }
    }

    /// Get a short description of the request for logging
    pub fn short_description(&self) -> String {
        let mut desc = format!("{} {}", self.method, self.path);
        if !self.query_params.is_empty() {
            let params: Vec<String> = self
                .query_params
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            if params.len() <= 2 {
                desc.push_str(&format!("?{}", params.join("&")));
            } else {
                desc.push_str(&format!("?{}...", params[0]));
            }
        }
        desc
    }
}

/// Type of rate limiting detected
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RateLimitType {
    /// HTTP 429 Too Many Requests
    Http429,
    /// HTTP 403 Forbidden (likely rate limiting)
    Http403,
    /// Rate limit patterns detected in response body
    ResponsePattern,
}

/// Event type to describe internal HTTP client activity
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ClientEvent {
    /// Request started
    RequestStarted {
        /// Request details
        request: RequestInfo,
    },
    /// Request completed successfully
    RequestCompleted {
        /// Request details
        request: RequestInfo,
        /// HTTP status code
        status_code: u16,
        /// Duration of the request in milliseconds
        duration_ms: u64,
    },
    /// Rate limiting detected with backoff duration in seconds
    RateLimited {
        /// Duration to wait in seconds
        delay_seconds: u64,
        /// Request that triggered the rate limit (if available)
        request: Option<RequestInfo>,
        /// Type of rate limiting detected
        rate_limit_type: RateLimitType,
    },
    /// Scrobble edit attempt completed
    EditAttempted {
        /// The exact scrobble edit that was attempted
        edit: ExactScrobbleEdit,
        /// Whether the edit was successful
        success: bool,
        /// Optional error message if the edit failed
        error_message: Option<String>,
        /// Duration of the edit operation in milliseconds
        duration_ms: u64,
    },
}

/// Type alias for the broadcast receiver
pub type ClientEventReceiver = broadcast::Receiver<ClientEvent>;

/// Type alias for the watch receiver
pub type ClientEventWatcher = watch::Receiver<Option<ClientEvent>>;

/// Shared event broadcasting state that persists across client clones
#[derive(Clone)]
pub struct SharedEventBroadcaster {
    event_tx: broadcast::Sender<ClientEvent>,
    last_event_tx: watch::Sender<Option<ClientEvent>>,
}

impl SharedEventBroadcaster {
    /// Create a new shared event broadcaster
    pub fn new() -> Self {
        let (event_tx, _) = broadcast::channel(100);
        let (last_event_tx, _) = watch::channel(None);

        Self {
            event_tx,
            last_event_tx,
        }
    }

    /// Broadcast an event to all subscribers
    pub fn broadcast_event(&self, event: ClientEvent) {
        let _ = self.event_tx.send(event.clone());
        let _ = self.last_event_tx.send(Some(event));
    }

    /// Subscribe to events
    pub fn subscribe(&self) -> ClientEventReceiver {
        self.event_tx.subscribe()
    }

    /// Get the latest event
    pub fn latest_event(&self) -> Option<ClientEvent> {
        self.last_event_tx.borrow().clone()
    }
}

impl Default for SharedEventBroadcaster {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for SharedEventBroadcaster {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedEventBroadcaster")
            .field("subscribers", &self.event_tx.receiver_count())
            .finish()
    }
}

// ================================================================================================
// TESTS
// ================================================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_validity() {
        let valid_session = LastFmEditSession::new(
            "testuser".to_string(),
            vec!["sessionid=.eJy1234567890abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890".to_string()],
            Some("csrf_token_123".to_string()),
            "https://www.last.fm".to_string(),
        );
        assert!(valid_session.is_valid());

        let invalid_session = LastFmEditSession::new(
            "".to_string(),
            vec![],
            None,
            "https://www.last.fm".to_string(),
        );
        assert!(!invalid_session.is_valid());
    }

    #[test]
    fn test_session_serialization() {
        let session = LastFmEditSession::new(
            "testuser".to_string(),
            vec![
                "sessionid=.test123".to_string(),
                "csrftoken=abc".to_string(),
            ],
            Some("csrf_token_123".to_string()),
            "https://www.last.fm".to_string(),
        );

        let json = session.to_json().unwrap();
        let restored_session = LastFmEditSession::from_json(&json).unwrap();

        assert_eq!(session.username, restored_session.username);
        assert_eq!(session.cookies, restored_session.cookies);
        assert_eq!(session.csrf_token, restored_session.csrf_token);
        assert_eq!(session.base_url, restored_session.base_url);
    }
}
