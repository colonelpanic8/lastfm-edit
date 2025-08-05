use lastfm_edit::{Album, Artist, Track};
use serde::{Deserialize, Serialize};

/// Events emitted by list commands
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ListEvent {
    /// Starting to list items
    Started {
        command: String,
        artist: Option<String>,
        album: Option<String>,
    },
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
    /// Command completed with summary
    Summary {
        command: String,
        total_items: usize,
        artist: Option<String>,
        album: Option<String>,
    },
    /// Error occurred
    Error { message: String },
}

/// Trait for handling list command output
pub trait ListOutputHandler {
    fn handle_event(&mut self, event: ListEvent);
}

/// Human-readable output handler for list commands
pub struct HumanReadableListHandler {
    verbose: bool,
    format: bool,
}

impl HumanReadableListHandler {
    pub fn new(verbose: bool, format: bool) -> Self {
        Self { verbose, format }
    }
}

impl ListOutputHandler for HumanReadableListHandler {
    fn handle_event(&mut self, event: ListEvent) {
        match event {
            ListEvent::Started {
                command,
                artist,
                album,
            } => match command.as_str() {
                "artists" => println!("🎵 Listing artists in your library"),
                "albums" => {
                    if let Some(artist) = artist {
                        println!("🎵 Listing albums for artist: '{artist}'");
                    }
                }
                "tracks" => {
                    if let Some(artist) = artist {
                        println!("🎵 Listing tracks for artist: '{artist}'");
                        println!("   (with complete album information)");
                    }
                }
                "tracks-by-album" => {
                    if let Some(artist) = artist {
                        println!("🎵 Listing tracks by album for artist: '{artist}'");
                    }
                }
                "album-tracks" => {
                    if let (Some(album), Some(artist)) = (album, artist) {
                        println!("🎵 Listing tracks for album: '{album}' by '{artist}'");
                    }
                }
                _ => {}
            },
            ListEvent::ArtistFound { index, artist } => {
                if self.format {
                    if self.verbose {
                        println!("  [{index:3}] {artist} ({} plays)", artist.playcount);
                    } else {
                        println!("  [{index:3}] {artist}");
                    }
                } else if self.verbose {
                    println!("  [{index:3}] {} ({} plays)", artist.name, artist.playcount);
                } else {
                    println!("  [{index:3}] {}", artist.name);
                }
            }
            ListEvent::AlbumFound { index, album } => {
                if self.format {
                    if self.verbose {
                        println!("  [{index:3}] {album} ({} plays)", album.playcount);
                    } else {
                        println!("  [{index:3}] {album}");
                    }
                } else if self.verbose {
                    println!("  [{index:3}] {} ({} plays)", album.name, album.playcount);
                } else {
                    println!("  [{index:3}] {}", album.name);
                }
            }
            ListEvent::TrackFound { index, track } => {
                if self.format {
                    if self.verbose {
                        let album_artist_info = track
                            .album_artist
                            .as_deref()
                            .unwrap_or("Same as track artist");
                        println!("  [{index:3}] {track} ({} plays)", track.playcount);
                        println!("       Album Artist: {album_artist_info}");
                        if let Some(timestamp) = track.timestamp {
                            println!("       Last Played: {timestamp}");
                        }
                    } else {
                        println!("  [{index:3}] {track}");
                    }
                } else if self.verbose {
                    let album_info = track.album.as_deref().unwrap_or("Unknown Album");
                    let album_artist_info = track
                        .album_artist
                        .as_deref()
                        .unwrap_or("Same as track artist");
                    println!("  [{index:3}] {} ({} plays)", track.name, track.playcount);
                    println!("       Album: {album_info}");
                    println!("       Album Artist: {album_artist_info}");
                    if let Some(timestamp) = track.timestamp {
                        println!("       Last Played: {timestamp}");
                    }
                } else {
                    let album_info = track.album.as_deref().unwrap_or("Unknown Album");
                    println!("  [{index:3}] {} [{}]", track.name, album_info);
                }

                if self.verbose {
                    println!();
                }
            }
            ListEvent::AlbumSection { album_index, album } => {
                if self.verbose {
                    println!(
                        "\n📀 Album {}: {} ({} plays)",
                        album_index, album.name, album.playcount
                    );
                } else {
                    println!("\n📀 Album {}: {}", album_index, album.name);
                }
            }
            ListEvent::AlbumTrackFound {
                track_index, track, ..
            } => {
                if self.format {
                    if self.verbose {
                        println!(
                            "    [{:2}] {track} ({} plays)",
                            track_index, track.playcount
                        );
                        if let Some(timestamp) = track.timestamp {
                            println!("         Last Played: {timestamp}");
                        }
                    } else {
                        println!("    [{track_index:2}] {track}");
                    }
                } else if self.verbose {
                    println!(
                        "    [{:2}] {} ({} plays)",
                        track_index, track.name, track.playcount
                    );
                    if let Some(timestamp) = track.timestamp {
                        println!("         Last Played: {timestamp}");
                    }
                } else {
                    println!("    [{:2}] {}", track_index, track.name);
                }
            }
            ListEvent::Summary {
                command,
                total_items,
                artist,
                ..
            } => match command.as_str() {
                "artists" => {
                    if total_items == 0 {
                        println!("  No artists found in your library.");
                    } else {
                        println!(
                            "\nFound {} artist{} in your library",
                            total_items,
                            if total_items == 1 { "" } else { "s" }
                        );
                    }
                }
                "albums" => {
                    if let Some(artist) = artist {
                        if total_items == 0 {
                            println!("  No albums found for this artist.");
                        } else {
                            println!(
                                "\nFound {} album{} for '{artist}'",
                                total_items,
                                if total_items == 1 { "" } else { "s" }
                            );
                        }
                    }
                }
                "tracks" => {
                    if let Some(artist) = artist {
                        if total_items == 0 {
                            println!("  No tracks found for this artist.");
                        } else {
                            println!(
                                "\nFound {} track{} for '{artist}'",
                                total_items,
                                if total_items == 1 { "" } else { "s" }
                            );
                        }
                    }
                }
                "tracks-by-album" => {
                    if let Some(artist) = artist {
                        if total_items == 0 {
                            println!("  No albums found for this artist.");
                        } else {
                            println!(
                                "\nListed {} album{} for '{artist}'",
                                total_items,
                                if total_items == 1 { "" } else { "s" }
                            );
                        }
                    }
                }
                "album-tracks" => {
                    if total_items == 0 {
                        println!("  No tracks found for this album.");
                    }
                }
                _ => {}
            },
            ListEvent::Error { message } => {
                println!("    ❌ Error: {message}");
            }
        }
    }
}

/// JSON output handler for list commands (JSONL format)
pub struct JsonListHandler;

impl JsonListHandler {
    pub fn new() -> Self {
        Self
    }
}

impl ListOutputHandler for JsonListHandler {
    fn handle_event(&mut self, event: ListEvent) {
        // Output each event as a single line of JSON
        if let Ok(json) = serde_json::to_string(&event) {
            println!("{json}");
        } else {
            eprintln!("❌ Failed to serialize event to JSON");
        }
    }
}
