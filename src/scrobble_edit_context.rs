use crate::{LastFmClient, Result, ScrobbleEdit};

/// Context object that bridges track listing data with edit functionality.
///
/// This structure provides a high-level interface for editing scrobbles by combining
/// track discovery data with edit operations. It handles the complexity of choosing
/// between bulk edits and specific scrobble edits.
///
/// # Examples
///
/// ```rust,no_run
/// use lastfm_edit::{ScrobbleEditContext, EditStrategy, IntoEditContext};
/// use lastfm_edit::{LastFmClient, AsyncPaginatedIterator};
/// use http_client::HttpClient;
///
/// # tokio_test::block_on(async {
/// let mut client = LastFmClient::new(HttpClient::new());
/// // client.login(...).await?;
///
/// // Find tracks and convert to edit contexts
/// let mut tracks = client.artist_tracks("Radiohead");
/// let first_track = tracks.next().await?.unwrap();
/// let edit_context = first_track.into_edit_context();
///
/// // Execute a bulk edit
/// let success = edit_context.execute_edit(
///     &mut client,
///     "Corrected Track Name".to_string(),
///     None
/// ).await?;
///
/// if success {
///     println!("Edit completed successfully");
/// }
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// # });
/// ```
#[derive(Debug, Clone)]
pub struct ScrobbleEditContext {
    /// The track name to be edited
    pub track_name: String,
    /// The artist name
    pub artist_name: String,

    /// Edit strategy - determines how to perform the edit
    pub strategy: EditStrategy,

    /// Optional album information if available
    pub album_name: Option<String>,

    /// Playcount from track listing (informational)
    ///
    /// This gives an estimate of how many scrobbles might be affected
    /// when using [`EditStrategy::EditAll`].
    pub playcount: u32,
}

/// Strategy for performing scrobble edits.
///
/// This enum determines whether to edit all matching scrobbles or only
/// specific instances identified by timestamps.
///
/// # Examples
///
/// ```rust
/// use lastfm_edit::EditStrategy;
///
/// // Edit all instances - good for fixing systematic errors
/// let bulk_strategy = EditStrategy::EditAll;
///
/// // Edit specific scrobbles - good for precision edits
/// let specific_strategy = EditStrategy::SpecificScrobbles(vec![1640995200, 1641000000]);
/// ```
#[derive(Debug, Clone)]
pub enum EditStrategy {
    /// Edit all instances of this track/artist combination.
    ///
    /// Uses Last.fm's bulk edit functionality to update all scrobbles
    /// with matching metadata. This is efficient but affects all instances.
    EditAll,

    /// Edit specific scrobbles with known timestamps.
    ///
    /// Targets only the scrobbles identified by the provided timestamps.
    /// This requires fetching actual scrobble data first but provides
    /// more precise control.
    SpecificScrobbles(Vec<u64>),
}

impl ScrobbleEditContext {
    /// Create an edit context from track listing data.
    ///
    /// This is the primary bridge between track discovery and edit functionality.
    /// The context defaults to [`EditStrategy::EditAll`] for simplicity.
    ///
    /// # Arguments
    ///
    /// * `track_name` - The track name from track listings
    /// * `artist_name` - The artist name
    /// * `playcount` - Play count (gives estimate of affected scrobbles)
    /// * `album_name` - Optional album name if available
    ///
    /// # Examples
    ///
    /// ```rust
    /// use lastfm_edit::ScrobbleEditContext;
    ///
    /// let context = ScrobbleEditContext::from_track_listing(
    ///     "Paranoid Android".to_string(),
    ///     "Radiohead".to_string(),
    ///     42,
    ///     Some("OK Computer".to_string())
    /// );
    /// ```
    pub fn from_track_listing(
        track_name: String,
        artist_name: String,
        playcount: u32,
        album_name: Option<String>,
    ) -> Self {
        Self {
            track_name,
            artist_name,
            album_name,
            playcount,
            strategy: EditStrategy::EditAll, // Default to edit_all for simplicity
        }
    }

