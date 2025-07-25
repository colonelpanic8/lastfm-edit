/// Example demonstrating how to use SessionManager with custom app names.
///
/// This example shows how other libraries can use SessionManager to store
/// Last.fm sessions with their own application prefix in XDG directories.
///
/// Usage:
///   direnv exec . cargo run --example custom_session_manager
use lastfm_edit::{LastFmEditClientImpl, SessionManager};
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    println!("🎵 Custom SessionManager Example");
    println!("=================================\n");

    // Create a custom session manager for your application
    let session_manager = SessionManager::new("my-music-app");
    println!("📁 Using app name: '{}'", session_manager.app_name());
    println!("📂 Sessions will be stored in: ~/.local/share/my-music-app/users/{{username}}/session.json\n");

    let username = env::var("LASTFM_EDIT_USERNAME")
        .expect("LASTFM_EDIT_USERNAME environment variable not set");
    let password = env::var("LASTFM_EDIT_PASSWORD")
        .expect("LASTFM_EDIT_PASSWORD environment variable not set");

    // Check if we have a saved session with our custom manager
    if session_manager.session_exists(&username) {
        println!("📁 Found existing session for user '{username}', attempting to restore...");

        match session_manager.load_session(&username) {
            Ok(session) => {
                println!("📥 Session loaded successfully");

                // Create client with loaded session
                let http_client = http_client::native::NativeClient::new();
                let client = LastFmEditClientImpl::from_session(Box::new(http_client), session);

                // Validate the session
                println!("🔍 Validating session...");
                if client.validate_session().await {
                    println!("✅ Session is valid, using saved session");

                    // Test the session by fetching recent tracks
                    println!("🎧 Testing session by fetching recent tracks...");
                    let tracks = client.get_recent_scrobbles(1).await?;
                    let recent_tracks: Vec<_> = tracks.into_iter().take(3).collect();
                    println!("📊 Found {} recent tracks:", recent_tracks.len());

                    for track in recent_tracks {
                        println!("   🎵 {} - {}", track.artist, track.name);
                    }

                    return Ok(());
                } else {
                    println!("❌ Session is invalid or expired");
                    // Remove invalid session file
                    let _ = session_manager.remove_session(&username);
                }
            }
            Err(e) => {
                println!("❌ Failed to load session: {e}");
                // Remove corrupted session file
                let _ = session_manager.remove_session(&username);
            }
        }
    }

    // No valid session found, perform fresh login
    println!("🔐 No valid session found, performing fresh login...");
    let http_client = http_client::native::NativeClient::new();
    let client =
        LastFmEditClientImpl::login_with_credentials(Box::new(http_client), &username, &password)
            .await?;

    // Save the new session with our custom manager
    println!("💾 Saving session with custom session manager...");
    let session = client.get_session();
    if let Err(e) = session_manager.save_session(&session) {
        println!("⚠️  Warning: Failed to save session: {e}");
        println!("   (You'll need to login again next time)");
    } else {
        println!("✅ Session saved to custom location");

        // Show the session path
        let session_path = session_manager.get_session_path(&username)?;
        println!("📂 Session saved to: {}", session_path.display());
    }

    // Test the new session
    println!("🎧 Testing session by fetching recent tracks...");
    let tracks = client.get_recent_scrobbles(1).await?;
    let recent_tracks: Vec<_> = tracks.into_iter().take(3).collect();
    println!("📊 Found {} recent tracks:", recent_tracks.len());

    for track in recent_tracks {
        println!("   🎵 {} - {}", track.artist, track.name);
    }

    // Demonstrate listing saved users
    println!("\n👥 Listing all saved users for this app:");
    let saved_users = session_manager.list_saved_users()?;
    if saved_users.is_empty() {
        println!("   No saved users found");
    } else {
        for user in saved_users {
            println!("   - {user}");
        }
    }

    println!("\n🎉 Example completed!");
    println!("💡 Your custom session is saved separately from the default lastfm-edit sessions.");
    println!(
        "💡 Other apps using SessionManager with different names won't interfere with each other."
    );

    Ok(())
}
