use crate::{LastFmEditClient, LastFmEditClientImpl, ScrobbleEdit};

/// Create a ScrobbleEdit from command line arguments
#[allow(clippy::too_many_arguments)]
pub fn create_scrobble_edit_from_args(
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

/// Handle the edit command
pub async fn handle_edit_command(
    client: &LastFmEditClientImpl,
    edit: &ScrobbleEdit,
    dry_run: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Show the ScrobbleEdit that will be sent
    println!("\n📦 ScrobbleEdit to be sent:");
    println!("{edit:#?}");

    // Discover and apply/show variations
    discover_and_handle_edits(client, edit, dry_run).await
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
