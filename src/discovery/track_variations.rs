use super::common::filter_by_original_album_artist;
use crate::{
    AsyncDiscoveryIterator, ExactScrobbleEdit, LastFmEditClientImpl, Result, ScrobbleEdit,
};
use async_trait::async_trait;

/// Case 2: Track variations discovery (track specified, album not specified)
///
/// This discovers all album variations of a specific track by loading the track's
/// scrobble data incrementally and yielding each variation as it processes them.
/// This is now truly incremental like the artist and album tracks discovery.
pub struct TrackVariationsDiscovery {
    client: LastFmEditClientImpl,
    edit: ScrobbleEdit,
    track_name: String,
    scrobbles_loaded: bool,
    current_results: Vec<ExactScrobbleEdit>,
    current_index: usize,
}

impl TrackVariationsDiscovery {
    pub fn new(client: LastFmEditClientImpl, edit: ScrobbleEdit, track_name: String) -> Self {
        Self {
            client,
            edit,
            track_name,
            scrobbles_loaded: false,
            current_results: Vec::new(),
            current_index: 0,
        }
    }
}

#[async_trait(?Send)]
impl AsyncDiscoveryIterator<ExactScrobbleEdit> for TrackVariationsDiscovery {
    async fn next(&mut self) -> Result<Option<ExactScrobbleEdit>> {
        // If we have results from current batch, return the next one
        if self.current_index < self.current_results.len() {
            let result = self.current_results[self.current_index].clone();
            self.current_index += 1;
            return Ok(Some(result));
        }

        // If we haven't loaded scrobbles yet, load them
        if !self.scrobbles_loaded {
            log::debug!(
                "Getting scrobble data for track '{}' by '{}'",
                self.track_name,
                self.edit.artist_name_original
            );

            match self
                .client
                .load_edit_form_values_internal(&self.track_name, &self.edit.artist_name_original)
                .await
            {
                Ok(track_scrobbles) => {
                    // Apply user's changes and filtering
                    let mut modified_edits = Vec::new();
                    for scrobble in track_scrobbles {
                        let mut modified_edit = scrobble.clone();
                        if let Some(new_track_name) = &self.edit.track_name {
                            modified_edit.track_name = new_track_name.clone();
                        }
                        if let Some(new_album_name) = &self.edit.album_name {
                            modified_edit.album_name = new_album_name.clone();
                        }
                        modified_edit.artist_name = self.edit.artist_name.clone();
                        if let Some(new_album_artist_name) = &self.edit.album_artist_name {
                            modified_edit.album_artist_name = new_album_artist_name.clone();
                        }
                        modified_edit.edit_all = self.edit.edit_all;
                        modified_edits.push(modified_edit);
                    }

                    let filtered_edits =
                        filter_by_original_album_artist(modified_edits, &self.edit);

                    if !filtered_edits.is_empty() {
                        // Store results and return the first one
                        self.current_results = filtered_edits;
                        self.current_index = 1; // We'll return the first result below
                        self.scrobbles_loaded = true;
                        return Ok(Some(self.current_results[0].clone()));
                    }
                }
                Err(e) => {
                    log::debug!(
                        "Failed to get scrobble data for track '{}': {}",
                        self.track_name,
                        e
                    );
                    self.scrobbles_loaded = true;
                    return Err(e);
                }
            }
            self.scrobbles_loaded = true;
        }

        // No more results
        Ok(None)
    }
}
