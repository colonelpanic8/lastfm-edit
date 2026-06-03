use super::utils::parse_range;
use lastfm_edit::delete_manifest::{
    execute_delete_targets, read_manifest, target_from_track, write_manifest, DeleteManifestSource,
    DeleteTarget,
};
use lastfm_edit::{LastFmEditClientImpl, Track};
use serde::{Deserialize, Serialize};
use std::io::{self, Write};
use std::path::Path;

/// Events emitted by delete commands (JSON output to stdout)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DeleteEvent {
    /// Found a scrobble that would be deleted (dry run)
    ScrobbleFound {
        index: usize,
        offset: Option<u64>,
        artist: String,
        track: String,
        timestamp: Option<u64>,
    },
    /// Scrobble was deleted
    ScrobbleDeleted {
        index: usize,
        artist: String,
        track: String,
        timestamp: u64,
        success: bool,
        message: Option<String>,
    },
    /// Summary of delete operation
    Summary {
        total_found: usize,
        successful_deletions: usize,
        failed_deletions: usize,
        dry_run: bool,
    },
    /// A delete manifest was written
    ManifestWritten { path: String, total_entries: usize },
}

/// Output a delete event as JSON to stdout
fn output_event(event: &DeleteEvent) {
    if let Ok(json) = serde_json::to_string(event) {
        println!("{json}");
    } else {
        log::error!("Failed to serialize event to JSON");
    }
}

/// Utility function to ask for user confirmation (goes to stderr)
fn ask_for_confirmation(message: &str) -> Result<bool, Box<dyn std::error::Error>> {
    eprint!("{message} (y/N): ");
    io::stderr().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    let response = input.trim().to_lowercase();
    Ok(response == "y" || response == "yes")
}

/// Handle deletion of scrobbles from recent pages
pub async fn handle_delete_recent_pages(
    client: &LastFmEditClientImpl,
    pages_range: &str,
    dry_run: bool,
    manifest_output: Option<&Path>,
    delete_delay_ms: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let (start_page, end_page) = parse_range(pages_range, "pages")?;

    log::info!("Delete recent scrobbles from pages {start_page}-{end_page}");
    if dry_run {
        log::info!("DRY RUN - No actual deletions will be performed");
    }

    let mut scrobbles_to_delete = Vec::new();
    let mut index = 0;

    // Collect scrobbles from the specified pages
    for page in start_page..=end_page {
        log::info!("Processing page {page}...");

        match client.get_recent_scrobbles(page.try_into().unwrap()).await {
            Ok(scrobbles) => {
                if scrobbles.is_empty() {
                    log::info!("No scrobbles found on page {page}");
                    break;
                }

                log::info!("Found {} scrobbles on page {page}", scrobbles.len());

                for scrobble in scrobbles {
                    index += 1;
                    if let Some(timestamp) = scrobble.timestamp {
                        output_event(&DeleteEvent::ScrobbleFound {
                            index,
                            offset: None,
                            artist: scrobble.artist.clone(),
                            track: scrobble.name.clone(),
                            timestamp: Some(timestamp),
                        });

                        scrobbles_to_delete.push(target_from_track(&scrobble, None, timestamp));
                    } else {
                        log::warn!(
                            "Skipping scrobble without timestamp: '{}' by '{}'",
                            scrobble.name,
                            scrobble.artist
                        );
                    }
                }
            }
            Err(e) => {
                log::error!("Error fetching page {page}: {e}");
                break;
            }
        }
    }

    handle_collected_scrobbles(
        client,
        scrobbles_to_delete,
        dry_run,
        manifest_output,
        DeleteManifestSource {
            kind: "recent-pages".to_string(),
            range: Some(pages_range.to_string()),
        },
        delete_delay_ms,
    )
    .await
}

