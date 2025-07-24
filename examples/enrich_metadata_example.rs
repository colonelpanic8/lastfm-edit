use clap::{Args, Parser, Subcommand};
use lastfm_edit::{LastFmEditClientImpl, ScrobbleEdit};
use std::env;

/// Example demonstrating the new discover_scrobble_edit_variations functionality.
///
/// This example shows how you can create a ScrobbleEdit with different levels of specificity
/// and the client will automatically discover all relevant scrobble instances based on what you specify:
/// - Track-specific: discovers all album variations of that track
/// - Album-specific: discovers all tracks in that album  
/// - Artist-specific: discovers all tracks by that artist
///
/// Usage examples:
/// # Edit specific track
/// direnv exec . cargo run --example enrich_metadata_example track --artist "Jimi Hendrix" --track "Lover Man"
///
/// # Edit all tracks in an album
/// direnv exec . cargo run --example enrich_metadata_example album --artist "Radiohead" --album "OK Computer"
///
/// # Edit all tracks by an artist
/// direnv exec . cargo run --example enrich_metadata_example artist --artist "The Beatles"
///
/// # Show what edits would be performed (dry run)
/// direnv exec . cargo run --example enrich_metadata_example track --artist "Jimi Hendrix" --track "Lover Man" --dry-run
#[derive(Parser)]
#[command(
    name = "enrich_metadata_example",
    about = "Demonstrate scrobble edit discovery functionality",
    long_about = None
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Perform a dry run without actually submitting edits
    #[arg(long, global = true)]
    dry_run: bool,

    /// Show detailed debug information
    #[arg(long, global = true)]
    verbose: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Edit a specific track (discovers all album variations)
    Track(TrackEditArgs),
    /// Edit all tracks in an album
    Album(AlbumEditArgs),
    /// Edit all tracks by an artist
    Artist(ArtistEditArgs),
}

#[derive(Args)]
struct TrackEditArgs {
    /// Artist name (required)
    #[arg(long)]
    artist: String,

    /// Track name (required)
    #[arg(long)]
    track: String,

    /// New track name (optional - if not provided, shows discovery only)
    #[arg(long)]
    new_track: Option<String>,

    /// New artist name (optional)
    #[arg(long)]
    new_artist: Option<String>,

    /// New album name (optional)
    #[arg(long)]
    new_album: Option<String>,
}

#[derive(Args)]
struct AlbumEditArgs {
    /// Artist name (required)
    #[arg(long)]
    artist: String,

    /// Album name (required)
    #[arg(long)]
    album: String,

    /// New artist name (optional)
    #[arg(long)]
    new_artist: Option<String>,

    /// New album name (optional)
    #[arg(long)]
    new_album: Option<String>,
}

#[derive(Args)]
struct ArtistEditArgs {
    /// Artist name (required)
    #[arg(long)]
    artist: String,

    /// New artist name (optional - if not provided, shows discovery only)
    #[arg(long)]
    new_artist: Option<String>,
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

    match cli.command {
        Commands::Track(args) => handle_track_edit(&client, args, cli.dry_run).await?,
        Commands::Album(args) => handle_album_edit(&client, args, cli.dry_run).await?,
        Commands::Artist(args) => handle_artist_edit(&client, args, cli.dry_run).await?,
    }

    Ok(())
}

async fn handle_track_edit(
    client: &LastFmEditClientImpl,
    args: TrackEditArgs,
    dry_run: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("\\n🎵 Track Edit Mode");
    println!("Target: '{}' by '{}'", args.track, args.artist);

    // Create a ScrobbleEdit for track-specific discovery
    let mut edit = ScrobbleEdit::from_track_and_artist(&args.track, &args.artist);

    // Apply any new values specified
    if let Some(new_track) = &args.new_track {
        edit = edit.with_track_name(new_track);
        println!("Will rename track to: '{new_track}'");
    }
    if let Some(new_artist) = &args.new_artist {
        edit = edit.with_artist_name(new_artist);
        println!("Will change artist to: '{new_artist}'");
    }
    if let Some(new_album) = &args.new_album {
        edit = edit.with_album_name(new_album);
        println!("Will change album to: '{new_album}'");
    }

    discover_and_show_edits(client, &edit, "track", dry_run).await
}

async fn handle_album_edit(
    client: &LastFmEditClientImpl,
    args: AlbumEditArgs,
    dry_run: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("\\n💿 Album Edit Mode");
    println!("Target: '{}' by '{}'", args.album, args.artist);

    // Create a ScrobbleEdit for album-specific discovery
    let mut edit = ScrobbleEdit::for_album(&args.album, &args.artist, &args.artist);

    // Apply any new values specified
    if let Some(new_artist) = &args.new_artist {
        edit = edit.with_artist_name(new_artist);
        println!("Will change artist to: '{new_artist}'");
    }
    if let Some(new_album) = &args.new_album {
        edit = edit.with_album_name(new_album);
        println!("Will change album name to: '{new_album}'");
    }

    discover_and_show_edits(client, &edit, "album", dry_run).await
}

