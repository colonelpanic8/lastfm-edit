/// Represents a scrobble edit operation.
///
/// This structure contains all the information needed to edit a specific scrobble
/// on Last.fm, including both the original and new metadata values.
///
/// # Examples
///
/// ```rust
/// use lastfm_edit::ScrobbleEdit;
///
/// // Create an edit to fix a track name
/// let edit = ScrobbleEdit::from_track_info(
///     "Paranoid Andriod", // original (misspelled)
///     "OK Computer",
///     "Radiohead",
///     1640995200
/// )
/// .with_track_name("Paranoid Android"); // corrected
///
/// // Create an edit to change artist name
/// let edit = ScrobbleEdit::from_track_info(
///     "Creep",
///     "Pablo Honey",
///     "Radio Head", // original (wrong)
///     1640995200
/// )
/// .with_artist_name("Radiohead") // corrected
/// .with_edit_all(true); // update all instances
/// ```
#[derive(Debug, Clone)]
pub struct ScrobbleEdit {
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
    ///
    /// This identifies the specific scrobble instance to modify.
    pub timestamp: u64,
    /// Whether to edit all instances or just this specific scrobble
    ///
    /// When `true`, Last.fm will update all scrobbles with matching metadata.
    /// When `false`, only this specific scrobble (identified by timestamp) is updated.
    pub edit_all: bool,
}

/// Response from a scrobble edit operation.
///
/// This structure contains the result of attempting to edit a scrobble,
/// including success status and any error messages.
///
/// # Examples
///
/// ```rust
/// use lastfm_edit::EditResponse;
///
/// let response = EditResponse {
///     success: true,
///     message: Some("Track name updated successfully".to_string()),
/// };
///
/// if response.success {
///     println!("Edit succeeded: {}", response.message.unwrap_or_default());
/// } else {
///     eprintln!("Edit failed: {}", response.message.unwrap_or_default());
/// }
/// ```
#[derive(Debug)]
pub struct EditResponse {
    /// Whether the edit operation was successful
    pub success: bool,
    /// Optional message describing the result or any errors
    pub message: Option<String>,
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
    ///
    /// # Examples
    ///
    /// ```rust
    /// use lastfm_edit::ScrobbleEdit;
    ///
    /// let edit = ScrobbleEdit::from_track_info(
    ///     "Highway to Hell",
    ///     "Highway to Hell",
    ///     "AC/DC",
    ///     1640995200
    /// )
    /// .with_track_name("Highway to Hell (Remastered)");
    /// ```
    pub fn from_track_info(
        original_track: &str,
        original_album: &str,
        original_artist: &str,
        timestamp: u64,
    ) -> Self {
        Self::new(
            original_track.to_string(),
            original_album.to_string(),
            original_artist.to_string(),
            original_artist.to_string(), // album_artist defaults to artist
            original_track.to_string(),
            original_album.to_string(),
            original_artist.to_string(),
            original_artist.to_string(), // album_artist defaults to artist
            timestamp,
            false, // edit_all defaults to false
        )
    }

    /// Set the new track name.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use lastfm_edit::ScrobbleEdit;
    /// let edit = ScrobbleEdit::from_track_info("Wrong Name", "Album", "Artist", 1640995200)
    ///     .with_track_name("Correct Name");
    /// ```
    pub fn with_track_name(mut self, track_name: &str) -> Self {
        self.track_name = track_name.to_string();
        self
    }

    /// Set the new album name.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use lastfm_edit::ScrobbleEdit;
    /// let edit = ScrobbleEdit::from_track_info("Track", "Wrong Album", "Artist", 1640995200)
    ///     .with_album_name("Correct Album");
    /// ```
    pub fn with_album_name(mut self, album_name: &str) -> Self {
        self.album_name = album_name.to_string();
        self
    }

    /// Set the new artist name.
    ///
    /// This also sets the album artist name to the same value.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use lastfm_edit::ScrobbleEdit;
    /// let edit = ScrobbleEdit::from_track_info("Track", "Album", "Wrong Artist", 1640995200)
    ///     .with_artist_name("Correct Artist");
    /// ```
    pub fn with_artist_name(mut self, artist_name: &str) -> Self {
        self.artist_name = artist_name.to_string();
        self.album_artist_name = artist_name.to_string();
        self
    }

    /// Set whether to edit all instances of this track.
    ///
    /// When `true`, Last.fm will update all scrobbles with the same metadata.
    /// When `false` (default), only the specific scrobble is updated.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use lastfm_edit::ScrobbleEdit;
    /// let edit = ScrobbleEdit::from_track_info("Track", "Album", "Artist", 1640995200)
    ///     .with_track_name("New Name")
    ///     .with_edit_all(true); // Update all instances
    /// ```
    pub fn with_edit_all(mut self, edit_all: bool) -> Self {
        self.edit_all = edit_all;
        self
    }
}
