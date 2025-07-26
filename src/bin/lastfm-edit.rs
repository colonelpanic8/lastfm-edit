use clap::{Parser, Subcommand};
use lastfm_edit::{LastFmEditClient, LastFmEditClientImpl, ScrobbleEdit, SessionPersistence};
use std::env;

/// Last.fm scrobble metadata editor
#[derive(Parser)]
#[command(
    name = "lastfm-edit",
    about = "Last.fm scrobble metadata editor",
    long_about = None
)]
struct Cli {
    /// Show detailed debug information
    #[arg(long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
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

        /// Whether to edit all instances (optional, defaults to false)
        #[arg(long)]
        edit_all: bool,

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
    /// # Delete recent scrobbles from pages 1-2
    /// lastfm-edit delete --recent-pages 1-2 --apply
    ///
    /// # Delete the 10th through 50th most recent scrobbles (0-indexed, so 9-49)  
    /// lastfm-edit delete --recent-offset 9-49 --apply  
    ///
    /// # Delete the 5 most recent scrobbles (offset 0-4)
    /// lastfm-edit delete --recent-offset 0-4 --apply
    Delete {
        /// Range of recent pages to delete (e.g., "1-3" for pages 1 through 3)
        #[arg(long, conflicts_with_all = ["timestamp_range", "recent_offset"])]
        recent_pages: Option<String>,

        /// Timestamp range to delete (e.g., "1640995200-1641000000")
        #[arg(long, conflicts_with_all = ["recent_pages", "recent_offset"])]
        timestamp_range: Option<String>,

        /// Offset range from most recent scrobble (e.g., "0-9" for the 0th through 9th most recent, 0-indexed)
        #[arg(long, conflicts_with_all = ["recent_pages", "timestamp_range"])]
        recent_offset: Option<String>,

        /// Actually perform the deletions (default is dry-run mode)
        #[arg(long)]
        apply: bool,

        /// Perform a dry run without actually deleting (default behavior)
        #[arg(long)]
        dry_run: bool,
    },
    /// Show details for specific scrobbles by offset
    ///
    /// This command shows detailed information for scrobbles at the specified
    /// offsets from your most recent scrobbles. This is useful for inspecting
    /// specific scrobbles before deciding whether to delete or edit them.
    ///
    /// Usage examples:
    /// # Show details for the 5th most recent scrobble (0-indexed)
    /// lastfm-edit show 4
    ///
    /// # Show details for the 0th, 2nd, and 9th most recent scrobbles
    /// lastfm-edit show 0 2 9
    ///
    /// # Show details for scrobbles 5 through 15 (0-indexed)
    /// lastfm-edit show 5 6 7 8 9 10 11 12 13 14 15
    Show {
        /// Offset positions of scrobbles to show (0-indexed, e.g., 0 = most recent)
        offsets: Vec<u64>,
    },
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

    // Validate arguments before authentication
    if let Commands::Show { offsets } = &cli.command {
        if offsets.is_empty() {
            eprintln!("Error: Must specify at least one offset");
            std::process::exit(1);
        }
    }

    let username = env::var("LASTFM_EDIT_USERNAME")
        .expect("LASTFM_EDIT_USERNAME environment variable not set");
    let password = env::var("LASTFM_EDIT_PASSWORD")
        .expect("LASTFM_EDIT_PASSWORD environment variable not set");

    // Try to load existing session first, then fallback to fresh login
    let client = load_or_create_client(&username, &password).await?;
    println!("✅ Successfully authenticated as {}", client.username());

