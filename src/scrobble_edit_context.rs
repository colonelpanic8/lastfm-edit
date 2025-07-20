use crate::{ScrobbleEdit, LastFmClient, Result};

/// Context object that bridges track listing data with edit functionality
#[derive(Debug, Clone)]
pub struct ScrobbleEditContext {
    /// The track that can be edited
    pub track_name: String,
    pub artist_name: String,
    
    /// Edit strategy - determines how to perform the edit
    pub strategy: EditStrategy,
    
    /// Optional album information if available
    pub album_name: Option<String>,
    
    /// Playcount from track listing (informational)
    pub playcount: u32,
}

#[derive(Debug, Clone)]
pub enum EditStrategy {
    /// Edit all instances of this track/artist combination
    /// Uses edit_all=on and relies on Last.fm's bulk edit functionality
    EditAll,
    
    /// Edit specific scrobbles with known timestamps
    /// Requires fetching actual scrobble data first
    SpecificScrobbles(Vec<u64>), // timestamps
}

impl ScrobbleEditContext {
    /// Create an edit context from track listing data
    /// This is the bridge between track discovery and edit functionality
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
    
    /// Create a scrobble edit request from this context
    pub fn create_edit(&self, new_track_name: String, new_album_name: Option<String>) -> ScrobbleEdit {
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
    
    /// Execute the edit using the provided client
    /// This will automatically fetch real scrobble data if needed
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
                match client.find_recent_scrobble_for_track(&self.track_name, &self.artist_name, 3).await? {
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

    /// Execute the edit with real scrobble data lookup
    /// This is a convenience method that ensures we use real scrobble timestamps
    pub async fn execute_edit_with_real_data(
        &self,
        client: &mut LastFmClient,
        new_track_name: String,
        new_album_name: Option<String>,
    ) -> Result<bool> {
        // First, try to find the most recent scrobble for this track
        match client.find_recent_scrobble_for_track(&self.track_name, &self.artist_name, 5).await? {
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
                        "Found recent scrobble but no timestamp available".to_string()
                    ))
                }
            }
            None => {
                Err(crate::LastFmError::Parse(
                    format!("No recent scrobble found for '{}' by '{}'", self.track_name, self.artist_name)
                ))
            }
        }
    }
    
    /// Get a description of what this edit will do
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
                    timestamps.len(), self.track_name, self.artist_name, new_track_name
                )
            }
        }
    }
}

/// Helper trait to convert track data into edit contexts
pub trait IntoEditContext {
    fn into_edit_context(self) -> ScrobbleEditContext;
}

impl IntoEditContext for crate::Track {
    fn into_edit_context(self) -> ScrobbleEditContext {
        ScrobbleEditContext::from_track_listing(
            self.name,
            self.artist,
            self.playcount,
            None, // Track listing doesn't have reliable album data
        )
    }
}