use lastfm_edit::{Album, Artist, Track};
use serde::{Deserialize, Serialize};

/// Events emitted by list commands (JSON output to stdout)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ListEvent {
    /// Found an artist
    ArtistFound { index: usize, artist: Artist },
    /// Found an album
    AlbumFound { index: usize, album: Album },
    /// Found a track
    TrackFound { index: usize, track: Track },
    /// Starting a new album section (for tracks-by-album)
    AlbumSection { album_index: usize, album: Album },
    /// Found a track within an album section
    AlbumTrackFound {
        album_index: usize,
        track_index: usize,
        track: Track,
    },
}

/// Output a list event as JSON to stdout
pub fn output_event(event: &ListEvent) {
    if let Ok(json) = serde_json::to_string(event) {
        println!("{json}");
    } else {
        log::error!("Failed to serialize event to JSON");
    }
}

/// Log the start of a list command
pub fn log_started(command: &str, artist: Option<&str>, album: Option<&str>) {
    match command {
        "artists" => log::info!("Listing artists in your library"),
        "albums" => {
            if let Some(artist) = artist {
                log::info!("Listing albums for artist: '{artist}'");
            }
        }
        "tracks" | "tracks-direct" => {
            if let Some(artist) = artist {
                log::info!("Listing tracks for artist: '{artist}'");
            }
        }
        "tracks-by-album" => {
            if let Some(artist) = artist {
                log::info!("Listing tracks by album for artist: '{artist}'");
            }
        }
        "album-tracks" => {
            if let (Some(album), Some(artist)) = (album, artist) {
                log::info!("Listing tracks for album: '{album}' by '{artist}'");
            }
        }
        _ => {}
    }
}

/// Log the summary of a list command
pub fn log_summary(command: &str, total_items: usize, artist: Option<&str>) {
    match command {
        "artists" => {
            if total_items == 0 {
                log::info!("No artists found in your library");
            } else {
                log::info!(
                    "Found {} artist{} in your library",
                    total_items,
                    if total_items == 1 { "" } else { "s" }
                );
            }
        }
        "albums" => {
            if let Some(artist) = artist {
                if total_items == 0 {
                    log::info!("No albums found for '{artist}'");
                } else {
                    log::info!(
                        "Found {} album{} for '{artist}'",
                        total_items,
                        if total_items == 1 { "" } else { "s" }
                    );
                }
            }
        }
        "tracks" | "tracks-direct" => {
            if let Some(artist) = artist {
                if total_items == 0 {
                    log::info!("No tracks found for '{artist}'");
                } else {
                    log::info!(
                        "Found {} track{} for '{artist}'",
                        total_items,
                        if total_items == 1 { "" } else { "s" }
                    );
                }
            }
        }
        "tracks-by-album" => {
            if let Some(artist) = artist {
                if total_items == 0 {
                    log::info!("No albums found for '{artist}'");
                } else {
                    log::info!(
                        "Listed {} album{} for '{artist}'",
                        total_items,
                        if total_items == 1 { "" } else { "s" }
                    );
                }
            }
        }
        "album-tracks" => {
            if total_items == 0 {
                log::info!("No tracks found for this album");
            } else {
                log::info!(
                    "Found {} track{} for album",
                    total_items,
                    if total_items == 1 { "" } else { "s" }
                );
            }
        }
        _ => {}
    }
}
