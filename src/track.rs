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
/// };
///
/// println!("{} by {} (played {} times)", track.name, track.artist, track.playcount);
/// ```
#[derive(Debug, Clone)]
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
#[derive(Debug, Clone)]
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
