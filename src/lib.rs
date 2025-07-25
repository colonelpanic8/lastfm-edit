//! # lastfm-edit
//!
//! A Rust crate for programmatic access to Last.fm's scrobble editing functionality via web scraping.
//!
//! This crate provides a high-level interface for authenticating with Last.fm, browsing user libraries,
//! and performing bulk edits on scrobbled tracks. It uses web scraping to access functionality not
//! available through Last.fm's public API.
//!
//! ## Features
//!
//! - **Authentication**: Login to Last.fm with username/password
//! - **Library browsing**: Paginated access to tracks, albums, and recent scrobbles
//! - **Bulk editing**: Edit track names, artist names, and album information
//! - **Async iterators**: Stream large datasets efficiently
//! - **HTTP client abstraction**: Works with any HTTP client implementation
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use lastfm_edit::{LastFmEditClient, LastFmEditClientImpl, AsyncPaginatedIterator, Result};
//!
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!     // Create HTTP client and login
//!     let http_client = http_client::native::NativeClient::new();
//!     let client = LastFmEditClientImpl::login_with_credentials(
//!         Box::new(http_client),
//!         "username",
//!         "password"
//!     ).await?;
//!
//!     // Browse recent tracks
//!     let mut recent_tracks = client.recent_tracks();
//!     while let Some(track) = recent_tracks.next().await? {
//!         println!("{} - {}", track.artist, track.name);
//!     }
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Core Components
//!
//! - [`LastFmEditClient`] - Main client trait for interacting with Last.fm
//! - [`Track`], [`Album`] - Data structures for music metadata
//! - [`AsyncPaginatedIterator`] - Trait for streaming paginated data
//! - [`ScrobbleEdit`] - Represents track edit operations
//! - [`LastFmError`] - Error types for the crate
//!
//! ## Installation
//!
//! Add this to your `Cargo.toml`:
//! ```toml
//! [dependencies]
//! lastfm-edit = "0.1.0"
//! http-client = { version = "6.5", features = ["curl_client"] }
//! tokio = { version = "1.0", features = ["full"] }
//! ```
//!
//! ## Usage Patterns
//!
//! ### Basic Library Browsing
//!
//! ```rust,no_run
//! use lastfm_edit::{LastFmEditClient, LastFmEditClientImpl, AsyncPaginatedIterator, Result};
//!
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!     let http_client = http_client::native::NativeClient::new();
//!     let client = LastFmEditClientImpl::login_with_credentials(
//!         Box::new(http_client),
//!         "username",
//!         "password"
//!     ).await?;
//!
//!     // Get all tracks by an artist
//!     let mut tracks = client.artist_tracks("Radiohead");
//!     while let Some(track) = tracks.next().await? {
//!         println!("{} - {}", track.artist, track.name);
//!     }
//!
//!     Ok(())
//! }
//! ```
//!
//! ### Bulk Track Editing
//!
//! ```rust,no_run
//! use lastfm_edit::{LastFmEditClient, LastFmEditClientImpl, ScrobbleEdit, AsyncPaginatedIterator, Result};
//!
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!     let http_client = http_client::native::NativeClient::new();
//!     let client = LastFmEditClientImpl::login_with_credentials(
//!         Box::new(http_client),
//!         "username",
//!         "password"
//!     ).await?;
//!
//!     // Find and edit tracks
//!     let tracks = client.artist_tracks("Artist Name").collect_all().await?;
//!     for track in tracks {
//!         if track.name.contains("(Remaster)") {
//!             let new_name = track.name.replace(" (Remaster)", "");
//!
//!             // Create edit for this track
//!             let edit = ScrobbleEdit::from_track_info(
//!                 &track.name,
//!                 &track.name, // Use track name as album fallback
//!                 &track.artist,
//!                 0 // No timestamp needed for bulk edit
//!             )
//!             .with_track_name(&new_name)
//!             .with_edit_all(true);
//!
//!             let response = client.edit_scrobble(&edit).await?;
//!             if response.success() {
//!                 println!("Successfully edited: {} -> {}", track.name, new_name);
//!             }
//!         }
//!     }
//!
//!     Ok(())
//! }
//! ```
//!
//! ### Recent Tracks Monitoring
//!
//! ```rust,no_run
//! use lastfm_edit::{LastFmEditClient, LastFmEditClientImpl, AsyncPaginatedIterator, Result};
//!
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!     let http_client = http_client::native::NativeClient::new();
//!     let client = LastFmEditClientImpl::login_with_credentials(
//!         Box::new(http_client),
//!         "username",
//!         "password"
//!     ).await?;
//!
//!     // Get recent tracks (first 100)
//!     let recent_tracks = client.recent_tracks().take(100).await?;
//!     println!("Found {} recent tracks", recent_tracks.len());
//!
//!     Ok(())
//! }
//! ```
//!
//! ### Mocking for Testing
//!
//! Enable the `mock` feature to use `MockLastFmEditClient` for testing:
//!
//! ```toml
//! [dev-dependencies]
//! lastfm-edit = { version = "1.0.0", features = ["mock"] }
//! mockall = "0.13"
//! ```
//!
//! ```rust,ignore
//! #[cfg(feature = "mock")]
//! mod tests {
//!     use lastfm_edit::{LastFmEditClient, MockLastFmEditClient, Result, EditResponse, ScrobbleEdit};
//!     use mockall::predicate::*;
//!
//!     #[tokio::test]
//!     async fn test_edit_workflow() -> Result<()> {
//!         let mut mock_client = MockLastFmEditClient::new();
//!
//!         // Set up expectations
//!         mock_client
//!             .expect_login()
//!             .with(eq("testuser"), eq("testpass"))
//!             .times(1)
//!             .returning(|_, _| Ok(()));
//!
//!         mock_client
//!             .expect_edit_scrobble()
//!             .times(1)
//!             .returning(|_| Ok(EditResponse {
//!                 success: true,
//!                 message: Some("Edit successful".to_string()),
//!             }));
//!
//!         // Use as trait object
//!         let client: &dyn LastFmEditClient = &mock_client;
//!
//!         client.login("testuser", "testpass").await?;
//!
//!         let edit = ScrobbleEdit::new(
//!             Some("Old Track".to_string()),
//!             Some("Old Album".to_string()),
//!             Some("Old Artist".to_string()),
//!             Some("Old Artist".to_string()),
//!             "New Track".to_string(),
//!             "New Album".to_string(),
//!             "New Artist".to_string(),
//!             "New Artist".to_string(),
//!             1640995200,
//!             false,
//!         );
//!
//!         let response = client.edit_scrobble(&edit).await?;
//!         assert!(response.success);
//!
//!         Ok(())
//!     }
//! }
//! ```
//!
//! ## License
//!
//! MIT

