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
//! use lastfm_edit::{LastFmClient, AsyncPaginatedIterator, Result};
//!
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!     // Create client with any HTTP implementation
//!     let http_client = http_client::native::NativeClient::new();
//!     let mut client = LastFmClient::new(Box::new(http_client));
//!
//!     // Login to Last.fm
//!     client.login("username", "password").await?;
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
//! - [`LastFmClient`] - Main client for interacting with Last.fm
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
//! use lastfm_edit::{LastFmClient, AsyncPaginatedIterator, Result};
//!
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!     let http_client = http_client::native::NativeClient::new();
//!     let mut client = LastFmClient::new(Box::new(http_client));
//!
//!     client.login("username", "password").await?;
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
//! use lastfm_edit::{LastFmClient, ScrobbleEditContext, EditStrategy, Result};
//!
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!     let http_client = http_client::native::NativeClient::new();
//!     let mut client = LastFmClient::new(Box::new(http_client));
//!
//!     client.login("username", "password").await?;
//!
//!     // Create edit context for bulk operations
//!     let mut context = ScrobbleEditContext::new(&mut client, EditStrategy::DryRun);
//!
//!     // Find and edit tracks
//!     let tracks = client.artist_tracks("Artist Name").collect_all().await?;
//!     for track in tracks {
//!         if track.name.contains("(Remaster)") {
//!             let new_name = track.name.replace(" (Remaster)", "");
//!             context.edit_track(&track, Some(&new_name), None, None).await?;
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
//! use lastfm_edit::{LastFmClient, AsyncPaginatedIterator, Result};
//!
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!     let http_client = http_client::native::NativeClient::new();
//!     let mut client = LastFmClient::new(Box::new(http_client));
//!
//!     client.login("username", "password").await?;
//!
//!     // Get recent tracks (first 100)
//!     let recent_tracks = client.recent_tracks().take(100).collect_all().await?;
//!     println!("Found {} recent tracks", recent_tracks.len());
//!
//!     Ok(())
//! }
//! ```
//!
//! ## License
//!
//! MIT

pub mod album;
pub mod client;
pub mod edit;
pub mod error;
pub mod iterator;
pub mod scrobble_edit_context;
pub mod track;

pub use album::{Album, AlbumPage};
pub use client::LastFmClient;
pub use edit::{EditResponse, ScrobbleEdit};
pub use error::LastFmError;
pub use iterator::{
    ArtistAlbumsIterator, ArtistTracksIterator, AsyncPaginatedIterator, RecentTracksIterator,
};
pub use scrobble_edit_context::{EditStrategy, IntoEditContext, ScrobbleEditContext};
pub use track::{Track, TrackPage};

// Re-export scraper types for testing
pub use scraper::Html;

/// A convenient type alias for [`Result`] with [`LastFmError`] as the error type.
pub type Result<T> = std::result::Result<T, LastFmError>;
