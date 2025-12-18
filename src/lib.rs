#![doc = include_str!("../README.md")]

pub mod client;
pub mod discovery;
pub mod edit_analysis;
pub mod headers;
pub mod iterator;
pub mod login;
pub mod parsing;
pub mod retry;
pub mod session_persistence;
pub mod r#trait;
pub mod types;
pub mod vcr_form_data;
pub mod vcr_matcher;
pub mod vcr_test_utils;

pub use client::LastFmEditClientImpl;
pub use discovery::{
    AlbumTracksDiscovery, ArtistTracksDiscovery, AsyncDiscoveryIterator, ExactMatchDiscovery,
    TrackVariationsDiscovery,
};
pub use login::LoginManager;
pub use r#trait::LastFmEditClient;

// Re-export all types from the consolidated types module
pub use iterator::AsyncPaginatedIterator;
pub use types::{
    Album, AlbumPage, Artist, ArtistPage, ClientConfig, ClientEvent, ClientEventReceiver,
    ClientEventWatcher, EditResponse, ExactScrobbleEdit, LastFmEditSession, LastFmError,
    OperationalDelayConfig, RateLimitConfig, RateLimitType, RequestInfo, RetryConfig, RetryResult,
    ScrobbleEdit, SharedEventBroadcaster, SingleEditResponse, Track, TrackPage,
};

// Type aliases for iterators with the concrete client type
pub type ArtistsIterator = iterator::ArtistsIterator<LastFmEditClientImpl>;
pub type ArtistTracksIterator = iterator::ArtistTracksIterator<LastFmEditClientImpl>;
pub type ArtistTracksDirectIterator = iterator::ArtistTracksDirectIterator<LastFmEditClientImpl>;
pub type ArtistAlbumsIterator = iterator::ArtistAlbumsIterator<LastFmEditClientImpl>;
pub type AlbumTracksIterator = iterator::AlbumTracksIterator<LastFmEditClientImpl>;
pub type RecentTracksIterator = iterator::RecentTracksIterator<LastFmEditClientImpl>;
pub type SearchTracksIterator = iterator::SearchTracksIterator<LastFmEditClientImpl>;
pub type SearchAlbumsIterator = iterator::SearchAlbumsIterator<LastFmEditClientImpl>;
pub type SearchArtistsIterator = iterator::SearchArtistsIterator<LastFmEditClientImpl>;
#[cfg(feature = "mock")]
pub use r#trait::MockLastFmEditClient;

// Re-export the mock iterator when the mock feature is enabled
#[cfg(feature = "mock")]
pub use iterator::MockAsyncPaginatedIterator;
pub use session_persistence::{SessionManager, SessionPersistence};

// Re-export scraper types for testing
pub use scraper::Html;

/// A convenient type alias for [`Result`] with [`LastFmError`] as the error type.
pub type Result<T> = std::result::Result<T, LastFmError>;
