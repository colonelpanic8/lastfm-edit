use lastfm_edit::{LastFmEditClient, LastFmEditClientImpl, ScrobbleEdit};
use std::env;

#[tokio::test]
async fn test_edit_wilco_whole_love_album() -> Result<(), Box<dyn std::error::Error>> {
    // Skip test if credentials are not available
    let username = match env::var("LASTFM_EDIT_USERNAME") {
        Ok(u) => u,
        Err(_) => {
            println!("⚠️  Skipping test: LASTFM_EDIT_USERNAME not set");
            return Ok(());
        }
    };

    let password = match env::var("LASTFM_EDIT_PASSWORD") {
        Ok(p) => p,
        Err(_) => {
            println!("⚠️  Skipping test: LASTFM_EDIT_PASSWORD not set");
            return Ok(());
        }
    };

    println!("🎵 Testing Wilco 'Whole Love' → 'The Whole Love' album correction");
    println!("📊 Username: {username}");

    // Create direct HTTP client (no VCR)
    let http_client = Box::new(http_client::native::NativeClient::new());

    // Login to Last.fm
    println!("🔐 Logging in to Last.fm...");
    let client =
        LastFmEditClientImpl::login_with_credentials(http_client, &username, &password).await?;
    println!("✅ Successfully logged in");

    // Search for Wilco tracks with album "Whole Love"
    println!("🔍 Searching for Wilco tracks with album 'Whole Love'...");

    let mut tracks_found = 0;
    let mut tracks_edited = 0;

    // Use the paginated iterator to find tracks
    let mut track_stream = client.recent_tracks();

    println!("📖 Scanning recent tracks for Wilco - 'Whole Love'...");

    while let Ok(Some(track)) = track_stream.next().await {
        // Check if this is a Wilco track with album "Whole Love"
        if track.artist.to_lowercase().contains("wilco")
            && track
                .album
                .as_ref()
                .map(|a| a.to_lowercase())
                .unwrap_or_default()
                .contains("whole love")
            && !track
                .album
                .as_ref()
                .map(|a| a.to_lowercase())
                .unwrap_or_default()
                .contains("the whole love")
        // Don't edit if already correct
        {
            tracks_found += 1;
            println!(
                "🎯 Found track: {} - {} (Album: {})",
                track.artist,
                track.name,
                track.album.as_ref().unwrap_or(&"Unknown".to_string())
            );

            // Create the edit to change album to "The Whole Love"
            let edit = ScrobbleEdit::for_album(
                track.album.as_ref().unwrap_or(&"Whole Love".to_string()), // Original album name
                &track.artist,                                             // Artist name
                &track.artist, // Album artist (same as artist)
            )
            .with_track_name(&track.name)
            .with_album_name("The Whole Love") // Corrected album name
            .with_edit_all(true); // Edit all instances

            // SAFETY: Only enable actual editing if explicitly requested
            let actually_edit = env::var("ACTUALLY_EDIT_SCROBBLES").unwrap_or_default() == "true";

            if actually_edit {
                println!("✏️  Editing scrobble...");
                match client.edit_scrobble(&edit).await {
                    Ok(_) => {
                        tracks_edited += 1;
                        println!(
                            "✅ Successfully edited track: {} → The Whole Love",
                            track.name
                        );
                    }
                    Err(e) => {
                        println!("❌ Failed to edit track {}: {}", track.name, e);
                    }
                }
            } else {
                println!("🧪 TEST MODE: Would edit album to 'The Whole Love' (set ACTUALLY_EDIT_SCROBBLES=true to enable)");
                tracks_edited += 1; // Count as "would be edited"
            }
        }

        // Limit search to avoid scanning too many tracks
        if tracks_found >= 10 {
            println!("📊 Limiting search to first 10 matching tracks");
            break;
        }
    }

    // Handle iterator termination or check more tracks if needed
    let mut tracks_scanned = 0;
    const MAX_TRACKS_TO_SCAN: u32 = 200; // Limit to first 200 tracks to avoid timeout

    loop {
        tracks_scanned += 1;
        if tracks_scanned > MAX_TRACKS_TO_SCAN {
            println!("📊 Reached scan limit of {MAX_TRACKS_TO_SCAN} tracks");
            break;
        }

        match track_stream.next().await {
            Ok(Some(_)) => continue,
            Ok(None) => {
                println!("📖 Reached end of recent tracks after scanning {tracks_scanned} tracks");
                break;
            }
            Err(e) => {
                println!("⚠️  Error fetching track: {e}");
                break;
            }
        }
    }

    // Report results
    println!("\n📈 RESULTS:");
    println!("   - Wilco 'Whole Love' tracks found: {tracks_found}");
    println!("   - Tracks edited: {tracks_edited}");

    if tracks_found == 0 {
        println!("ℹ️  No Wilco tracks with 'Whole Love' album found in recent tracks");
        println!("   This could mean:");
        println!("   - No such tracks exist in your library");
        println!("   - They're not in your recent tracks");
        println!("   - They've already been corrected to 'The Whole Love'");
    } else {
        println!("✅ Test completed successfully!");
    }

    Ok(())
}
