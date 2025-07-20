pub mod album;
pub mod client;
pub mod edit;
pub mod error;
pub mod iterator;
pub mod scrobble_edit_context;
pub mod track;
pub mod tui;

pub use album::{Album, AlbumPage};
pub use client::LastFmClient;
pub use edit::{EditResponse, ScrobbleEdit};
pub use error::LastFmError;
pub use iterator::{ArtistAlbumsIterator, ArtistTracksIterator};
pub use scrobble_edit_context::{EditStrategy, IntoEditContext, ScrobbleEditContext};
pub use track::{Track, TrackPage};
pub use tui::{run_track_editor, TrackEditorApp};

pub type Result<T> = std::result::Result<T, LastFmError>;
