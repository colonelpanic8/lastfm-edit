use lastfm_edit::{LastFmEditClient, LastFmEditClientImpl, LastFmEditSession};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Testing Artist Tracks with Album Information ===\n");

    // Create a test session (this won't work without real credentials)
    let test_session = LastFmEditSession::new(
        "test".to_string(),
        vec!["sessionid=.test123".to_string()],
        Some("csrf".to_string()),
        "https://www.last.fm".to_string(),
    );

    let client = LastFmEditClientImpl::from_session(
        Box::new(http_client::native::NativeClient::new()),
        test_session,
    );

    println!("🎵 Testing artist tracks iteration (album-based approach)");
    println!("   This will get tracks by iterating through albums first");
    println!("   Each track should now have album information populated");

    let mut tracks_iterator = client.artist_tracks("The Beatles");

    // Get first 5 tracks
    for i in 0..5 {
        match tracks_iterator.next().await {
            Ok(Some(track)) => {
                let album_info = track.album.as_deref().unwrap_or("No album info");
                let album_artist_info = track
                    .album_artist
                    .as_deref()
                    .unwrap_or("Same as track artist");

                println!(
                    "  [{:2}] {} - {} [{}]",
                    i + 1,
                    track.artist,
                    track.name,
                    album_info
                );
                println!(
                    "       Album Artist: {} | Play Count: {}",
                    album_artist_info, track.playcount
                );

                if let Some(timestamp) = track.timestamp {
                    println!("       Last Played: {timestamp}");
                }
                println!();
            }
            Ok(None) => {
                println!("  No more tracks found");
                break;
            }
            Err(e) => {
                println!("  ❌ Error: {e}");
                break;
            }
        }
    }

    println!("✨ Key improvements:");
    println!("   • Tracks now include complete album information");
    println!("   • Album artist information is available when different from track artist");
    println!("   • Implementation iterates through albums first, then gets tracks per album");
    println!("   • This provides richer metadata compared to the previous direct track approach");

    Ok(())
}
