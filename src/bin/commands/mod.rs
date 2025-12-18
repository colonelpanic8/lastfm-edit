pub mod delete;
pub mod edit;
pub mod list;
pub mod list_output;
pub mod search;
pub mod search_output;
pub mod show;
pub mod show_output;
pub mod utils;

use clap::{arg, Subcommand, ValueEnum};
use lastfm_edit::LastFmEditClientImpl;

#[derive(ValueEnum, Clone)]
pub enum SearchType {
    /// Search for tracks
    Tracks,
    /// Search for albums
    Albums,
    /// Search for artists
    Artists,
}

#[derive(Subcommand)]
pub enum ListCommands {
    /// List all artists in your library
    ///
    /// This command lists all artists in your Last.fm library, sorted by play count
    /// (highest first). Shows artist names and scrobble counts.
    ///
    /// Usage examples:
    /// # List all artists
    /// lastfm-edit list artists
    ///
    /// # List first 20 artists with play counts
    /// lastfm-edit list artists --limit 20 --details
    ///
    /// # List artists with formatted display
    /// lastfm-edit list artists --format
    Artists {
        /// Maximum number of artists to show (0 for no limit)
        #[arg(long, default_value = "0")]
        limit: usize,

        /// Show additional details like play counts
        #[arg(long)]
        details: bool,

        /// Show formatted output
        #[arg(long)]
        format: bool,
    },

    /// List albums for an artist
    ///
    /// This command lists all albums in your library for a specified artist.
    /// The albums are sorted by play count (highest first).
    ///
    /// Usage examples:
    /// # List all albums for The Beatles
    /// lastfm-edit list albums "The Beatles"
    ///
    /// # List first 10 albums with play counts
    /// lastfm-edit list albums "Radiohead" --limit 10 --details
    ///
    /// # List albums with formatted display (Artist - Album Name)
    /// lastfm-edit list albums "The Beatles" --format
    Albums {
        /// Artist name
        artist: String,

        /// Maximum number of albums to show (0 for no limit)
        #[arg(long, default_value = "0")]
        limit: usize,

        /// Show additional details like play counts
        #[arg(long)]
        details: bool,

        /// Show formatted output (Artist - Album/Track Name [Album Name])
        #[arg(long)]
        format: bool,
    },

    /// List all tracks for an artist with album information (album-based iteration)
    ///
    /// This command lists all tracks in your library for a specified artist,
    /// with complete album information included. Unlike tracks-by-album, this
    /// shows tracks in a flat list with album details for each track.
    /// Note: This uses album-based iteration, so tracks without album metadata may be missed.
    ///
    /// Usage examples:
    /// # List all tracks for The Beatles with album info
    /// lastfm-edit list tracks "The Beatles"
    ///
    /// # List first 20 tracks with play counts and details
    /// lastfm-edit list tracks "Radiohead" --limit 20 --details
    ///
    /// # List tracks with formatted display (Artist - Track Name [Album Name])
    /// lastfm-edit list tracks "The Beatles" --format
    Tracks {
        /// Artist name
        artist: String,

        /// Maximum number of tracks to show (0 for no limit)
        #[arg(long, default_value = "0")]
        limit: usize,

        /// Show additional details like play counts and album artist
        #[arg(long)]
        details: bool,

        /// Show formatted output (Artist - Track Name [Album Name])
        #[arg(long)]
        format: bool,
    },

    /// List all tracks for an artist using direct track iteration
    ///
    /// This command lists all tracks in your library for a specified artist using
    /// direct track iteration. This approach finds ALL tracks, including those
    /// without album metadata (singles, B-sides, etc.) that may be missed by the
    /// regular tracks command.
    ///
    /// Usage examples:
    /// # List all tracks including those without albums
    /// lastfm-edit list tracks-direct "The Beatles"
    ///
    /// # Compare with regular tracks command to find missing tracks
    /// lastfm-edit list tracks-direct "The Beatles" --limit 20 --details
    TracksDirect {
        /// Artist name
        artist: String,

        /// Maximum number of tracks to show (0 for no limit)
        #[arg(long, default_value = "0")]
        limit: usize,

        /// Show additional details like play counts and album artist
        #[arg(long)]
        details: bool,

        /// Show formatted output (Artist - Track Name [Album Name])
        #[arg(long)]
        format: bool,
    },