/// Handle deletion of scrobbles from timestamp range
pub async fn handle_delete_timestamp_range(
    client: &LastFmEditClientImpl,
    timestamp_range: &str,
    dry_run: bool,
    manifest_output: Option<&Path>,
    delete_delay_ms: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let (start_ts, end_ts) = parse_range(timestamp_range, "timestamp")?;

    log::info!("Delete scrobbles from timestamp range {start_ts}-{end_ts}");
    if dry_run {
        log::info!("DRY RUN - No actual deletions will be performed");
    }

    let mut scrobbles_to_delete = Vec::new();
    let mut index = 0;
    let max_pages = 20;

    for page in 1..=max_pages {
        log::debug!("Searching page {page} for scrobbles in timestamp range...");

        match client.get_recent_scrobbles(page).await {
            Ok(scrobbles) => {
                if scrobbles.is_empty() {
                    log::info!("No more scrobbles found, stopping search");
                    break;
                }

                for scrobble in scrobbles {
                    if let Some(timestamp) = scrobble.timestamp {
                        if timestamp >= start_ts && timestamp <= end_ts {
                            index += 1;
                            output_event(&DeleteEvent::ScrobbleFound {
                                index,
                                offset: None,
                                artist: scrobble.artist.clone(),
                                track: scrobble.name.clone(),
                                timestamp: Some(timestamp),
                            });

                            scrobbles_to_delete.push(target_from_track(&scrobble, None, timestamp));
                        }
                    }
                }
            }
            Err(e) => {
                log::error!("Error fetching page {page}: {e}");
                break;
            }
        }
    }

    handle_collected_scrobbles(
        client,
        scrobbles_to_delete,
        dry_run,
        manifest_output,
        DeleteManifestSource {
            kind: "timestamp-range".to_string(),
            range: Some(timestamp_range.to_string()),
        },
        delete_delay_ms,
    )
    .await
}

/// Handle deletion of scrobbles by offset from most recent
pub async fn handle_delete_recent_offset(
    client: &LastFmEditClientImpl,
    offset_range: &str,
    dry_run: bool,
    manifest_output: Option<&Path>,
    delete_delay_ms: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let (start_offset, end_offset) = parse_range(offset_range, "offset")?;

    log::info!("Delete scrobbles by offset: {start_offset}-{end_offset} (0-indexed)");
    if dry_run {
        log::info!("DRY RUN - No actual deletions will be performed");
    }

    let mut all_scrobbles: Vec<Track> = Vec::new();
    let mut page = 1;
    let needed_scrobbles = (end_offset + 1) as usize;

    log::info!("Collecting recent scrobbles to reach offset {end_offset}...");

    while all_scrobbles.len() < needed_scrobbles {
        match client.get_recent_scrobbles(page.try_into().unwrap()).await {
            Ok(scrobbles) => {
                if scrobbles.is_empty() {
                    log::info!("No more scrobbles found on page {page}");
                    break;
                }

                log::debug!(
                    "Page {page}: Found {} scrobbles (total: {})",
                    scrobbles.len(),
                    all_scrobbles.len() + scrobbles.len()
                );
                all_scrobbles.extend(scrobbles);
                page += 1;

                if all_scrobbles.len() >= needed_scrobbles {
                    break;
                }
            }
            Err(e) => {
                log::error!("Error fetching page {page}: {e}");
                break;
            }
        }
    }

    if all_scrobbles.len() <= start_offset as usize {
        log::error!(
            "Not enough recent scrobbles found. You have {} scrobbles, but requested offset starts at {} (0-indexed)",
            all_scrobbles.len(),
            start_offset
        );
        output_event(&DeleteEvent::Summary {
            total_found: 0,
            successful_deletions: 0,
            failed_deletions: 0,
            dry_run,
        });
        return Ok(());
    }

    // Extract scrobbles in range
    let actual_end_index = std::cmp::min(end_offset as usize, all_scrobbles.len() - 1);
    let scrobbles_in_range = &all_scrobbles[start_offset as usize..=actual_end_index];

    let mut scrobbles_to_delete = Vec::new();

    for (i, scrobble) in scrobbles_in_range.iter().enumerate() {
        let offset = start_offset + i as u64;
        if let Some(timestamp) = scrobble.timestamp {
            output_event(&DeleteEvent::ScrobbleFound {
                index: i + 1,
                offset: Some(offset),
                artist: scrobble.artist.clone(),
                track: scrobble.name.clone(),
                timestamp: Some(timestamp),
            });

            scrobbles_to_delete.push(target_from_track(scrobble, Some(offset), timestamp));
        } else {
            log::warn!(
                "Skipping scrobble at offset {} without timestamp: '{}' by '{}'",
                offset,
                scrobble.name,
                scrobble.artist
            );
        }
    }

    handle_collected_scrobbles(
        client,
        scrobbles_to_delete,
        dry_run,
        manifest_output,
        DeleteManifestSource {
            kind: "recent-offset".to_string(),
            range: Some(offset_range.to_string()),
        },
        delete_delay_ms,
    )
    .await
}

