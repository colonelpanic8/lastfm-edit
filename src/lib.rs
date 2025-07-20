pub mod client;
pub mod error;
pub mod iterator;
pub mod track;

pub use client::LastFmClient;
pub use error::LastFmError;
pub use iterator::ArtistTracksIterator;
pub use track::{Track, TrackPage};

pub type Result<T> = std::result::Result<T, LastFmError>;
