use lastfm_edit::{ExactScrobbleEdit, LastFmEditClient, LastFmEditClientImpl, ScrobbleEdit};
use serde::{Deserialize, Serialize};

/// Events emitted by edit commands (JSON output to stdout)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum EditEvent {
    /// Found a scrobble variation to edit
    VariationFound {
        index: usize,
        variation: ExactScrobbleEdit,
    },
    /// Edit was applied successfully
    EditApplied {
        index: usize,
        variation: ExactScrobbleEdit,
        success: bool,
        message: Option<String>,
    },
    /// Dry run - would have edited this
    DryRunVariation {
        index: usize,
        variation: ExactScrobbleEdit,
    },
    /// Summary of edit operation
    Summary {
        total_found: usize,
        successful_edits: usize,
        failed_edits: usize,
        dry_run: bool,
    },
}

/// Output an edit event as JSON to stdout
fn output_event(event: &EditEvent) {
    if let Ok(json) = serde_json::to_string(event) {
        println!("{json}");
    } else {
        log::error!("Failed to serialize event to JSON");
    }
}

/// Create a ScrobbleEdit from command line arguments
#[allow(clippy::too_many_arguments)]
pub fn create_scrobble_edit_from_args(
    artist: &str,
    track: Option<&str>,
    album: Option<&str>,
    album_artist: Option<&str>,
    new_track: Option<&str>,
    new_album: Option<&str>,
    new_artist: Option<&str>,
    new_album_artist: Option<&str>,
    timestamp: Option<u64>,
    edit_all: bool,
) -> ScrobbleEdit {
    // Determine the new artist name (use provided new_artist or original artist)
    let new_artist = new_artist.unwrap_or(artist);

    ScrobbleEdit::new(
        track.map(|s| s.to_string()),
        album.map(|s| s.to_string()),
        artist.to_string(),
        album_artist.map(|s| s.to_string()),
        new_track.map(|s| s.to_string()),
        new_album.map(|s| s.to_string()),
        new_artist.to_string(),
        new_album_artist.map(|s| s.to_string()),
        timestamp,
        edit_all,
    )
}

/// Handle the edit command
pub async fn handle_edit_command(
    client: &LastFmEditClientImpl,
    edit: &ScrobbleEdit,
    dry_run: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    log::info!("Edit request: {edit:?}");
    discover_and_handle_edits(client, edit, dry_run).await
}

async fn discover_and_handle_edits(
    client: &LastFmEditClientImpl,
    edit: &ScrobbleEdit,
    dry_run: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    log::info!("Discovering scrobble edit variations...");

    let mut discovery_iterator = client.discover_scrobbles(edit.clone());
    let mut count = 0;
    let mut successful_edits = 0;
    let mut failed_edits = 0;

    while let Some(discovered_edit) = discovery_iterator.next().await? {
        count += 1;

        log::debug!(
            "Found variation {}: '{}' by '{}' on '{}'",
            count,
            discovered_edit.track_name_original,
            discovered_edit.artist_name_original,
            discovered_edit.album_name_original
        );

        if dry_run {
            output_event(&EditEvent::DryRunVariation {
                index: count,
                variation: discovered_edit.clone(),
            });
        } else {
            log::info!("Applying edit {count}...");

            // Apply the user's changes to create the final exact edit
            let mut final_edit = discovered_edit.clone();
            if let Some(new_track_name) = &edit.track_name {
                final_edit.track_name = new_track_name.clone();
            }
            if let Some(new_album_name) = &edit.album_name {
                final_edit.album_name = new_album_name.clone();
            }
            final_edit.artist_name = edit.artist_name.clone();
            if let Some(new_album_artist_name) = &edit.album_artist_name {
                final_edit.album_artist_name = new_album_artist_name.clone();
            }
            final_edit.edit_all = edit.edit_all;

            match client.edit_scrobble_single(&final_edit, 3).await {
                Ok(response) => {
                    let success = response.all_successful();
                    let message = if success {
                        None
                    } else {
                        Some(response.summary_message())
                    };

                    if success {
                        successful_edits += 1;
                        log::info!("Edit {count} applied successfully");
                    } else {
                        failed_edits += 1;
                        log::warn!("Edit {} failed: {}", count, response.summary_message());
                    }

                    output_event(&EditEvent::EditApplied {
                        index: count,
                        variation: discovered_edit.clone(),
                        success,
                        message,
                    });
                }
                Err(e) => {
                    failed_edits += 1;
                    log::error!("Error applying edit {count}: {e}");

                    output_event(&EditEvent::EditApplied {
                        index: count,
                        variation: discovered_edit.clone(),
                        success: false,
                        message: Some(e.to_string()),
                    });
                }
            }
        }
    }

    if count == 0 {
        log::info!("No matching scrobbles found");
        log::info!("This might mean:");
        log::info!("  - The specified metadata is not in your recent scrobbles");
        log::info!("  - The names don't match exactly");
        log::info!("  - There's a network or parsing issue");
    }

    output_event(&EditEvent::Summary {
        total_found: count,
        successful_edits,
        failed_edits,
        dry_run,
    });

    if dry_run {
        log::info!("DRY RUN - Found {count} variation(s), no edits performed");
        log::info!("Use --apply to execute these edits");
    } else {
        log::info!(
            "Edit complete: {successful_edits} successful, {failed_edits} failed out of {count} total"
        );
    }

    Ok(())
}