pub async fn handle_delete_manifest(
    client: &LastFmEditClientImpl,
    manifest_path: &Path,
    dry_run: bool,
    delete_delay_ms: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let manifest = read_manifest(manifest_path)?;
    let scrobbles_to_delete = manifest.targets();

    for (i, scrobble) in scrobbles_to_delete.iter().enumerate() {
        output_event(&DeleteEvent::ScrobbleFound {
            index: i + 1,
            offset: scrobble.offset,
            artist: scrobble.artist.clone(),
            track: scrobble.track.clone(),
            timestamp: Some(scrobble.timestamp),
        });
    }

    execute_deletions(client, scrobbles_to_delete, dry_run, delete_delay_ms).await
}

async fn handle_collected_scrobbles(
    client: &LastFmEditClientImpl,
    scrobbles: Vec<DeleteTarget>,
    dry_run: bool,
    manifest_output: Option<&Path>,
    source: DeleteManifestSource,
    delete_delay_ms: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(path) = manifest_output {
        write_manifest(path, source, &scrobbles)?;
        output_event(&DeleteEvent::Summary {
            total_found: scrobbles.len(),
            successful_deletions: 0,
            failed_deletions: 0,
            dry_run: true,
        });
        return Ok(());
    }

    execute_deletions(client, scrobbles, dry_run, delete_delay_ms).await
}

/// Common deletion execution logic
async fn execute_deletions(
    client: &LastFmEditClientImpl,
    scrobbles: Vec<DeleteTarget>,
    dry_run: bool,
    delete_delay_ms: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    if scrobbles.is_empty() {
        log::info!("No scrobbles with timestamps found to delete");
        output_event(&DeleteEvent::Summary {
            total_found: 0,
            successful_deletions: 0,
            failed_deletions: 0,
            dry_run,
        });
        return Ok(());
    }

    log::info!("Found {} scrobbles to delete", scrobbles.len());

    if dry_run {
        log::info!("DRY RUN - No actual deletions performed");
        log::info!("Use --apply to execute these deletions");
        output_event(&DeleteEvent::Summary {
            total_found: scrobbles.len(),
            successful_deletions: 0,
            failed_deletions: 0,
            dry_run: true,
        });
        return Ok(());
    }

    // Ask for confirmation
    eprintln!();
    eprintln!("About to delete {} scrobble(s):", scrobbles.len());
    if let Some(first) = scrobbles.first() {
        eprintln!("  First: '{}' by '{}'", first.track, first.artist);
    }
    if scrobbles.len() > 1 {
        if let Some(last) = scrobbles.last() {
            eprintln!("  Last:  '{}' by '{}'", last.track, last.artist);
        }
    }

    if !ask_for_confirmation("\nDo you want to proceed with deleting these scrobbles?")? {
        log::info!("Deletion cancelled by user");
        output_event(&DeleteEvent::Summary {
            total_found: scrobbles.len(),
            successful_deletions: 0,
            failed_deletions: 0,
            dry_run: false,
        });
        return Ok(());
    }

    log::info!("Deleting scrobbles...");

    let summary = execute_delete_targets(
        client,
        &scrobbles,
        std::time::Duration::from_millis(delete_delay_ms),
        |index, scrobble, result| {
            log::debug!(
                "Deleting {}/{}: '{}' by '{}'",
                index,
                scrobbles.len(),
                scrobble.track,
                scrobble.artist
            );

            output_event(&DeleteEvent::ScrobbleDeleted {
                index,
                artist: scrobble.artist.clone(),
                track: scrobble.track.clone(),
                timestamp: scrobble.timestamp,
                success: result.success(),
                message: result.message().map(str::to_string),
            });
        },
    )
    .await?;

    output_event(&DeleteEvent::Summary {
        total_found: summary.total_found,
        successful_deletions: summary.successful_deletions,
        failed_deletions: summary.failed_deletions,
        dry_run: false,
    });

    log::info!(
        "Deletion complete: {} successful, {} failed out of {} total",
        summary.successful_deletions,
        summary.failed_deletions,
        summary.total_found
    );

    Ok(())
}
