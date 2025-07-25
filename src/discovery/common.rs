use crate::{ExactScrobbleEdit, ScrobbleEdit};

/// Filter discovered edits based on original album artist if specified
///
/// When album_artist_name_original is specified in the ScrobbleEdit, we only want
/// to return ExactScrobbleEdits that match that original album artist value.
/// This prevents implicit fan-out over different album artists.
pub fn filter_by_original_album_artist(
    discovered_edits: Vec<ExactScrobbleEdit>,
    edit: &ScrobbleEdit,
) -> Vec<ExactScrobbleEdit> {
    if let Some(target_album_artist) = &edit.album_artist_name_original {
        log::debug!(
            "Filtering {} discovered edits to only include album artist '{}'",
            discovered_edits.len(),
            target_album_artist
        );

        let filtered: Vec<ExactScrobbleEdit> = discovered_edits
            .into_iter()
            .filter(|scrobble| scrobble.album_artist_name_original == *target_album_artist)
            .collect();

        log::debug!(
            "After filtering by album artist '{}': {} edits remain",
            target_album_artist,
            filtered.len()
        );

        filtered
    } else {
        discovered_edits
    }
}
