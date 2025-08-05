use lastfm_edit::{Album, Track};
use serde::{Deserialize, Serialize};

/// Events emitted by search commands
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SearchEvent {
    /// Starting to search for items
    Started {
        search_type: String, // "tracks" or "albums"
        query: String,
        offset: usize,
        limit: usize,
    },
    /// Found a track in search results
    TrackFound { index: usize, track: Track },
    /// Found an album in search results
    AlbumFound { index: usize, album: Album },
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

/// Human-readable output handler for search commands
pub struct HumanReadableSearchHandler {
    details: bool,
    found_any: bool,
}

impl HumanReadableSearchHandler {
    pub fn new(details: bool) -> Self {
        Self {
            details,
            found_any: false,
        }
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
                    println!(
                        "🔍 Searching for {} containing '{}' (starting from #{})...",
                        search_type,
                        query,
                        offset + 1
                    );
                } else {
                    println!("🔍 Searching for {search_type} containing '{query}'...");
                }
            }
            SearchEvent::TrackFound { index, track } => {
                // Add blank line before first result
                if !self.found_any {
                    self.found_any = true;
                    println!();
                }

                if self.details {
                    println!(
                        "{}. {} - {} (played {} time{})",
                        index,
                        track.artist,
                        track.name,
                        track.playcount,
                        if track.playcount == 1 { "" } else { "s" }
                    );

                    if let Some(album) = &track.album {
                        println!("   Album: {album}");
                    }

                    if let Some(album_artist) = &track.album_artist {
                        if album_artist != &track.artist {
                            println!("   Album Artist: {album_artist}");
                        }
                    }
                    println!(); // Blank line between verbose entries
                } else {
                    println!("{}. {} - {}", index, track.artist, track.name);
                }
            }
            SearchEvent::AlbumFound { index, album } => {
                // Add blank line before first result
                if !self.found_any {
                    self.found_any = true;
                    println!();
                }

                if self.details {
                    println!(
                        "{}. {} - {} (played {} time{})",
                        index,
                        album.artist,
                        album.name,
                        album.playcount,
                        if album.playcount == 1 { "" } else { "s" }
                    );
                    println!(); // Blank line between verbose entries
                } else {
                    println!("{}. {} - {}", index, album.artist, album.name);
                }
            }
            SearchEvent::Summary {
                total_displayed,
                offset,
                limit,
                ..
            } => {
                println!(
                    "✅ Displayed {} result{}",
                    total_displayed,
                    if total_displayed == 1 { "" } else { "s" }
                );

                if offset > 0 {
                    println!("   (Starting from result #{})", offset + 1);
                }
                if limit > 0 && total_displayed >= limit {
                    println!("   (Limited to {limit} results)");
                }
            }
            SearchEvent::NoResults { query, .. } => {
                println!("❌ No results found matching '{query}'");
            }
            SearchEvent::Finished { .. } => {
                // Nothing needed for human-readable output - summary already printed
            }
        }
    }
}

/// JSON output handler for search commands (JSONL format)
pub struct JsonSearchHandler;

impl JsonSearchHandler {
    pub fn new() -> Self {
        Self
    }
}

impl SearchOutputHandler for JsonSearchHandler {
    fn handle_event(&mut self, event: SearchEvent) {
        // Output each event as a single line of JSON
        if let Ok(json) = serde_json::to_string(&event) {
            println!("{json}");
        } else {
            eprintln!("❌ Failed to serialize event to JSON");
        }
    }
}
