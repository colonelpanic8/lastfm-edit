use chrono::{DateTime, Utc};

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
#[derive(Debug, Clone)]
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
#[derive(Debug, Clone)]
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
    pub fn scrobbled_at(&self) -> Option<DateTime<Utc>> {
        self.timestamp
            .and_then(|ts| DateTime::from_timestamp(ts as i64, 0))
    }
}
