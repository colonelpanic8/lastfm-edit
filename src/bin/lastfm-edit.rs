use clap::Parser;
use lastfm_edit::{LastFmEditClientImpl, ScrobbleEdit};
use std::env;

/// Last.fm scrobble metadata editor
///
/// This tool allows you to edit scrobble metadata by specifying what to search for
/// and what to change it to. You can specify any combination of fields to search for,
/// and any combination of new values to change them to.
///
/// Usage examples:
/// # Discover variations for an artist (dry run by default)
/// lastfm-edit --artist "Jimi Hendrix"
///
/// # Discover variations with optional track name
/// lastfm-edit --artist "Radiohead" --track "Creep"
///
/// # Actually apply edits (change artist name)
/// lastfm-edit --artist "The Beatles" --new-artist "Beatles, The" --apply
///
/// # Change track name for specific track
/// lastfm-edit --artist "Jimi Hendrix" --track "Lover Man" --new-track "Lover Man (Live)" --apply
#[derive(Parser)]
#[command(
    name = "lastfm-edit",
    about = "Last.fm scrobble metadata editor",
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

    /// Actually apply the edits (default is dry-run mode)
    #[arg(long)]
    apply: bool,

    /// Perform a dry run without actually submitting edits (default behavior)
    #[arg(long)]
    dry_run: bool,

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

    // Determine whether to actually apply edits
    // Default is dry-run mode unless --apply is specified
    let dry_run = !cli.apply;

    // Create and authenticate client
    let http_client = http_client::native::NativeClient::new();
    let client = LastFmEditClientImpl::new(Box::new(http_client));

    println!("🔐 Logging in to Last.fm...");
    client.login(&username, &password).await?;
    println!("✅ Successfully authenticated as {}", client.username());

    // Create ScrobbleEdit based on provided arguments
    let edit = create_scrobble_edit(&cli);

    // Show the ScrobbleEdit that will be sent
    println!("\n📦 ScrobbleEdit to be sent:");
    println!("{edit:#?}");

    // Discover and apply/show variations
    discover_and_handle_edits(&client, &edit, dry_run).await?;

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

async fn discover_and_handle_edits(
    client: &LastFmEditClientImpl,
    edit: &ScrobbleEdit,
    dry_run: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("\n🔍 Discovering scrobble edit variations...");

    // Use the discovery iterator for incremental results
    let mut discovery_iterator = client.discover_scrobbles(edit.clone());
    let mut discovered_edits = Vec::new();
    let mut count = 0;

    // Process results incrementally
    while let Some(discovered_edit) = discovery_iterator.next().await? {
        count += 1;
        println!("\n  {count}. Found scrobble:");
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

        discovered_edits.push(discovered_edit);
    }

    if discovered_edits.is_empty() {
        println!("No matching scrobbles found. This might mean:");
        println!("  - The specified metadata is not in your recent scrobbles");
        println!("  - The names don't match exactly");
        println!("  - There's a network or parsing issue");
        return Ok(());
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

    if dry_run {
        println!("\n🔍 DRY RUN - No actual edits performed");
        println!("Use --apply to execute these edits");
    } else {
        println!("\n🚀 Executing edits...");
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

    Ok(())
}
