use clap::Parser;
use lastfm_edit::{LastFmEditClientImpl, ScrobbleEdit};
use std::env;

/// Example demonstrating the discover_scrobble_edit_variations functionality.
///
/// This example allows you to specify all fields of a ScrobbleEdit (optionally),
/// except for artist which is required. It then discovers the variations and prints them out.
///
/// Usage examples:
/// # Discover variations for an artist
/// direnv exec . cargo run --example enrich_metadata_example --artist "Jimi Hendrix"
///
/// # Discover variations with optional track name
/// direnv exec . cargo run --example enrich_metadata_example --artist "Radiohead" --track "Creep"
///
/// # Discover variations with multiple fields
/// direnv exec . cargo run --example enrich_metadata_example --artist "The Beatles" --album "Abbey Road" --track "Come Together"
///
/// # Show detailed debug information
/// direnv exec . cargo run --example enrich_metadata_example --artist "Radiohead" --verbose
#[derive(Parser)]
#[command(
    name = "enrich_metadata_example",
    about = "Discover scrobble edit variations for specified metadata",
    long_about = None
)]
struct Cli {
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

    /// Whether to edit all instances (optional, defaults to false)
    #[arg(long)]
    edit_all: bool,

    /// Show detailed debug information
    #[arg(long)]
    verbose: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    if cli.verbose {
        env_logger::Builder::from_default_env()
            .filter_level(log::LevelFilter::Debug)
            .init();
    } else {
        env_logger::Builder::from_default_env()
            .filter_level(log::LevelFilter::Info)
            .init();
    }

    let username = env::var("LASTFM_EDIT_USERNAME")
        .expect("LASTFM_EDIT_USERNAME environment variable not set");
    let password = env::var("LASTFM_EDIT_PASSWORD")
        .expect("LASTFM_EDIT_PASSWORD environment variable not set");

    // Create and authenticate client
    let http_client = http_client::native::NativeClient::new();
    let client = LastFmEditClientImpl::new(Box::new(http_client));

    println!("🔐 Logging in to Last.fm...");
    client.login(&username, &password).await?;
    println!("✅ Successfully authenticated as {}", client.username());

    // Create ScrobbleEdit based on provided arguments
    let edit = create_scrobble_edit(&cli);

    // Show what we're searching for
    print_search_criteria(&cli);

    // Discover and print variations
    discover_and_print_variations(&client, &edit).await?;

    Ok(())
}

fn create_scrobble_edit(cli: &Cli) -> ScrobbleEdit {
    // Determine the new artist name (use provided new_artist or original artist)
    let new_artist = cli.new_artist.as_deref().unwrap_or(&cli.artist);

    ScrobbleEdit::new(
        cli.track.clone(),
        cli.album.clone(),
        cli.artist.clone(),
        cli.album_artist.clone(),
        cli.new_track.clone(),
        cli.new_album.clone(),
        new_artist.to_string(),
        cli.new_album_artist.clone(),
        cli.timestamp,
        cli.edit_all,
    )
}

fn print_search_criteria(cli: &Cli) {
    println!("\n🔍 Search Criteria:");
    println!("  Artist: '{}'", cli.artist);

    if let Some(track) = &cli.track {
        println!("  Track: '{track}'");
    }

    if let Some(album) = &cli.album {
        println!("  Album: '{album}'");
    }

    if let Some(album_artist) = &cli.album_artist {
        println!("  Album Artist: '{album_artist}'");
    }

    if let Some(timestamp) = cli.timestamp {
        println!("  Timestamp: {timestamp}");
    }

    if cli.edit_all {
        println!("  Edit All: true");
    }

    // Show what changes would be made if any
    let has_changes = cli.new_track.is_some()
        || cli.new_album.is_some()
        || cli.new_artist.is_some()
        || cli.new_album_artist.is_some();

    if has_changes {
        println!("\n📝 Proposed Changes:");
        if let Some(new_track) = &cli.new_track {
            println!("  New Track: '{new_track}'");
        }
        if let Some(new_album) = &cli.new_album {
            println!("  New Album: '{new_album}'");
        }
        if let Some(new_artist) = &cli.new_artist {
            println!("  New Artist: '{new_artist}'");
        }
        if let Some(new_album_artist) = &cli.new_album_artist {
            println!("  New Album Artist: '{new_album_artist}'");
        }
    }
}

async fn discover_and_print_variations(
    client: &LastFmEditClientImpl,
    edit: &ScrobbleEdit,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n🔍 Discovering scrobble edit variations...");

    match client.discover_scrobble_edit_variations(edit).await {
        Ok(discovered_edits) => {
            println!("✅ Found {} scrobble variations:", discovered_edits.len());

            if discovered_edits.is_empty() {
                println!("No matching scrobbles found. This might mean:");
                println!("  - The specified metadata is not in your recent scrobbles");
                println!("  - The names don't match exactly");
                println!("  - There's a network or parsing issue");
                return Ok(());
            }

            // Print each discovered variation
            for (i, discovered_edit) in discovered_edits.iter().enumerate() {
                println!("\n  {}. Original Metadata:", i + 1);
                println!("     Track: '{}'", discovered_edit.track_name_original);
                println!("     Album: '{}'", discovered_edit.album_name_original);
                println!("     Artist: '{}'", discovered_edit.artist_name_original);
                println!(
                    "     Album Artist: '{}'",
                    discovered_edit.album_artist_name_original
                );
                println!("     Timestamp: {}", discovered_edit.timestamp);

                // Show what this would change to
                println!("     Would change to:");
                println!("       Track: '{}'", discovered_edit.track_name);
                println!("       Album: '{}'", discovered_edit.album_name);
                println!("       Artist: '{}'", discovered_edit.artist_name);
                println!(
                    "       Album Artist: '{}'",
                    discovered_edit.album_artist_name
                );
            }

            println!("\n📊 Summary:");
            println!("  Total variations found: {}", discovered_edits.len());

            // Group by unique original metadata combinations
            let mut unique_tracks = std::collections::HashSet::new();
            let mut unique_albums = std::collections::HashSet::new();

            for edit in &discovered_edits {
                unique_tracks.insert(&edit.track_name_original);
                unique_albums.insert(&edit.album_name_original);
            }

            println!("  Unique tracks: {}", unique_tracks.len());
            println!("  Unique albums: {}", unique_albums.len());

            println!("\n💡 To actually perform these edits, use the edit_scrobble method:");
            println!("  client.edit_scrobble(&edit).await?;");
        }
        Err(e) => {
            println!("❌ Could not discover scrobble variations: {e}");
            println!("This might mean:");
            println!("  - The specified metadata is not in your recent scrobbles");
            println!("  - The names don't match exactly");
            println!("  - There's a network or parsing issue");
        }
    }

    Ok(())
}