pub mod album;
pub mod client;
pub mod discovery;
pub mod discovery_iterator;
pub mod edit;
pub mod error;
pub mod headers;
pub mod iterator;
pub mod login;
pub mod parsing;
pub mod session;
pub mod track;
pub mod r#trait;

pub use album::{Album, AlbumPage};
pub use client::{ClientEvent, ClientEventReceiver, ClientEventWatcher, LastFmEditClientImpl};
pub use discovery::{
    AlbumTracksDiscovery, ArtistTracksDiscovery, ExactMatchDiscovery, TrackVariationsDiscovery,
};
pub use discovery_iterator::AsyncDiscoveryIterator;
pub use login::LoginManager;
pub use r#trait::LastFmEditClient;

// Re-export the mock when the mock feature is enabled
pub use edit::{EditResponse, ExactScrobbleEdit, ScrobbleEdit, SingleEditResponse};
pub use error::LastFmError;
pub use iterator::{
    AlbumTracksIterator, ArtistAlbumsIterator, ArtistTracksIterator, AsyncPaginatedIterator,
    RecentTracksIterator,
};
#[cfg(feature = "mock")]
pub use r#trait::MockLastFmEditClient;

// Iterator-based convenience methods for the client
impl LastFmEditClientImpl {
    /// Create an iterator for browsing an artist's tracks from the user's library.
    pub fn artist_tracks(&self, artist: &str) -> ArtistTracksIterator {
        ArtistTracksIterator::new(self.clone(), artist.to_string())
    }

    /// Create an iterator for browsing an artist's albums from the user's library.
    pub fn artist_albums(&self, artist: &str) -> ArtistAlbumsIterator {
        ArtistAlbumsIterator::new(self.clone(), artist.to_string())
    }

    /// Create an iterator for browsing tracks in a specific album from the user's library.
    pub fn album_tracks(&self, album_name: &str, artist_name: &str) -> AlbumTracksIterator {
        AlbumTracksIterator::new(
            self.clone(),
            album_name.to_string(),
            artist_name.to_string(),
        )
    }

    /// Create an iterator for browsing the user's recent tracks/scrobbles.
    pub fn recent_tracks(&self) -> RecentTracksIterator {
        RecentTracksIterator::new(self.clone())
    }

    /// Create an iterator for browsing the user's recent tracks starting from a specific page.
    pub fn recent_tracks_from_page(&self, starting_page: u32) -> RecentTracksIterator {
        RecentTracksIterator::with_starting_page(self.clone(), starting_page)
    }

    /// Create an incremental discovery iterator for scrobble editing
    ///
    /// This returns the appropriate discovery iterator based on what fields are specified
    /// in the ScrobbleEdit. The iterator yields `ExactScrobbleEdit` results incrementally,
    /// which helps avoid rate limiting issues when discovering many scrobbles.
    ///
    /// Returns a `Box<dyn AsyncDiscoveryIterator<ExactScrobbleEdit>>` to handle the different
    /// discovery strategies uniformly.
    pub fn discover_scrobbles(
        &self,
        edit: ScrobbleEdit,
    ) -> Box<dyn AsyncDiscoveryIterator<ExactScrobbleEdit>> {
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
}

// Re-export the mock iterator when the mock feature is enabled
#[cfg(feature = "mock")]
pub use iterator::MockAsyncPaginatedIterator;
pub use session::LastFmEditSession;
pub use track::{Track, TrackPage};

// Re-export scraper types for testing
pub use scraper::Html;

/// A convenient type alias for [`Result`] with [`LastFmError`] as the error type.
pub type Result<T> = std::result::Result<T, LastFmError>;
