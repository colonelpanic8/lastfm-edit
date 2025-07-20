#[derive(Debug, Clone)]
pub struct ScrobbleEdit {
    // Original track information
    pub track_name_original: String,
    pub album_name_original: String,
    pub artist_name_original: String,
    pub album_artist_name_original: String,

    // Edited track information
    pub track_name: String,
    pub album_name: String,
    pub artist_name: String,
    pub album_artist_name: String,

    // Metadata
    pub timestamp: u64,
    pub edit_all: bool, // Whether to edit all instances or just this one
}

#[derive(Debug)]
pub struct EditResponse {
    pub success: bool,
    pub message: Option<String>,
}

impl ScrobbleEdit {
    pub fn new() -> Self {
        Self {
            track_name_original: String::new(),
            album_name_original: String::new(),
            artist_name_original: String::new(),
            album_artist_name_original: String::new(),
            track_name: String::new(),
            album_name: String::new(),
            artist_name: String::new(),
            album_artist_name: String::new(),
            timestamp: 0,
            edit_all: false,
        }
    }

    /// Create an edit request from track information
    pub fn from_track_info(
        original_track: &str,
        original_album: &str,
        original_artist: &str,
        timestamp: u64,
    ) -> Self {
        Self {
            track_name_original: original_track.to_string(),
            album_name_original: original_album.to_string(),
            artist_name_original: original_artist.to_string(),
            album_artist_name_original: original_artist.to_string(),
            track_name: original_track.to_string(),
            album_name: original_album.to_string(),
            artist_name: original_artist.to_string(),
            album_artist_name: original_artist.to_string(),
            timestamp,
            edit_all: false,
        }
    }

    /// Set the new track name
    pub fn with_track_name(mut self, track_name: &str) -> Self {
        self.track_name = track_name.to_string();
        self
    }

    /// Set the new album name
    pub fn with_album_name(mut self, album_name: &str) -> Self {
        self.album_name = album_name.to_string();
        self
    }

    /// Set the new artist name
    pub fn with_artist_name(mut self, artist_name: &str) -> Self {
        self.artist_name = artist_name.to_string();
        self.album_artist_name = artist_name.to_string();
        self
    }

    /// Set whether to edit all instances of this track
    pub fn with_edit_all(mut self, edit_all: bool) -> Self {
        self.edit_all = edit_all;
        self
    }
}

impl Default for ScrobbleEdit {
    fn default() -> Self {
        Self::new()
    }
}
