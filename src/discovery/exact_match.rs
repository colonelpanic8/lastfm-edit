use super::common::filter_by_original_album_artist;
use crate::{
    AsyncDiscoveryIterator, ExactScrobbleEdit, LastFmEditClientImpl, Result, ScrobbleEdit,
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
            // Perform the lookup
            match self
                .client
                .discover_track_album_exact_match(&self.edit, &self.track_name, &self.album_name)
                .await
            {
                Ok(discovered_edits) => {
                    // Apply album artist filtering
                    let filtered_edits =
                        filter_by_original_album_artist(discovered_edits, &self.edit);
                    self.result = filtered_edits.into_iter().next();
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
