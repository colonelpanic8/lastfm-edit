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
//! use lastfm_edit::{LastFmClient, Result};
//! use http_client::HttpClient;
//!
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!     // Create client with any HTTP implementation
//!     let http_client = HttpClient::new();
//!     let mut client = LastFmClient::new(http_client);
//!
//!     // Login to Last.fm
//!     client.login("username", "password").await?;
//!
//!     // Browse recent tracks
//!     let mut recent_tracks = client.recent_tracks("username").await?;
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
//! ## Examples
//!
//! See the `examples/` directory for complete usage examples including:
//! - Basic login and track listing
//! - Bulk track renaming operations
//! - Artist and album browsing
//! - Recent tracks monitoring

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
