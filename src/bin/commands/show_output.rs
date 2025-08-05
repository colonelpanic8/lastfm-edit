use lastfm_edit::Track;
use serde::{Deserialize, Serialize};

/// Events emitted by show commands
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ShowEvent {
    /// Starting to collect scrobbles for the requested offsets
    Started { offsets: Vec<u64>, max_offset: u64 },
    /// Collecting scrobbles from a specific page
    CollectingPage {
        page: u32,
        scrobbles_found: usize,
        total_collected: usize,
    },
    /// Finished collecting, showing summary
    CollectionComplete {
        total_scrobbles: usize,
        unavailable_offsets: Vec<u64>,
    },
    /// Showing details for a specific scrobble
    ScrobbleDetails { offset: u64, scrobble: Track },
    /// Requested offset is not available (beyond available scrobbles)
    OffsetUnavailable { offset: u64, total_available: usize },
    /// Show command finished
    Finished {
        total_shown: usize,
        unavailable_count: usize,
    },
}

/// Trait for handling show command output
pub trait ShowOutputHandler {
    fn handle_event(&mut self, event: ShowEvent);
}

/// Human-readable output handler for show commands
pub struct HumanReadableShowHandler;

impl HumanReadableShowHandler {
    pub fn new() -> Self {
        Self
    }
}

impl ShowOutputHandler for HumanReadableShowHandler {
    fn handle_event(&mut self, event: ShowEvent) {
        match event {
            ShowEvent::Started {
                offsets,
                max_offset,
            } => {
                println!(
                    "📋 Showing details for scrobbles at offsets: {}",
                    offsets
                        .iter()
                        .map(|n| n.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                println!("\n📄 Collecting recent scrobbles to reach offset {max_offset}...");
            }
            ShowEvent::CollectingPage {
                page,
                scrobbles_found,
                total_collected,
            } => {
                if scrobbles_found > 0 {
                    println!(
                        "  Page {page}: Found {scrobbles_found} scrobbles (total: {total_collected})"
                    );
                } else {
                    println!("  No more scrobbles found on page {page}");
                }
            }
            ShowEvent::CollectionComplete {
                total_scrobbles,
                unavailable_offsets,
            } => {
                println!("\n📊 Total scrobbles collected: {total_scrobbles}");

                if !unavailable_offsets.is_empty() {
                    println!(
                        "\n⚠️  The following offsets are not available (you only have {total_scrobbles} scrobbles):"
                    );
                    for offset in &unavailable_offsets {
                        println!("    - Offset {offset}");
                    }
                    println!();
                }

                println!("🎵 Scrobble Details:");
                println!("{}", "=".repeat(80));
            }
            ShowEvent::ScrobbleDetails { offset, scrobble } => {
                println!(
                    "\n📍 Offset {offset} ({}{})",
                    offset,
                    match offset {
                        0 => "st most recent (index 0)",
                        1 => "nd most recent (index 1)",
                        2 => "rd most recent (index 2)",
                        _ => "th most recent",
                    }
                );

                println!("   🎤 Artist: {}", scrobble.artist);
                println!("   🎵 Track:  {}", scrobble.name);
                println!("   🔢 Play Count: {}", scrobble.playcount);

                if let Some(album) = &scrobble.album {
                    println!("   💿 Album:  {album}");
                } else {
                    println!("   💿 Album:  (no album info)");
                }

                if let Some(album_artist) = &scrobble.album_artist {
                    if album_artist != &scrobble.artist {
                        println!("   👥 Album Artist: {album_artist}");
                    }
                }

                if let Some(timestamp) = scrobble.timestamp {
                    use super::utils::format_timestamp;
                    println!(
                        "   🕐 Timestamp: {} ({})",
                        timestamp,
                        format_timestamp(timestamp)
                    );
                } else {
                    println!("   🕐 Timestamp: (no timestamp)");
                }
            }
            ShowEvent::OffsetUnavailable { .. } => {
                // This is handled in CollectionComplete for better grouping
            }
            ShowEvent::Finished {
                unavailable_count, ..
            } => {
                if unavailable_count > 0 {
                    println!(
                        "\n❌ Could not show {unavailable_count} offset(s) due to insufficient scrobbles"
                    );
                }
                println!("\n✅ Finished showing scrobble details");
            }
        }
    }
}

/// JSON output handler for show commands (JSONL format)
pub struct JsonShowHandler;

impl JsonShowHandler {
    pub fn new() -> Self {
        Self
    }
}

impl ShowOutputHandler for JsonShowHandler {
    fn handle_event(&mut self, event: ShowEvent) {
        // Output each event as a single line of JSON
        if let Ok(json) = serde_json::to_string(&event) {
            println!("{json}");
        } else {
            eprintln!("❌ Failed to serialize event to JSON");
        }
    }
}
