use lastfm_edit::{LastFmEditClientImpl, SessionPersistence};
use std::env;
use std::io::{self, Write};

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
        log::info!("Found existing session for user '{username}', attempting to restore...");

        match SessionPersistence::load_session(username) {
            Ok(session) => {
                log::info!("Session loaded successfully");

                // Create client with loaded session
                let http_client = http_client::native::NativeClient::new();
                let client = LastFmEditClientImpl::from_session(Box::new(http_client), session);

                // Validate the session
                log::info!("Validating session...");
                if client.validate_session().await {
                    log::info!("Session is valid, using saved session");
                    return Ok(client);
                } else {
                    log::warn!("Session is invalid or expired");
                    // Remove invalid session file
                    let _ = SessionPersistence::remove_session(username);
                }
            }
            Err(e) => {
                log::warn!("Failed to load session: {e}");
                // Remove corrupted session file
                let _ = SessionPersistence::remove_session(username);
            }
        }
    }

    // No valid session found, perform fresh login
    log::info!("No valid session found, performing fresh login...");
    let http_client = http_client::native::NativeClient::new();
    let client =
        LastFmEditClientImpl::login_with_credentials(Box::new(http_client), username, password)
            .await?;

    // Save the new session
    log::info!("Saving session for future use...");
    let session = client.get_session();
    if let Err(e) = SessionPersistence::save_session(&session) {
        log::warn!("Failed to save session: {e}");
        log::warn!("You'll need to login again next time");
    } else {
        log::info!("Session saved successfully");
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

/// Try to restore the most recent session from available saved sessions.
///
/// This function looks for all saved sessions and attempts to restore the most recent valid one.
/// Returns Some(client) if a valid session was found and restored, None otherwise.
pub async fn try_restore_most_recent_session() -> Option<LastFmEditClientImpl> {
    // Get list of all saved users
    let saved_users = match SessionPersistence::list_saved_users() {
        Ok(users) => users,
        Err(_) => return None,
    };

    if saved_users.is_empty() {
        return None;
    }

    // Try each saved user session, starting with the first one found
    // In a more sophisticated implementation, we could sort by last modified time
    for username in saved_users {
        log::info!("Attempting to restore session for user '{username}'...");

        match SessionPersistence::load_session(&username) {
            Ok(session) => {
                log::info!("Session loaded successfully");

                // Create client with loaded session
                let http_client = http_client::native::NativeClient::new();
                let client = LastFmEditClientImpl::from_session(Box::new(http_client), session);

                // Validate the session
                log::info!("Validating session...");
                if client.validate_session().await {
                    log::info!("Session is valid for user '{username}'");
                    return Some(client);
                } else {
                    log::warn!("Session is invalid or expired for user '{username}'");
                    // Remove invalid session file
                    let _ = SessionPersistence::remove_session(&username);
                }
            }
            Err(e) => {
                log::warn!("Failed to load session for user '{username}': {e}");
                // Remove corrupted session file
                let _ = SessionPersistence::remove_session(&username);
            }
        }
    }

    None
}

/// Prompt the user for their Last.fm credentials interactively.
///
/// This function prompts for username and password via stdin, hiding password input.
/// Returns (username, password) tuple.
pub fn prompt_for_credentials() -> (String, String) {
    print!("Last.fm username: ");
    io::stdout().flush().unwrap();

    let mut username = String::new();
    io::stdin().read_line(&mut username).unwrap();
    let username = username.trim().to_string();

    // For password, we'll use a simple prompt for now
    // In a more sophisticated implementation, we could use a crate like `rpassword` to hide input
    print!("Last.fm password: ");
    io::stdout().flush().unwrap();

    let mut password = String::new();
    io::stdin().read_line(&mut password).unwrap();
    let password = password.trim().to_string();

    (username, password)
}