    /// List tracks organized by album for an artist
    ///
    /// This command lists all tracks in your library for a specified artist,
    /// organized by album. For each album, it shows all tracks from that album.
    ///
    /// Usage examples:
    /// # List all tracks by album for The Beatles
    /// lastfm-edit list tracks-by-album "The Beatles"
    ///
    /// # List tracks for first 5 albums with play counts
    /// lastfm-edit list tracks-by-album "Pink Floyd" --limit 5 --details
    ///
    /// # List tracks with formatted display (Artist - Track Name [Album Name])
    /// lastfm-edit list tracks-by-album "The Beatles" --format
    TracksByAlbum {
        /// Artist name
        artist: String,

        /// Maximum number of albums to show (0 for no limit)
        #[arg(long, default_value = "0")]
        limit: usize,

        /// Show additional details like play counts
        #[arg(long)]
        details: bool,

        /// Show formatted output (Artist - Album/Track Name [Album Name])
        #[arg(long)]
        format: bool,
    },

    /// List tracks for a specific album
    ///
    /// This command lists all tracks for a specific album by a specific artist.
    /// This is useful for albums with special characters like slashes in their names.
    ///
    /// Usage examples:
    /// # List all tracks for AC/DC's "Back in Black" album
    /// lastfm-edit list album-tracks "Back in Black" "AC/DC"
    ///
    /// # List tracks with details and formatted output
    /// lastfm-edit list album-tracks "The Dark Side of the Moon" "Pink Floyd" --details --format
    AlbumTracks {
        /// Album name
        album: String,

        /// Artist name
        artist: String,

        /// Show additional details like play counts
        #[arg(long)]
        details: bool,

        /// Show formatted output (Artist - Track Name [Album Name])
        #[arg(long)]
        format: bool,
    },
}