    /// Create a scrobble edit request from this context.
    ///
    /// This method generates a [`ScrobbleEdit`] based on the context's strategy
    /// and the provided new values.
    ///
    /// # Arguments
    ///
    /// * `new_track_name` - The corrected track name
    /// * `new_album_name` - Optional corrected album name
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use lastfm_edit::ScrobbleEditContext;
    /// let context = ScrobbleEditContext::from_track_listing(
    ///     "Wrong Name".to_string(),
    ///     "Artist".to_string(),
    ///     10,
    ///     None
    /// );
    ///
    /// let edit = context.create_edit(
    ///     "Correct Name".to_string(),
    ///     Some("Album".to_string())
    /// );
    /// ```
    pub fn create_edit(
        &self,
        new_track_name: String,
        new_album_name: Option<String>,
    ) -> ScrobbleEdit {
        let original_album = self.album_name.as_deref().unwrap_or(&self.track_name);
        let target_album = new_album_name.as_deref().unwrap_or(&new_track_name);

        match &self.strategy {
            EditStrategy::EditAll => {
                ScrobbleEdit::from_track_info(
                    &self.track_name,
                    original_album,
                    &self.artist_name,
                    0, // No timestamp needed for edit_all
                )
                .with_track_name(&new_track_name)
                .with_album_name(target_album)
                .with_edit_all(true)
            }
            EditStrategy::SpecificScrobbles(timestamps) => {
                // For now, just use the first timestamp
                // In a full implementation, you'd want to handle multiple timestamps
                let timestamp = timestamps.first().copied().unwrap_or(0);

                ScrobbleEdit::from_track_info(
                    &self.track_name,
                    original_album,
                    &self.artist_name,
                    timestamp,
                )
                .with_track_name(&new_track_name)
                .with_album_name(target_album)
                .with_edit_all(false)
            }
        }
    }

    /// Execute the edit using the provided client.
    ///
    /// This method automatically handles the complexity of finding real scrobble
    /// data when needed and submitting the edit request to Last.fm.
    ///
    /// # Arguments
    ///
    /// * `client` - Authenticated Last.fm client
    /// * `new_track_name` - The corrected track name
    /// * `new_album_name` - Optional corrected album name
    ///
    /// # Returns
    ///
    /// Returns `Ok(true)` if the edit was successful, `Ok(false)` if it failed,
    /// or `Err(...)` if there was a network or other error.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # use lastfm_edit::{ScrobbleEditContext, LastFmClient};
    /// # use http_client::HttpClient;
    /// # tokio_test::block_on(async {
    /// let mut client = LastFmClient::new(HttpClient::new());
    /// let context = ScrobbleEditContext::from_track_listing(
    ///     "Wrong Name".to_string(),
    ///     "Artist".to_string(),
    ///     5,
    ///     None
    /// );
    ///
    /// let success = context.execute_edit(
    ///     &mut client,
    ///     "Correct Name".to_string(),
    ///     None
    /// ).await?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// # });
    /// ```
    pub async fn execute_edit(
        &self,
        client: &mut LastFmClient,
        new_track_name: String,
        new_album_name: Option<String>,
    ) -> Result<bool> {
        // For EditAll strategy, try to get a real scrobble timestamp first
        let edit = match &self.strategy {
            EditStrategy::EditAll => {
                // Try to find a recent scrobble to get real timestamp data
                match client
                    .find_recent_scrobble_for_track(&self.track_name, &self.artist_name, 3)
                    .await?
                {
                    Some(recent_scrobble) => {
                        if let Some(timestamp) = recent_scrobble.timestamp {
                            // Use real scrobble data for better compatibility
                            ScrobbleEdit::from_track_info(
                                &self.track_name,
                                &self.track_name, // Use track name as album fallback
                                &self.artist_name,
                                timestamp,
                            )
                            .with_track_name(&new_track_name)
                            .with_album_name(new_album_name.as_deref().unwrap_or(&new_track_name))
                            .with_edit_all(true)
                        } else {
                            // Fallback to original approach if no timestamp
                            self.create_edit(new_track_name.clone(), new_album_name.clone())
                        }
                    }
                    None => {
                        // No recent scrobble found, use original approach
                        self.create_edit(new_track_name.clone(), new_album_name.clone())
                    }
                }
            }
            EditStrategy::SpecificScrobbles(_) => {
                // For specific scrobbles, use the provided timestamps
                self.create_edit(new_track_name.clone(), new_album_name.clone())
            }
        };

        let response = client.edit_scrobble(&edit).await?;
        Ok(response.success)
    }

