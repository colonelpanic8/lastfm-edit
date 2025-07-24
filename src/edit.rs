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
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ScrobbleEdit {
    /// Original track name as it appears in the scrobble (required)
    pub track_name_original: String,
    /// Original album name as it appears in the scrobble (optional)
    pub album_name_original: Option<String>,
    /// Original artist name as it appears in the scrobble (required)
    pub artist_name_original: String,
    /// Original album artist name as it appears in the scrobble (optional)
    pub album_artist_name_original: Option<String>,

    /// New track name to set
    pub track_name: String,
    /// New album name to set
    pub album_name: String,
    /// New artist name to set
    pub artist_name: String,
    /// New album artist name to set
    pub album_artist_name: String,

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
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SingleEditResponse {
    /// Whether this individual edit operation was successful
    pub success: bool,
    /// Optional message describing the result or any errors
    pub message: Option<String>,
    /// Information about which album variation was edited
    pub album_info: Option<String>,
}

/// Response from a scrobble edit operation that may affect multiple album variations.
///
/// When editing a track that appears on multiple albums, this response contains
/// the results of all individual edit operations performed.
///
/// # Examples
///
/// ```rust
/// use lastfm_edit::EditResponse;
///
/// // Check if all edits succeeded
/// if response.all_successful() {
///     println!("All {} edits succeeded!", response.total_edits());
/// } else {
///     println!("Some edits failed: {}", response.summary_message());
/// }
/// ```
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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
/// This type is not part of the public API.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ExactScrobbleEdit {
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
        track_name_original: String,
        album_name_original: Option<String>,
        artist_name_original: String,
        album_artist_name_original: Option<String>,
        track_name: String,
        album_name: String,
        artist_name: String,
        album_artist_name: String,
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
            Some(original_album.to_string()),
            original_artist.to_string(),
            Some(original_artist.to_string()), // album_artist defaults to artist
            original_track.to_string(),
            original_album.to_string(),
            original_artist.to_string(),
            original_artist.to_string(), // album_artist defaults to artist
            Some(timestamp),
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
    ///
    /// # Examples
    ///
    /// ```rust
    /// use lastfm_edit::ScrobbleEdit;
    ///
    /// // Create an edit where the client will look up original metadata
    /// let edit = ScrobbleEdit::with_minimal_info(
    ///     "Corrected Track Name",
    ///     "Corrected Artist",
    ///     "Corrected Album",
    ///     1640995200
    /// );
    /// ```
    pub fn with_minimal_info(
        track_name: &str,
        artist_name: &str,
        album_name: &str,
        timestamp: u64,
    ) -> Self {
        Self::new(
            track_name.to_string(),
            Some(album_name.to_string()),
            artist_name.to_string(),
            Some(artist_name.to_string()),
            track_name.to_string(),
            album_name.to_string(),
            artist_name.to_string(),
            artist_name.to_string(),
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
    ///
    /// # Examples
    ///
    /// ```rust
    /// use lastfm_edit::ScrobbleEdit;
    ///
    /// // Create an edit where the client will look up album and timestamp info
    /// let edit = ScrobbleEdit::from_track_and_artist(
    ///     "Lover Man",
    ///     "Jimi Hendrix"
    /// );
    /// ```
    pub fn from_track_and_artist(track_name: &str, artist_name: &str) -> Self {
        Self::new(
            track_name.to_string(),
            None, // Client will look up original album name
            artist_name.to_string(),
            None, // Client will look up original album artist name
            track_name.to_string(),
            String::new(), // Will be filled by client
            artist_name.to_string(),
            artist_name.to_string(), // album_artist defaults to artist
            None,                    // Client will find representative timestamp
            false,
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

    /// Convert this exact edit back to a public ScrobbleEdit.
    ///
    /// This is useful when you need to expose the edit data through the public API.
    pub fn to_scrobble_edit(&self) -> ScrobbleEdit {
        ScrobbleEdit::new(
            self.track_name_original.clone(),
            Some(self.album_name_original.clone()),
            self.artist_name_original.clone(),
            Some(self.album_artist_name_original.clone()),
            self.track_name.clone(),
            self.album_name.clone(),
            self.artist_name.clone(),
            self.album_artist_name.clone(),
            Some(self.timestamp),
            self.edit_all,
        )
    }
}

impl EditResponse {
    /// Create a new EditResponse from a single result.
    pub fn single(success: bool, message: Option<String>, album_info: Option<String>) -> Self {
        Self {
            individual_results: vec![SingleEditResponse {
                success,
                message,
                album_info,
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