#[derive(Subcommand)]
pub enum Commands {
    /// Edit scrobble metadata
    ///
    /// This command allows you to edit scrobble metadata by specifying what to search for
    /// and what to change it to. You can specify any combination of fields to search for,
    /// and any combination of new values to change them to.
    ///
    /// Usage examples:
    /// # Discover variations for an artist (dry run by default)
    /// lastfm-edit edit --artist "Jimi Hendrix"
    ///
    /// # Discover variations with optional track name
    /// lastfm-edit edit --artist "Radiohead" --track "Creep"
    ///
    /// # Actually apply edits (change artist name)
    /// lastfm-edit edit --artist "The Beatles" --new-artist "Beatles, The" --apply
    ///
    /// # Change track name for specific track
    /// lastfm-edit edit --artist "Jimi Hendrix" --track "Lover Man" --new-track "Lover Man (Live)" --apply
    Edit {
        /// Artist name (required)
        #[arg(long)]
        artist: String,

        /// Track name (optional)
        #[arg(long)]
        track: Option<String>,

        /// Album name (optional)
        #[arg(long)]
        album: Option<String>,

        /// Album artist name (optional)
        #[arg(long)]
        album_artist: Option<String>,

        /// New track name (optional)
        #[arg(long)]
        new_track: Option<String>,

        /// New album name (optional)
        #[arg(long)]
        new_album: Option<String>,

        /// New artist name (optional)
        #[arg(long)]
        new_artist: Option<String>,

        /// New album artist name (optional)
        #[arg(long)]
        new_album_artist: Option<String>,

        /// Timestamp for specific scrobble (optional)
        #[arg(long)]
        timestamp: Option<u64>,

        /// Disable editing all instances (edit only specific scrobble, defaults to editing all)
        #[arg(long)]
        no_edit_all: bool,

        /// Actually apply the edits (default is dry-run mode)
        #[arg(long)]
        apply: bool,

        /// Perform a dry run without actually submitting edits (default behavior)
        #[arg(long)]
        dry_run: bool,
    },
    /// Delete scrobbles in a range
    ///
    /// This command allows you to delete scrobbles from your library. You can specify
    /// timestamp ranges, delete recent scrobbles from specific pages, or use offsets
    /// from the most recent scrobble.
    ///
    /// Usage examples:
    /// # Show recent scrobbles that would be deleted (dry run)
    /// lastfm-edit delete --recent-pages 1-3
    ///
    /// # Delete scrobbles from timestamp range
    /// lastfm-edit delete --timestamp-range 1640995200-1641000000 --apply
    ///
    /// # Delete scrobbles by offset from most recent (0-indexed)
    /// lastfm-edit delete --recent-offset 0-4 --apply
    Delete {
        /// Delete scrobbles from recent pages (format: start-end, 0-indexed)
        #[arg(long, conflicts_with_all = ["timestamp_range", "recent_offset"])]
        recent_pages: Option<String>,

        /// Delete scrobbles from timestamp range (format: start_ts-end_ts)
        #[arg(long, conflicts_with_all = ["recent_pages", "recent_offset"])]
        timestamp_range: Option<String>,

        /// Delete scrobbles by offset from most recent (format: start-end, 0-indexed)
        #[arg(long, conflicts_with_all = ["recent_pages", "timestamp_range"])]
        recent_offset: Option<String>,

        /// Actually perform the deletions (default is dry-run mode)
        #[arg(long)]
        apply: bool,

        /// Perform a dry run without actually deleting (default behavior)
        #[arg(long)]
        dry_run: bool,
    },
    /// Search tracks, albums, and artists in your library
    ///
    /// This command allows you to search through your Last.fm library for tracks, albums,
    /// or artists that match a specific query. You can limit the number of results and
    /// specify the type of search.
    ///
    /// Usage examples:
    /// # Search for tracks containing "remaster"
    /// lastfm-edit search tracks "remaster"
    ///
    /// # Search for first 20 albums containing "deluxe"
    /// lastfm-edit search albums "deluxe" --limit 20
    ///
    /// # Search for artists matching "radio"
    /// lastfm-edit search artists "radio" --limit 10
    ///
    /// # Search for tracks with unlimited results
    /// lastfm-edit search tracks "live" --limit 0
    ///
    /// # Skip first 10 results and show next 20
    /// lastfm-edit search tracks "live" --offset 10 --limit 20
    Search {
        /// Type of search: tracks, albums, or artists
        #[arg(value_enum)]
        search_type: SearchType,

        /// Search query
        query: String,

        /// Maximum number of results to show (0 for no limit)
        #[arg(long, default_value = "50")]
        limit: usize,

        /// Number of results to skip from the beginning (0-indexed)
        #[arg(long, default_value = "0")]
        offset: usize,

        /// Show additional details like play counts
        #[arg(long)]
        details: bool,
    },

    /// Show scrobble details for specific offsets
    ///
    /// This command displays detailed information for scrobbles at the specified
    /// offsets from your most recent scrobbles.
    ///
    /// Usage examples:
    /// # Show details for the most recent scrobble (offset 0)
    /// lastfm-edit show 0
    ///
    /// # Show details for multiple scrobbles (0-indexed)
    /// lastfm-edit show 0 1 2 5 10
    Show {
        /// Offsets of scrobbles to show (0-indexed, 0 = most recent)
        offsets: Vec<u64>,
    },