    /// Execute the edit with real scrobble data lookup.
    ///
    /// This convenience method ensures that real scrobble timestamps are used
    /// by explicitly searching the user's recent scrobbles first. This can be
    /// more reliable than relying on track listing data.
    ///
    /// # Arguments
    ///
    /// * `client` - Authenticated Last.fm client
    /// * `new_track_name` - The corrected track name
    /// * `new_album_name` - Optional corrected album name
    ///
    /// # Returns
    ///
    /// Returns `Ok(true)` if successful, or an error if no recent scrobble
    /// could be found or if the edit failed.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # use lastfm_edit::{ScrobbleEditContext, LastFmClient};
    /// # use http_client::HttpClient;
    /// # tokio_test::block_on(async {
    /// let mut client = LastFmClient::new(HttpClient::new());
    /// let context = ScrobbleEditContext::from_track_listing(
    ///     "Misspelled Track".to_string(),
    ///     "Artist".to_string(),
    ///     1,
    ///     None
    /// );
    ///
    /// // This will search recent scrobbles for real timestamp data
    /// let success = context.execute_edit_with_real_data(
    ///     &mut client,
    ///     "Correctly Spelled Track".to_string(),
    ///     None
    /// ).await?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// # });
    /// ```
    pub async fn execute_edit_with_real_data(
        &self,
        client: &mut LastFmClient,
        new_track_name: String,
        new_album_name: Option<String>,
    ) -> Result<bool> {
        // First, try to find the most recent scrobble for this track
        match client
            .find_recent_scrobble_for_track(&self.track_name, &self.artist_name, 5)
            .await?
        {
            Some(recent_scrobble) => {
                if let Some(timestamp) = recent_scrobble.timestamp {
                    // Create edit with real scrobble data
                    let edit = ScrobbleEdit::from_track_info(
                        &recent_scrobble.name,
                        &recent_scrobble.name, // Use track name as album fallback
                        &recent_scrobble.artist,
                        timestamp,
                    )
                    .with_track_name(&new_track_name)
                    .with_album_name(new_album_name.as_deref().unwrap_or(&new_track_name))
                    .with_edit_all(true);

                    let response = client.edit_scrobble(&edit).await?;
                    Ok(response.success)
                } else {
                    Err(crate::LastFmError::Parse(
                        "Found recent scrobble but no timestamp available".to_string(),
                    ))
                }
            }
            None => Err(crate::LastFmError::Parse(format!(
                "No recent scrobble found for '{}' by '{}'",
                self.track_name, self.artist_name
            ))),
        }
    }

    /// Get a human-readable description of what this edit will do.
    ///
    /// This is useful for logging or user confirmation before executing edits.
    ///
    /// # Arguments
    ///
    /// * `new_track_name` - The target track name for the edit
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use lastfm_edit::ScrobbleEditContext;
    /// let context = ScrobbleEditContext::from_track_listing(
    ///     "Old Name".to_string(),
    ///     "Artist".to_string(),
    ///     15,
    ///     None
    /// );
    ///
    /// let description = context.describe_edit("New Name");
    /// println!("{}", description);
    /// // Output: "Will edit ALL instances of 'Old Name' by 'Artist' to 'New Name' (approximately 15 scrobbles)"
    /// ```
    pub fn describe_edit(&self, new_track_name: &str) -> String {
        match &self.strategy {
            EditStrategy::EditAll => {
                format!(
                    "Will edit ALL instances of '{}' by '{}' to '{}' (approximately {} scrobbles)",
                    self.track_name, self.artist_name, new_track_name, self.playcount
                )
            }
            EditStrategy::SpecificScrobbles(timestamps) => {
                format!(
                    "Will edit {} specific scrobbles of '{}' by '{}' to '{}'",
                    timestamps.len(),
                    self.track_name,
                    self.artist_name,
                    new_track_name
                )
            }
        }
    }
}

/// Helper trait to convert track data into edit contexts.
///
/// This trait provides a convenient way to transform track listing data
/// into editable contexts, bridging the gap between discovery and editing.
///
/// # Examples
///
/// ```rust,no_run
/// use lastfm_edit::{LastFmClient, AsyncPaginatedIterator, IntoEditContext};
/// use http_client::HttpClient;
///
/// # tokio_test::block_on(async {
/// let mut client = LastFmClient::new(HttpClient::new());
/// // client.login(...).await?;
///
/// let mut tracks = client.artist_tracks("Radiohead");
/// if let Some(track) = tracks.next().await? {
///     let edit_context = track.into_edit_context();
///
///     // Now ready to edit
///     let success = edit_context.execute_edit(
///         &mut client,
///         "Fixed Track Name".to_string(),
///         None
///     ).await?;
/// }
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// # });
/// ```
pub trait IntoEditContext {
    /// Convert this item into a [`ScrobbleEditContext`].
    fn into_edit_context(self) -> ScrobbleEditContext;
}

impl IntoEditContext for crate::Track {
    /// Convert a [`Track`](crate::Track) into a [`ScrobbleEditContext`].
    ///
    /// The resulting context uses [`EditStrategy::EditAll`] by default and
    /// doesn't include album information since track listings don't have
    /// reliable album data.
    fn into_edit_context(self) -> ScrobbleEditContext {
        ScrobbleEditContext::from_track_listing(
            self.name,
            self.artist,
            self.playcount,
            None, // Track listing doesn't have reliable album data
        )
    }
}
