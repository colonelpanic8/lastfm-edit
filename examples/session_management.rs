/// Example demonstrating both login and session restore initialization methods.
///
/// This example shows how to:
/// 1. Initialize a client with username/password login
/// 2. Save the session state to a file
/// 3. Restore the session from the saved file
/// 4. Use both initialization patterns
///
/// Usage:
///   # First run - will prompt for credentials and save session
///   direnv exec . cargo run --example session_management
///
///   # Subsequent runs - will use saved session
///   direnv exec . cargo run --example session_management
use lastfm_edit::{AsyncPaginatedIterator, LastFmEditClient, LastFmEditSession, Result};
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::Path;

const SESSION_FILE: &str = "session.json";

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    println!("🎵 Last.fm Session Management Example");
    println!("=====================================\n");

    // Try to load existing session first
    if Path::new(SESSION_FILE).exists() {
        println!("📁 Found existing session file, attempting to restore...");
        match restore_from_session().await {
            Ok(client) => {
                println!("✅ Session restored successfully!");
                println!("👤 Logged in as: {}", client.username());

                // Test the restored session by fetching recent tracks
                println!("🎧 Testing session by fetching recent tracks...");
                let recent_tracks = client.recent_tracks().take(3).await?;
                println!("📊 Found {} recent tracks:", recent_tracks.len());

                for track in recent_tracks {
                    println!("   🎵 {} - {}", track.artist, track.name);
                }

                return Ok(());
            }
            Err(e) => {
                println!("❌ Failed to restore session: {e}");
                println!("🔄 Falling back to fresh login...\n");
                // Remove invalid session file
                let _ = fs::remove_file(SESSION_FILE);
            }
        }
    }

    // No valid session found, perform fresh login
    println!("🔑 No valid session found, performing fresh login...");
    let client = login_with_credentials().await?;
    println!("✅ Login successful!");
    println!("👤 Logged in as: {}", client.username());

    // Save session for future use
    println!("💾 Saving session to {SESSION_FILE}...");
    save_session(&client)?;
    println!("✅ Session saved!");

    // Test the new session
    println!("🎧 Testing session by fetching recent tracks...");
    let recent_tracks = client.recent_tracks().take(3).await?;
    println!("📊 Found {} recent tracks:", recent_tracks.len());

    for track in recent_tracks {
        println!("   🎵 {} - {}", track.artist, track.name);
    }

    println!("\n🎉 Example completed!");
    println!("💡 Next time you run this example, it will use the saved session automatically.");

    Ok(())
}

/// Restore client from saved session file
async fn restore_from_session() -> Result<LastFmEditClient> {
    let session_json = fs::read_to_string(SESSION_FILE)
        .map_err(|e| lastfm_edit::LastFmError::Http(format!("Failed to read session file: {e}")))?;

    let session = LastFmEditSession::from_json(&session_json)
        .map_err(|e| lastfm_edit::LastFmError::Http(format!("Failed to parse session: {e}")))?;

    if !session.is_valid() {
        return Err(lastfm_edit::LastFmError::Auth(
            "Invalid session data".to_string(),
        ));
    }

    let http_client = http_client::native::NativeClient::new();
    Ok(LastFmEditClient::from_session(
        Box::new(http_client),
        session,
    ))
}

/// Perform fresh login with credentials
async fn login_with_credentials() -> Result<LastFmEditClient> {
    // Method 1: Traditional create + login
    println!("🔧 Using traditional initialization pattern...");
    let http_client = http_client::native::NativeClient::new();
    let client = LastFmEditClient::new(Box::new(http_client));

    let username = get_username();
    let password = get_password();

    client.login(&username, &password).await?;
    Ok(client)

    // Alternative Method 2: One-step initialization (commented out)
    /*
    println!("🔧 Using one-step initialization pattern...");
    let username = get_username();
    let password = get_password();

    let http_client = http_client::native::NativeClient::new();
    LastFmEditClient::login_with_credentials(Box::new(http_client), &username, &password).await
    */
}

/// Save current session to file
fn save_session(client: &LastFmEditClient) -> Result<()> {
    let session = client.get_session();
    let session_json = session
        .to_json()
        .map_err(|e| lastfm_edit::LastFmError::Http(format!("Failed to serialize session: {e}")))?;

    fs::write(SESSION_FILE, session_json).map_err(|e| {
        lastfm_edit::LastFmError::Http(format!("Failed to write session file: {e}"))
    })?;

    Ok(())
}

/// Get username from environment variable or prompt
fn get_username() -> String {
    env::var("LASTFM_EDIT_USERNAME").unwrap_or_else(|_| {
        print!("Last.fm username: ");
        io::stdout().flush().unwrap();
        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();
        input.trim().to_string()
    })
}

/// Get password from environment variable or prompt
fn get_password() -> String {
    env::var("LASTFM_EDIT_PASSWORD").unwrap_or_else(|_| {
        print!("Last.fm password: ");
        io::stdout().flush().unwrap();
        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();
        input.trim().to_string()
    })
}
