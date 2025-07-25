use clap::Parser;
use lastfm_edit::{LastFmEditClient, LastFmEditClientImpl, ScrobbleEdit, SessionPersistence};
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

    // Try to load existing session first, then fallback to fresh login
    let client = load_or_create_client(&username, &password).await?;
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
    let mut edit_results = Vec::new();
    let mut count = 0;
    let mut successful_edits = 0;
    let mut failed_edits = 0;

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

        if dry_run {
            println!("     DRY RUN - proceeding without submitting edit");
            discovered_edits.push(discovered_edit);
        } else {
            // Apply edit immediately
            println!("     🔄 Applying edit...");

            // Apply the user's changes to create the final exact edit
            let mut final_edit = discovered_edit.clone();
            if let Some(new_track_name) = &edit.track_name {
                final_edit.track_name = new_track_name.clone();
            }
            if let Some(new_album_name) = &edit.album_name {
                final_edit.album_name = new_album_name.clone();
            }
            final_edit.artist_name = edit.artist_name.clone();
            if let Some(new_album_artist_name) = &edit.album_artist_name {
                final_edit.album_artist_name = new_album_artist_name.clone();
            }
            final_edit.edit_all = edit.edit_all;

            match client.edit_scrobble_single(&final_edit, 3).await {
                Ok(response) => {
                    if response.all_successful() {
                        successful_edits += 1;
                        println!("     ✅ Edit applied successfully!");
                    } else {
                        failed_edits += 1;
                        println!("     ❌ Edit failed: {}", response.summary_message());
                    }
                    edit_results.push(response);
                }
                Err(e) => {
                    failed_edits += 1;
                    println!("     ❌ Error applying edit: {e}");
                }
            }
        }
    }

    if count == 0 {
        println!("No matching scrobbles found. This might mean:");
        println!("  - The specified metadata is not in your recent scrobbles");
        println!("  - The names don't match exactly");
        println!("  - There's a network or parsing issue");
        return Ok(());
    }

    println!("\n📊 Summary:");
    println!("  Total variations found: {count}");

    if dry_run {
        // Group by unique original metadata combinations for dry run summary
        let mut unique_tracks = std::collections::HashSet::new();
        let mut unique_albums = std::collections::HashSet::new();

        for edit in &discovered_edits {
            unique_tracks.insert(&edit.track_name_original);
            unique_albums.insert(&edit.album_name_original);
        }

        println!("  Unique tracks: {}", unique_tracks.len());
        println!("  Unique albums: {}", unique_albums.len());
        println!("\n🔍 DRY RUN - No actual edits performed");
        println!("Use --apply to execute these edits");
    } else {
        println!("  Successful edits: {successful_edits}");
        println!("  Failed edits: {failed_edits}");

        if successful_edits > 0 {
            println!("\n✅ Edit session completed!");
        } else if failed_edits > 0 {
            println!("\n❌ All edits failed!");
        }

        if failed_edits > 0 {
            println!("\n⚠️  Failed edit details:");
            for (i, response) in edit_results.iter().enumerate() {
                if !response.all_successful() {
                    println!("    {}: {}", i + 1, response.summary_message());
                }
            }
        }
    }

    Ok(())
}

/// Load existing session or create a new client with fresh login.
///
/// This function implements the session management logic:
/// 1. Try to load a saved session from XDG directory
/// 2. Validate the loaded session
/// 3. If session is invalid or doesn't exist, perform fresh login
/// 4. Save the new session for future use
async fn load_or_create_client(
    username: &str,
    password: &str,
) -> Result<LastFmEditClientImpl, Box<dyn std::error::Error>> {
    // Check if we have a saved session
    if SessionPersistence::session_exists(username) {
        println!("📁 Found existing session for user '{username}', attempting to restore...");

        match SessionPersistence::load_session(username) {
            Ok(session) => {
                println!("📥 Session loaded successfully");

                // Create client with loaded session
                let http_client = http_client::native::NativeClient::new();
                let client = LastFmEditClientImpl::from_session(Box::new(http_client), session);

                // Validate the session
                println!("🔍 Validating session...");
                if client.validate_session().await {
                    println!("✅ Session is valid, using saved session");
                    return Ok(client);
                } else {
                    println!("❌ Session is invalid or expired");
                    // Remove invalid session file
                    let _ = SessionPersistence::remove_session(username);
                }
            }
            Err(e) => {
                println!("❌ Failed to load session: {e}");
                // Remove corrupted session file
                let _ = SessionPersistence::remove_session(username);
            }
        }
    }

    // No valid session found, perform fresh login
    println!("🔐 No valid session found, performing fresh login...");
    let http_client = http_client::native::NativeClient::new();
    let client =
        LastFmEditClientImpl::login_with_credentials(Box::new(http_client), username, password)
            .await?;

    // Save the new session
    println!("💾 Saving session for future use...");
    let session = client.get_session();
    if let Err(e) = SessionPersistence::save_session(&session) {
        println!("⚠️  Warning: Failed to save session: {e}");
        println!("   (You'll need to login again next time)");
    } else {
        println!("✅ Session saved successfully");
    }

    Ok(client)
}