async fn handle_artist_edit(
    client: &LastFmEditClientImpl,
    args: ArtistEditArgs,
    dry_run: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("\\n🎤 Artist Edit Mode");
    println!("Target: '{}'", args.artist);

    // Create a ScrobbleEdit for artist-specific discovery
    let mut edit = ScrobbleEdit::for_artist(&args.artist, &args.artist);

    // Apply any new values specified
    if let Some(new_artist) = &args.new_artist {
        edit = edit.with_artist_name(new_artist);
        println!("Will change artist to: '{new_artist}'");
    }

    discover_and_show_edits(client, &edit, "artist", dry_run).await
}

async fn discover_and_show_edits(
    client: &LastFmEditClientImpl,
    edit: &ScrobbleEdit,
    edit_type: &str,
    dry_run: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("\\n🔍 Discovering scrobble edit variations...");

    match client.discover_scrobble_edit_variations(edit).await {
        Ok(discovered_edits) => {
            println!(
                "✅ Found {} scrobble instances to edit:",
                discovered_edits.len()
            );

            if discovered_edits.is_empty() {
                println!("No matching scrobbles found. This might mean:");
                println!("  - The {edit_type} is not in your recent scrobbles");
                println!("  - The names don't match exactly");
                println!("  - There's a network or parsing issue");
                return Ok(());
            }

            // Show what will be edited
            for (i, discovered_edit) in discovered_edits.iter().enumerate() {
                println!(
                    "  {}. Track: '{}' | Album: '{}' by '{}' | Timestamp: {}",
                    i + 1,
                    discovered_edit.track_name_original,
                    discovered_edit.album_name_original,
                    discovered_edit.album_artist_name_original,
                    discovered_edit.timestamp
                );
            }

            // Show what changes would be made
            println!("\\n📝 Proposed changes:");
            let first_edit = &discovered_edits[0];

            if let Some(track_name) = &edit.track_name {
                if track_name != &first_edit.track_name_original {
                    println!(
                        "  Track name: '{}' → '{track_name}'",
                        first_edit.track_name_original
                    );
                }
            }

            if edit.artist_name != first_edit.artist_name_original {
                println!(
                    "  Artist: '{}' → '{}'",
                    first_edit.artist_name_original, edit.artist_name
                );
            }

            if edit.album_name != Some(first_edit.album_name_original.clone()) {
                println!(
                    "  Album: '{}' → '{}'",
                    first_edit.album_name_original,
                    edit.album_name.as_deref().unwrap_or("(keep original)")
                );
            }

            if dry_run {
                println!("\\n🔍 DRY RUN - No actual edits performed");
                println!("Remove --dry-run to execute these edits");
            } else {
                println!("\\n🚀 Executing edits...");
                match client.edit_scrobble(edit).await {
                    Ok(response) => {
                        if response.all_successful() {
                            println!(
                                "✅ All {} edits completed successfully!",
                                response.total_edits()
                            );
                        } else {
                            println!("⚠️  Some edits had issues:");
                            println!(
                                "  {} successful, {} failed",
                                response.successful_edits(),
                                response.failed_edits()
                            );
                            for (i, msg) in response.detailed_messages().iter().enumerate() {
                                println!("    {}: {}", i + 1, msg);
                            }
                        }
                    }
                    Err(e) => {
                        println!("❌ Error executing edits: {e}");
                    }
                }
            }

            // Show advanced usage example
            println!("\\n🔧 Advanced: ExactScrobbleEdit API");
            if let Some(first_discovered) = discovered_edits.first() {
                println!("For precise control, you can use ExactScrobbleEdit:");
                println!("  let exact_edit = ExactScrobbleEdit::new(");
                println!(
                    "      \\\"{}\\\".to_string(),",
                    first_discovered.track_name_original
                );
                println!(
                    "      \\\"{}\\\".to_string(),",
                    first_discovered.album_name_original
                );
                println!(
                    "      \\\"{}\\\".to_string(),",
                    first_discovered.artist_name_original
                );
                println!(
                    "      \\\"{}\\\".to_string(),",
                    first_discovered.album_artist_name_original
                );
                println!("      \\\"New Track Name\\\".to_string(),");
                println!("      \\\"New Album Name\\\".to_string(),");
                println!("      \\\"New Artist Name\\\".to_string(),");
                println!("      \\\"New Album Artist\\\".to_string(),");
                println!("      {},", first_discovered.timestamp);
                println!("      false");
                println!("  );");
                println!("  client.edit_scrobble_single(&exact_edit, 3).await?;");
            }
        }
        Err(e) => {
            println!("❌ Could not discover scrobble variations: {e}");
            println!("This might mean:");
            println!("  - The {edit_type} is not in your recent scrobbles");
            println!("  - The names don't match exactly");
            println!("  - There's a network or parsing issue");
        }
    }

    Ok(())
}