    /// List artists, albums, and tracks from your library
    ///
    /// This command allows you to browse your Last.fm library by listing artists,
    /// albums, and tracks.
    ///
    /// Usage examples:
    /// # List all artists in your library
    /// lastfm-edit list artists --limit 20 --details
    ///
    /// # List all albums for The Beatles
    /// lastfm-edit list albums "The Beatles"
    ///
    /// # List all tracks with album information
    /// lastfm-edit list tracks "Radiohead" --limit 20 --details
    ///
    /// # List tracks organized by album
    /// lastfm-edit list tracks-by-album "Pink Floyd" --limit 5 --details
    List {
        #[command(subcommand)]
        command: ListCommands,
    },
}

/// Execute the appropriate command handler based on the parsed command
pub async fn execute_command(
    command: Commands,
    client: &LastFmEditClientImpl,
    json_output: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        Commands::Edit {
            artist,
            track,
            album,
            album_artist,
            new_track,
            new_album,
            new_artist,
            new_album_artist,
            timestamp,
            no_edit_all,
            apply,
            dry_run,
        } => {
            // Determine whether this is a dry run or actual edit
            let is_dry_run = dry_run || !apply;

            let edit = edit::create_scrobble_edit_from_args(
                &artist,
                track.as_deref(),
                album.as_deref(),
                album_artist.as_deref(),
                new_track.as_deref(),
                new_album.as_deref(),
                new_artist.as_deref(),
                new_album_artist.as_deref(),
                timestamp,
                !no_edit_all, // edit_all is true by default, false only if --no-edit-all is provided
            );

            edit::handle_edit_command(client, &edit, is_dry_run).await
        }

        Commands::Delete {
            recent_pages,
            timestamp_range,
            recent_offset,
            apply,
            dry_run,
        } => {
            // Determine whether this is a dry run or actual deletion
            let is_dry_run = dry_run || !apply;

            if let Some(pages_range) = recent_pages {
                delete::handle_delete_recent_pages(client, &pages_range, is_dry_run).await
            } else if let Some(ts_range) = timestamp_range {
                delete::handle_delete_timestamp_range(client, &ts_range, is_dry_run).await
            } else if let Some(offset_range) = recent_offset {
                delete::handle_delete_recent_offset(client, &offset_range, is_dry_run).await
            } else {
                Err(
                    "Must specify one of: --recent-pages, --timestamp-range, or --recent-offset"
                        .into(),
                )
            }
        }

        Commands::Search {
            search_type,
            query,
            limit,
            offset,
            details,
        } => {
            search::handle_search_command(
                client,
                search_type,
                &query,
                limit,
                offset,
                details,
                json_output,
            )
            .await
        }

        Commands::Show { offsets } => {
            if offsets.is_empty() {
                return Err("Must specify at least one offset to show".into());
            }

            show::handle_show_scrobbles(client, &offsets, json_output).await
        }

        Commands::List { command } => match command {
            ListCommands::Artists {
                limit,
                details,
                format,
            } => list::handle_list_artists(client, limit, details, format, json_output).await,
            ListCommands::Albums {
                artist,
                limit,
                details,
                format,
            } => {
                list::handle_list_albums(client, &artist, limit, details, format, json_output).await
            }
            ListCommands::Tracks {
                artist,
                limit,
                details,
                format,
            } => {
                list::handle_list_tracks(client, &artist, limit, details, format, json_output).await
            }
            ListCommands::TracksDirect {
                artist,
                limit,
                details,
                format,
            } => {
                list::handle_list_tracks_direct(
                    client,
                    &artist,
                    limit,
                    details,
                    format,
                    json_output,
                )
                .await
            }
            ListCommands::TracksByAlbum {
                artist,
                limit,
                details,
                format,
            } => {
                list::handle_list_tracks_by_album(
                    client,
                    &artist,
                    limit,
                    details,
                    format,
                    json_output,
                )
                .await
            }
            ListCommands::AlbumTracks {
                album,
                artist,
                details,
                format,
            } => {
                list::handle_list_album_tracks(
                    client,
                    &album,
                    &artist,
                    details,
                    format,
                    json_output,
                )
                .await
            }
        },
    }
}
