use super::common::filter_by_original_album_artist;
use crate::{
    AsyncDiscoveryIterator, ExactScrobbleEdit, LastFmEditClientImpl, LastFmError, Result,
    ScrobbleEdit,
};
use async_trait::async_trait;

/// Case 1: Exact match discovery (track + album specified)
///
/// This discovers the specific scrobble that matches both the track and album,
/// yielding at most one result.
pub struct ExactMatchDiscovery {
    client: LastFmEditClientImpl,
    edit: ScrobbleEdit,
    track_name: String,
    album_name: String,
    result: Option<ExactScrobbleEdit>,
    completed: bool,
}

impl ExactMatchDiscovery {
    pub fn new(
        client: LastFmEditClientImpl,
        edit: ScrobbleEdit,
        track_name: String,
        album_name: String,
    ) -> Self {
        Self {
            client,
            edit,
            track_name,
            album_name,
            result: None,
            completed: false,
        }
    }
}

#[async_trait(?Send)]
impl AsyncDiscoveryIterator<ExactScrobbleEdit> for ExactMatchDiscovery {
    async fn next(&mut self) -> Result<Option<ExactScrobbleEdit>> {
        if self.completed {
            return Ok(None);
        }

        if self.result.is_none() {
            // Perform the lookup inline (previously discover_track_album_exact_match)
            log::debug!(
                "Looking up missing metadata for track '{}' on album '{}' by '{}'",
                self.track_name,
                self.album_name,
                self.edit.artist_name_original
            );

            match self
                .client
                .load_edit_form_values_internal(&self.track_name, &self.edit.artist_name_original)
                .await
            {
                Ok(all_variations) => {
                    // Filter by album artist first if specified, then find the variation that matches the specific album
                    let filtered_variations =
                        filter_by_original_album_artist(all_variations, &self.edit);

                    if let Some(exact_edit) = filtered_variations
                        .iter()
                        .find(|variation| variation.album_name_original == self.album_name)
                    {
                        // Apply the user's desired changes to this exact variation
                        let mut modified_edit = exact_edit.clone();
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

                        self.result = Some(modified_edit);
                    } else {
                        let album_artist_filter = if self.edit.album_artist_name_original.is_some()
                        {
                            format!(
                                " with album artist '{}'",
                                self.edit.album_artist_name_original.as_ref().unwrap()
                            )
                        } else {
                            String::new()
                        };
                        self.completed = true;
                        return Err(LastFmError::Parse(format!(
                            "Track '{}' not found on album '{}' by '{}'{} in recent scrobbles",
                            self.track_name,
                            self.album_name,
                            self.edit.artist_name_original,
                            album_artist_filter
                        )));
                    }
                }
                Err(e) => {
                    self.completed = true;
                    return Err(e);
                }
            }
        }

        self.completed = true;
        Ok(self.result.take())
    }
}
