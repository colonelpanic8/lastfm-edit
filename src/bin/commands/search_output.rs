use lastfm_edit::{Album, Artist, Track};
use serde::{Deserialize, Serialize};

/// Events emitted by search commands
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SearchEvent {
    /// Starting to search for items
    Started {
        search_type: String, // "tracks", "albums", or "artists"
        query: String,
        offset: usize,
        limit: usize,
    },
    /// Found a track in search results
    TrackFound { index: usize, track: Track },
    /// Found an album in search results
    AlbumFound { index: usize, album: Album },
    /// Found an artist in search results
    ArtistFound { index: usize, artist: Artist },
    /// Search completed with summary
    Summary {
        search_type: String,
        query: String,
        total_displayed: usize,
        offset: usize,
        limit: usize,
    },
    /// No results found
    NoResults { search_type: String, query: String },
    /// Search command finished
    Finished { search_type: String, query: String },
}

/// Trait for handling search command output
pub trait SearchOutputHandler {
    fn handle_event(&mut self, event: SearchEvent);
}

/// Default output handler for search commands
/// Status messages go to stderr, results go to stdout as JSON (one per line)
pub struct HumanReadableSearchHandler;

impl HumanReadableSearchHandler {
    pub fn new(_details: bool) -> Self {
        Self
    }
}

impl SearchOutputHandler for HumanReadableSearchHandler {
    fn handle_event(&mut self, event: SearchEvent) {
        match event {
            SearchEvent::Started {
                search_type,
                query,
                offset,
                ..
            } => {
                if offset > 0 {
                    eprintln!(
                        "Searching for {} containing '{}' (starting from #{})...",
                        search_type,
                        query,
                        offset + 1
                    );
                } else {
                    eprintln!("Searching for {search_type} containing '{query}'...");
                }
            }
            SearchEvent::TrackFound { track, .. } => {
                if let Ok(json) = serde_json::to_string(&track) {
                    println!("{json}");
                }
            }
            SearchEvent::AlbumFound { album, .. } => {
                if let Ok(json) = serde_json::to_string(&album) {
                    println!("{json}");
                }
            }
            SearchEvent::ArtistFound { artist, .. } => {
                if let Ok(json) = serde_json::to_string(&artist) {
                    println!("{json}");
                }
            }
            SearchEvent::Summary {
                total_displayed,
                offset,
                limit,
                ..
            } => {
                eprintln!("Displayed {total_displayed} result(s)");

                if offset > 0 {
                    eprintln!("  (Starting from result #{})", offset + 1);
                }
                if limit > 0 && total_displayed >= limit {
                    eprintln!("  (Limited to {limit} results)");
                }
            }
            SearchEvent::NoResults { query, .. } => {
                eprintln!("No results found matching '{query}'");
            }
            SearchEvent::Finished { .. } => {
                // Nothing needed - summary already printed
            }
        }
    }
}

/// JSON output handler for search commands (full event stream)
/// Outputs all events as JSON to stderr, results to stdout
pub struct JsonSearchHandler;

impl JsonSearchHandler {
    pub fn new() -> Self {
        Self
    }
}

impl SearchOutputHandler for JsonSearchHandler {
    fn handle_event(&mut self, event: SearchEvent) {
        match &event {
            // Status messages go to stderr as JSON
            SearchEvent::Started { .. }
            | SearchEvent::Summary { .. }
            | SearchEvent::NoResults { .. }
            | SearchEvent::Finished { .. } => {
                if let Ok(json) = serde_json::to_string(&event) {
                    eprintln!("{json}");
                }
            }
            // Results go to stdout (just the item)
            SearchEvent::TrackFound { track, .. } => {
                if let Ok(json) = serde_json::to_string(track) {
                    println!("{json}");
                }
            }
            SearchEvent::AlbumFound { album, .. } => {
                if let Ok(json) = serde_json::to_string(album) {
                    println!("{json}");
                }
            }
            SearchEvent::ArtistFound { artist, .. } => {
                if let Ok(json) = serde_json::to_string(artist) {
                    println!("{json}");
                }
            }
        }
    }
}
