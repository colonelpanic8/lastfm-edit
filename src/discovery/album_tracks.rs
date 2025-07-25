use super::common::filter_by_original_album_artist;
use crate::{
    AsyncDiscoveryIterator, AsyncPaginatedIterator, ExactScrobbleEdit, LastFmEditClientImpl,
    Result, ScrobbleEdit,
};
use async_trait::async_trait;

/// Case 3: Album tracks discovery (album specified, track not specified)
///
/// This discovers all tracks in a specific album by iterating through the album's tracks
/// and for each track, loading its scrobbles incrementally. This is now truly incremental
/// like the artist tracks discovery.
pub struct AlbumTracksDiscovery {
    client: LastFmEditClientImpl,
    edit: ScrobbleEdit,
    album_name: String,
    tracks_iterator: crate::AlbumTracksIterator,
    current_track_results: Vec<ExactScrobbleEdit>,
    current_track_index: usize,
}

impl AlbumTracksDiscovery {
    pub fn new(client: LastFmEditClientImpl, edit: ScrobbleEdit, album_name: String) -> Self {
        let tracks_iterator = crate::AlbumTracksIterator::new(
            client.clone(),
            album_name.clone(),
            edit.artist_name_original.clone(),
        );

        Self {
            client,
            edit,
            album_name,
            tracks_iterator,
            current_track_results: Vec::new(),
            current_track_index: 0,
        }
    }
}

#[async_trait(?Send)]
impl AsyncDiscoveryIterator<ExactScrobbleEdit> for AlbumTracksDiscovery {
    async fn next(&mut self) -> Result<Option<ExactScrobbleEdit>> {
        // If we have results from the current track, return the next one
        if self.current_track_index < self.current_track_results.len() {
            let result = self.current_track_results[self.current_track_index].clone();
            self.current_track_index += 1;
            return Ok(Some(result));
        }

        // Get the next track from the iterator
        while let Some(track) = self.tracks_iterator.next().await? {
            log::debug!(
                "Getting scrobble data for track '{}' from album '{}' by '{}'",
                track.name,
                self.album_name,
                self.edit.artist_name_original
            );

            // Get scrobble data for this track
            match self
                .client
                .load_edit_form_values_internal(&track.name, &self.edit.artist_name_original)
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
                        self.current_track_results = filtered_edits;
                        self.current_track_index = 1; // We'll return the first result below
                        return Ok(Some(self.current_track_results[0].clone()));
                    }
                }
                Err(e) => {
                    log::debug!(
                        "Failed to get scrobble data for track '{}': {}",
                        track.name,
                        e
                    );
                    // Continue with next track
                }
            }
        }

        // No more tracks
        Ok(None)
    }
}