    match cli.command {
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
            edit_all,
            apply,
            dry_run: _,
        } => {
            // Determine whether to actually apply edits
            // Default is dry-run mode unless --apply is specified
            let dry_run = !apply;

            // Create ScrobbleEdit based on provided arguments
            let edit = create_scrobble_edit_from_args(
                &artist,
                track.as_deref(),
                album.as_deref(),
                album_artist.as_deref(),
                new_track.as_deref(),
                new_album.as_deref(),
                new_artist.as_deref(),
                new_album_artist.as_deref(),
                timestamp,
                edit_all,
            );

            // Show the ScrobbleEdit that will be sent
            println!("\n📦 ScrobbleEdit to be sent:");
            println!("{edit:#?}");

            // Discover and apply/show variations
            discover_and_handle_edits(&client, &edit, dry_run).await?;
        }
        Commands::Delete {
            recent_pages,
            timestamp_range,
            recent_offset,
            apply,
            dry_run: _,
        } => {
            // Determine whether to actually perform deletions
            // Default is dry-run mode unless --apply is specified
            let dry_run = !apply;

            if let Some(pages_range) = recent_pages {
                handle_delete_recent_pages(&client, &pages_range, dry_run).await?;
            } else if let Some(ts_range) = timestamp_range {
                handle_delete_timestamp_range(&client, &ts_range, dry_run).await?;
            } else if let Some(offset_range) = recent_offset {
                handle_delete_recent_offset(&client, &offset_range, dry_run).await?;
            } else {
                eprintln!("Error: Must specify one of --recent-pages, --timestamp-range, or --recent-offset");
                std::process::exit(1);
            }
        }
        Commands::Show { offsets } => {
            handle_show_scrobbles(&client, &offsets).await?;
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn create_scrobble_edit_from_args(
    artist: &str,
    track: Option<&str>,
    album: Option<&str>,
    album_artist: Option<&str>,
    new_track: Option<&str>,
    new_album: Option<&str>,
    new_artist: Option<&str>,
    new_album_artist: Option<&str>,
    timestamp: Option<u64>,
    edit_all: bool,
) -> ScrobbleEdit {
    // Determine the new artist name (use provided new_artist or original artist)
    let new_artist = new_artist.unwrap_or(artist);

    ScrobbleEdit::new(
        track.map(|s| s.to_string()),
        album.map(|s| s.to_string()),
        artist.to_string(),
        album_artist.map(|s| s.to_string()),
        new_track.map(|s| s.to_string()),
        new_album.map(|s| s.to_string()),
        new_artist.to_string(),
        new_album_artist.map(|s| s.to_string()),
        timestamp,
        edit_all,
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

/// Handle deletion of scrobbles from recent pages
async fn handle_delete_recent_pages(
    client: &LastFmEditClientImpl,
    pages_range: &str,
    dry_run: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let (start_page, end_page) = parse_range(pages_range, "pages")?;

    println!("🗑️  Delete recent scrobbles from pages {start_page}-{end_page}");
    if dry_run {
        println!("🔍 DRY RUN - No actual deletions will be performed");
    }

    let mut total_scrobbles = 0;
    let mut successful_deletions = 0;
    let mut failed_deletions = 0;
    let mut scrobbles_to_delete = Vec::new();

    // Collect scrobbles from the specified pages
    for page in start_page..=end_page {
        println!("\n📄 Processing page {page}...");

        match client.get_recent_scrobbles(page.try_into().unwrap()).await {
            Ok(scrobbles) => {
                if scrobbles.is_empty() {
                    println!("  No scrobbles found on page {page}");
                    break; // No more pages
                }

                println!("  Found {} scrobbles on page {page}", scrobbles.len());
                total_scrobbles += scrobbles.len();

                for scrobble in scrobbles {
                    if let Some(timestamp) = scrobble.timestamp {
                        scrobbles_to_delete.push((
                            scrobble.artist.clone(),
                            scrobble.name.clone(),
                            timestamp,
                        ));

                        if dry_run {
                            println!(
                                "    Would delete: '{}' by '{}' ({})",
                                scrobble.name, scrobble.artist, timestamp
                            );
                        }
                    } else {
                        println!(
                            "    ⚠️  Skipping scrobble without timestamp: '{}' by '{}'",
                            scrobble.name, scrobble.artist
                        );
                    }
                }
            }
            Err(e) => {
                println!("  ❌ Error fetching page {page}: {e}");
                break;
            }
        }
    }

    if scrobbles_to_delete.is_empty() {
        println!("\n❌ No scrobbles with timestamps found in the specified page range");
        return Ok(());
    }

    println!("\n📊 Summary:");
    println!("  Total scrobbles found: {total_scrobbles}");
    println!("  Scrobbles with timestamps: {}", scrobbles_to_delete.len());

    if dry_run {
        println!("\n🔍 DRY RUN - No actual deletions performed");
        println!("Use --apply to execute these deletions");
        return Ok(());
    }

    // Actually delete the scrobbles
    println!("\n🗑️  Deleting scrobbles...");

    for (i, (artist, track, timestamp)) in scrobbles_to_delete.iter().enumerate() {
        println!(
            "  {}/{}: Deleting '{}' by '{}'",
            i + 1,
            scrobbles_to_delete.len(),
            track,
            artist
        );

        match client.delete_scrobble(artist, track, *timestamp).await {
            Ok(true) => {
                successful_deletions += 1;
                println!("    ✅ Deleted successfully");
            }
            Ok(false) => {
                failed_deletions += 1;
                println!("    ❌ Deletion failed");
            }
            Err(e) => {
                failed_deletions += 1;
                println!("    ❌ Error: {e}");
            }
        }

        // Add delay between deletions to be respectful to the server
        if i < scrobbles_to_delete.len() - 1 {
            tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
        }
    }

    println!("\n📊 Final Summary:");
    println!("  Successful deletions: {successful_deletions}");
    println!("  Failed deletions: {failed_deletions}");

    if successful_deletions > 0 {
        println!("\n✅ Deletion session completed!");
    } else if failed_deletions > 0 {
        println!("\n❌ All deletions failed!");
    }

    Ok(())
}

/// Handle deletion of scrobbles from timestamp range
async fn handle_delete_timestamp_range(
    client: &LastFmEditClientImpl,
    timestamp_range: &str,
    dry_run: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let (start_ts, end_ts) = parse_range(timestamp_range, "timestamp")?;

    println!("🗑️  Delete scrobbles from timestamp range {start_ts}-{end_ts}");
    if dry_run {
        println!("🔍 DRY RUN - No actual deletions will be performed");
    }

    let mut successful_deletions = 0;
    let mut failed_deletions = 0;
    let mut scrobbles_to_delete = Vec::new();

    // Search through recent scrobbles to find ones in the timestamp range
    let max_pages = 20; // Search up to 20 pages of recent scrobbles

    for page in 1..=max_pages {
        println!("📄 Searching page {page} for scrobbles in timestamp range...");

        match client.get_recent_scrobbles(page).await {
            Ok(scrobbles) => {
                if scrobbles.is_empty() {
                    println!("  No more scrobbles found, stopping search");
                    break;
                }

                let mut found_in_range = 0;
                for scrobble in scrobbles {
                    if let Some(timestamp) = scrobble.timestamp {
                        if timestamp >= start_ts && timestamp <= end_ts {
                            found_in_range += 1;
                            scrobbles_to_delete.push((
                                scrobble.artist.clone(),
                                scrobble.name.clone(),
                                timestamp,
                            ));

                            if dry_run {
                                println!(
                                    "    Would delete: '{}' by '{}' ({})",
                                    scrobble.name, scrobble.artist, timestamp
                                );
                            }
                        }
                    }
                }

                if found_in_range > 0 {
                    println!("  Found {found_in_range} scrobbles in range on page {page}");
                } else {
                    println!("  No scrobbles in range on page {page}");
                }
            }
            Err(e) => {
                println!("  ❌ Error fetching page {page}: {e}");
                break;
            }
        }
    }

    if scrobbles_to_delete.is_empty() {
        println!("\n❌ No scrobbles found in the specified timestamp range");
        return Ok(());
    }

    println!("\n📊 Summary:");
    println!(
        "  Scrobbles in timestamp range: {}",
        scrobbles_to_delete.len()
    );

    if dry_run {
        println!("\n🔍 DRY RUN - No actual deletions performed");
        println!("Use --apply to execute these deletions");
        return Ok(());
    }

    // Actually delete the scrobbles
    println!("\n🗑️  Deleting scrobbles...");

    for (i, (artist, track, timestamp)) in scrobbles_to_delete.iter().enumerate() {
        println!(
            "  {}/{}: Deleting '{}' by '{}'",
            i + 1,
            scrobbles_to_delete.len(),
            track,
            artist
        );

        match client.delete_scrobble(artist, track, *timestamp).await {
            Ok(true) => {
                successful_deletions += 1;
                println!("    ✅ Deleted successfully");
            }
            Ok(false) => {
                failed_deletions += 1;
                println!("    ❌ Deletion failed");
            }
            Err(e) => {
                failed_deletions += 1;
                println!("    ❌ Error: {e}");
            }
        }

        // Add delay between deletions to be respectful to the server
        if i < scrobbles_to_delete.len() - 1 {
            tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
        }
    }

    println!("\n📊 Final Summary:");
    println!("  Successful deletions: {successful_deletions}");
    println!("  Failed deletions: {failed_deletions}");

    if successful_deletions > 0 {
        println!("\n✅ Deletion session completed!");
    } else if failed_deletions > 0 {
        println!("\n❌ All deletions failed!");
    }

    Ok(())
}

/// Handle deletion of scrobbles by offset from most recent
async fn handle_delete_recent_offset(
    client: &LastFmEditClientImpl,
    offset_range: &str,
    dry_run: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let (start_offset, end_offset) = parse_range(offset_range, "offset")?;

    // Offsets are already 0-based, so use directly
    let start_index = start_offset;
    let end_index = end_offset;

    println!("🗑️  Delete scrobbles by offset: {start_offset}-{end_offset} ({start_offset}th to {end_offset}th most recent, 0-indexed)");
    if dry_run {
        println!("🔍 DRY RUN - No actual deletions will be performed");
    }

    let mut all_scrobbles = Vec::new();
    let mut successful_deletions = 0;
    let mut failed_deletions = 0;

    // Collect scrobbles until we have enough to cover the offset range
    let mut page = 1;
    let needed_scrobbles = (end_offset + 1) as usize; // +1 because 0-indexed

    println!("\n📄 Collecting recent scrobbles to reach offset {end_offset}...");

    while all_scrobbles.len() < needed_scrobbles {
        match client.get_recent_scrobbles(page).await {
            Ok(scrobbles) => {
                if scrobbles.is_empty() {
                    println!("  No more scrobbles found on page {page}");
                    break;
                }

                println!(
                    "  Page {page}: Found {} scrobbles (total: {})",
                    scrobbles.len(),
                    all_scrobbles.len() + scrobbles.len()
                );
                all_scrobbles.extend(scrobbles);
                page += 1;

                // Stop if we've collected enough
                if all_scrobbles.len() >= needed_scrobbles {
                    break;
                }
            }
            Err(e) => {
                println!("  ❌ Error fetching page {page}: {e}");
                break;
            }
        }
    }

    if all_scrobbles.len() <= start_offset as usize {
        println!("\n❌ Not enough recent scrobbles found. You have {} scrobbles, but requested offset starts at {} (0-indexed)", all_scrobbles.len(), start_offset);
        return Ok(());
    }

    // Extract the scrobbles in the specified offset range
    let actual_end_index = std::cmp::min(end_index as usize, all_scrobbles.len() - 1);
    let scrobbles_in_range = &all_scrobbles[start_index as usize..=actual_end_index];

    println!("\n📊 Summary:");
    println!(
        "  Total recent scrobbles collected: {}",
        all_scrobbles.len()
    );
    println!(
        "  Scrobbles in offset range {}-{}: {}",
        start_offset,
        std::cmp::min(end_offset, (all_scrobbles.len() as u64).saturating_sub(1)),
        scrobbles_in_range.len()
    );

    if dry_run {
        println!("\n🔍 Scrobbles that would be deleted:");
        for (i, scrobble) in scrobbles_in_range.iter().enumerate() {
            let offset_number = start_offset + i as u64;
            if let Some(timestamp) = scrobble.timestamp {
                println!(
                    "    {}: '{}' by '{}' ({})",
                    offset_number, scrobble.name, scrobble.artist, timestamp
                );
            } else {
                println!(
                    "    {}: '{}' by '{}' (no timestamp - cannot delete)",
                    offset_number, scrobble.name, scrobble.artist
                );
            }
        }

        println!("\n🔍 DRY RUN - No actual deletions performed");
        println!("Use --apply to execute these deletions");
        return Ok(());
    }

    // Actually delete the scrobbles
    println!("\n🗑️  Deleting scrobbles by offset...");

    for (i, scrobble) in scrobbles_in_range.iter().enumerate() {
        let offset_number = start_offset + i as u64;

        if let Some(timestamp) = scrobble.timestamp {
            println!(
                "  {}/{}: Deleting offset {} - '{}' by '{}'",
                i + 1,
                scrobbles_in_range.len(),
                offset_number,
                scrobble.name,
                scrobble.artist
            );

            match client
                .delete_scrobble(&scrobble.artist, &scrobble.name, timestamp)
                .await
            {
                Ok(true) => {
                    successful_deletions += 1;
                    println!("    ✅ Deleted successfully");
                }
                Ok(false) => {
                    failed_deletions += 1;
                    println!("    ❌ Deletion failed");
                }
                Err(e) => {
                    failed_deletions += 1;
                    println!("    ❌ Error: {e}");
                }
            }
        } else {
            failed_deletions += 1;
            println!(
                "  {}/{}: Skipping offset {} - '{}' by '{}' (no timestamp)",
                i + 1,
                scrobbles_in_range.len(),
                offset_number,
                scrobble.name,
                scrobble.artist
            );
        }

        // Add delay between deletions to be respectful to the server
        if i < scrobbles_in_range.len() - 1 {
            tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
        }
    }

    println!("\n📊 Final Summary:");
    println!("  Successful deletions: {successful_deletions}");
    println!("  Failed deletions: {failed_deletions}");

    if successful_deletions > 0 {
        println!("\n✅ Deletion session completed!");
    } else if failed_deletions > 0 {
        println!("\n❌ All deletions failed!");
    }

    Ok(())
}

/// Handle showing details for specific scrobbles by offset
async fn handle_show_scrobbles(
    client: &LastFmEditClientImpl,
    offsets: &[u64],
) -> Result<(), Box<dyn std::error::Error>> {
    // No validation needed for 0-based indexing - all u64 values are valid

    let max_offset = *offsets.iter().max().unwrap();

    println!(
        "📋 Showing details for scrobbles at offsets: {}",
        offsets
            .iter()
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );

    // Sort offsets for better output organization
    let mut sorted_offsets = offsets.to_vec();
    sorted_offsets.sort_unstable();

    let mut all_scrobbles = Vec::new();

    // Collect scrobbles until we have enough to cover the maximum offset
    let mut page = 1;
    let needed_scrobbles = (max_offset + 1) as usize; // +1 because 0-indexed

    println!("\n📄 Collecting recent scrobbles to reach offset {max_offset}...");

    while all_scrobbles.len() < needed_scrobbles {
        match client.get_recent_scrobbles(page).await {
            Ok(scrobbles) => {
                if scrobbles.is_empty() {
                    println!("  No more scrobbles found on page {page}");
                    break;
                }

                println!(
                    "  Page {page}: Found {} scrobbles (total: {})",
                    scrobbles.len(),
                    all_scrobbles.len() + scrobbles.len()
                );
                all_scrobbles.extend(scrobbles);
                page += 1;

                // Stop if we've collected enough
                if all_scrobbles.len() >= needed_scrobbles {
                    break;
                }
            }
            Err(e) => {
                println!("  ❌ Error fetching page {page}: {e}");
                break;
            }
        }
    }

    println!("\n📊 Total scrobbles collected: {}", all_scrobbles.len());

    // Check if we have enough scrobbles for all requested offsets
    let unavailable_offsets: Vec<u64> = offsets
        .iter()
        .filter(|&&offset| offset >= all_scrobbles.len() as u64)
        .copied()
        .collect();

    if !unavailable_offsets.is_empty() {
        println!(
            "\n⚠️  The following offsets are not available (you only have {} scrobbles):",
            all_scrobbles.len()
        );
        for offset in &unavailable_offsets {
            println!("    - Offset {offset}");
        }
        println!();
    }

    // Show details for each requested offset
    println!("🎵 Scrobble Details:");
    println!("{}", "=".repeat(80));

    for &offset in &sorted_offsets {
        if offset < all_scrobbles.len() as u64 {
            let scrobble = &all_scrobbles[offset as usize]; // Use offset directly as 0-based index

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
                println!(
                    "   🕐 Timestamp: {} ({})",
                    timestamp,
                    format_timestamp(timestamp)
                );
            } else {
                println!("   🕐 Timestamp: (no timestamp)");
            }
        }
    }

    if !unavailable_offsets.is_empty() {
        println!(
            "\n❌ Could not show {} offset(s) due to insufficient scrobbles",
            unavailable_offsets.len()
        );
    }

    println!("\n✅ Finished showing scrobble details");

    Ok(())
}

/// Format a Unix timestamp into a human-readable string
fn format_timestamp(timestamp: u64) -> String {
    // This is a simple formatter - in a full implementation you might want to use chrono
    // For now, just show it as "X seconds ago" or the raw timestamp
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    if timestamp <= now {
        let ago = now - timestamp;
        if ago < 60 {
            format!("{ago} seconds ago")
        } else if ago < 3600 {
            format!("{} minutes ago", ago / 60)
        } else if ago < 86400 {
            format!("{} hours ago", ago / 3600)
        } else {
            format!("{} days ago", ago / 86400)
        }
    } else {
        format!("{timestamp} (future timestamp)")
    }
}

/// Parse a range string like "1-3" or "1640995200-1641000000"
fn parse_range(
    range_str: &str,
    range_type: &str,
) -> Result<(u64, u64), Box<dyn std::error::Error>> {
    let parts: Vec<&str> = range_str.split('-').collect();
    if parts.len() != 2 {
        return Err(format!(
            "Invalid {range_type} range format. Expected 'start-end', got '{range_str}'"
        )
        .into());
    }

    let start: u64 = parts[0]
        .parse()
        .map_err(|_| format!("Invalid start {range_type}: '{}'", parts[0]))?;
    let end: u64 = parts[1]
        .parse()
        .map_err(|_| format!("Invalid end {range_type}: '{}'", parts[1]))?;

    if start > end {
        return Err(format!(
            "Start {range_type} ({start}) cannot be greater than end {range_type} ({end})"
        )
        .into());
    }

    Ok((start, end))
}
