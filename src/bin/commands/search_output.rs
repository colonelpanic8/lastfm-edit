use lastfm_edit::{Album, Artist, Track};
use serde::{Deserialize, Serialize};

/// Events emitted by search commands (JSON output to stdout)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
#[allow(clippy::enum_variant_names)]
pub enum SearchEvent {
    /// Found a track in search results
    TrackFound { index: usize, track: Track },
    /// Found an album in search results
    AlbumFound { index: usize, album: Album },
    /// Found an artist in search results
    ArtistFound { index: usize, artist: Artist },
}

/// Output a search event as JSON to stdout
pub fn output_event(event: &SearchEvent) {
    if let Ok(json) = serde_json::to_string(event) {
        println!("{json}");
    } else {
        log::error!("Failed to serialize event to JSON");
    }
}

/// Log the start of a search command
pub fn log_started(search_type: &str, query: &str, offset: usize) {
    if offset > 0 {
        log::info!(
            "Searching for {} containing '{}' (starting from #{})...",
            search_type,
            query,
            offset + 1
        );
    } else {
        log::info!("Searching for {search_type} containing '{query}'...");
    }
}

/// Log the summary of a search command
pub fn log_summary(total_displayed: usize, offset: usize, limit: usize) {
    log::info!("Displayed {total_displayed} result(s)");

    if offset > 0 {
        log::info!("  (Starting from result #{})", offset + 1);
    }
    if limit > 0 && total_displayed >= limit {
        log::info!("  (Limited to {limit} results)");
    }
}

/// Log no results found
pub fn log_no_results(query: &str) {
    log::info!("No results found matching '{query}'");
}
