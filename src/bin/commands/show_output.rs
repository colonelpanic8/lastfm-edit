use lastfm_edit::Track;
use serde::{Deserialize, Serialize};

/// Events emitted by show commands (JSON output to stdout)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ShowEvent {
    /// Showing details for a specific scrobble
    ScrobbleDetails { offset: u64, scrobble: Track },
    /// Requested offset is not available (beyond available scrobbles)
    OffsetUnavailable { offset: u64, total_available: usize },
}

/// Output a show event as JSON to stdout
pub fn output_event(event: &ShowEvent) {
    if let Ok(json) = serde_json::to_string(event) {
        println!("{json}");
    } else {
        log::error!("Failed to serialize event to JSON");
    }
}

/// Log the start of a show command
pub fn log_started(offsets: &[u64], max_offset: u64) {
    let offsets_str = offsets
        .iter()
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    log::info!("Showing details for scrobbles at offsets: {offsets_str}");
    log::info!("Collecting recent scrobbles to reach offset {max_offset}...");
}

/// Log page collection progress
pub fn log_collecting_page(page: u32, scrobbles_found: usize, total_collected: usize) {
    if scrobbles_found > 0 {
        log::debug!("Page {page}: Found {scrobbles_found} scrobbles (total: {total_collected})");
    } else {
        log::debug!("No more scrobbles found on page {page}");
    }
}

/// Log collection complete
pub fn log_collection_complete(total_scrobbles: usize, unavailable_offsets: &[u64]) {
    log::info!("Total scrobbles collected: {total_scrobbles}");

    if !unavailable_offsets.is_empty() {
        log::warn!(
            "The following offsets are not available (you only have {total_scrobbles} scrobbles): {unavailable_offsets:?}"
        );
    }
}

/// Log show finished
pub fn log_finished(total_shown: usize, unavailable_count: usize) {
    if unavailable_count > 0 {
        log::warn!("Could not show {unavailable_count} offset(s) due to insufficient scrobbles");
    }
    log::info!("Finished showing {total_shown} scrobble details");
}
