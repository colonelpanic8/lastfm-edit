use http_client_vcr::{VcrClient, VcrMode};
use lastfm_edit::vcr_filters::create_lastfm_filter_chain;
use lastfm_edit::{LastFmEditClient, LastFmEditClientImpl};
use std::env;
use std::fs;

#[tokio::test]
async fn test_edit_wilco_whole_love_album() -> Result<(), Box<dyn std::error::Error>> {
    // Create cassette path and ensure directory exists
    let cassette_path = "tests/fixtures/wilco_whole_love_edit.yaml";
    if let Some(parent_dir) = std::path::Path::new(cassette_path).parent() {
        fs::create_dir_all(parent_dir)?;
    }

    // Check if cassette exists - if not, we MUST record (one shot only!)
    let cassette_exists = std::path::Path::new(cassette_path).exists();
    let vcr_record = env::var("VCR_RECORD").unwrap_or_default() == "true";

    let mode = if !cassette_exists {
        println!("⚠️  RECORDING MODE: This will edit REAL scrobbles on Last.fm!");
        println!("   Cassette doesn't exist - we only get one shot at this!");
        VcrMode::Record
    } else if vcr_record {
        println!("⚠️  FORCE RECORDING MODE: This will edit REAL scrobbles again!");
        println!("   WARNING: This may fail if the data was already edited!");
        VcrMode::Record
    } else {
        println!("✅ REPLAY MODE: Using existing cassette for safe testing");
        VcrMode::Replay
    };

    // Get credentials - real ones required for recording, dummy for replay
    let (username, password) = if matches!(mode, VcrMode::Record) {
        let username = env::var("LASTFM_EDIT_USERNAME")
            .expect("LASTFM_EDIT_USERNAME environment variable required for recording");
        let password = env::var("LASTFM_EDIT_PASSWORD")
            .expect("LASTFM_EDIT_PASSWORD environment variable required for recording");
        println!(
            "🔑 Using real credentials: username '{}' (password length: {})",
            username,
            password.len()
        );
        (username, password)
    } else {
        // Use the same username that was used during recording for VCR matching
        // Password can be dummy since it's filtered in the cassette
        println!("🎭 Using stable test credentials for replay");
        ("IvanMalison".to_string(), "test_password".to_string())
    };

    // Create VCR client with Last.fm filters
    let inner_client = Box::new(http_client::native::NativeClient::new());
    let filter_chain = create_lastfm_filter_chain()?;

    let vcr_client = VcrClient::builder()
        .inner_client(inner_client)
        .mode(mode.clone())
        .cassette_path(cassette_path)
        .filter_chain(filter_chain)
        .build()
        .await?;

    // Create Last.fm client and verify login works
    println!("🔐 Attempting login to Last.fm...");
    let client =
        LastFmEditClientImpl::login_with_credentials(Box::new(vcr_client), &username, &password)
            .await?;

    println!("✅ Login successful!");

    println!("🎵 Testing basic track discovery (editing disabled for now)...");

    // Just test that we can browse tracks after login - no editing yet
    let mut wilco_tracks = client.artist_tracks("Wilco");
    let mut track_count = 0;

    // Sample a few tracks to verify the connection works
    while let Some(track) = wilco_tracks.next().await? {
        track_count += 1;
        println!(
            "📀 Found track: {} - {} (album: {:?})",
            track.artist, track.name, track.album
        );

        // Just sample the first 5 tracks for now
        if track_count >= 5 {
            break;
        }
    }

    println!("✅ Successfully discovered {track_count} Wilco tracks");
    println!("🏁 Login and discovery test completed successfully");
    Ok(())
}
