use lastfm_edit::{LastFmEditClientImpl, SessionPersistence};
use std::env;

/// Load existing session or create a new client with fresh login.
///
/// This function implements the session management logic:
/// 1. Try to load a saved session from XDG directory
/// 2. Validate the loaded session
/// 3. If session is invalid or doesn't exist, perform fresh login
/// 4. Save the new session for future use
pub async fn load_or_create_client(
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

/// Get username and password from environment variables
pub fn get_credentials() -> Result<(String, String), Box<dyn std::error::Error>> {
    let username = env::var("LASTFM_EDIT_USERNAME")
        .map_err(|_| "LASTFM_EDIT_USERNAME environment variable not set")?;
    let password = env::var("LASTFM_EDIT_PASSWORD")
        .map_err(|_| "LASTFM_EDIT_PASSWORD environment variable not set")?;
    Ok((username, password))
}

/// Format a Unix timestamp into a human-readable string
pub fn format_timestamp(timestamp: u64) -> String {
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
pub fn parse_range(
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
