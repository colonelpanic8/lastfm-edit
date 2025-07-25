use crate::session::LastFmEditSession;
use crate::{LastFmError, Result};
use std::fs;
use std::path::PathBuf;

/// Session persistence utilities for managing session data in XDG directories.
///
/// This module provides functionality to save and load Last.fm session data
/// using the XDG Base Directory Specification. Sessions are stored per-user
/// in the format: `~/.local/share/lastfm-edit/users/{username}/session.json`
pub struct SessionPersistence;

impl SessionPersistence {
    /// Get the session file path for a given username using XDG directories.
    ///
    /// Returns a path like: `~/.local/share/lastfm-edit/users/{username}/session.json`
    ///
    /// # Arguments
    /// * `username` - The Last.fm username
    ///
    /// # Returns
    /// Returns the path where the session should be stored, or an error if
    /// the XDG data directory cannot be determined.
    pub fn get_session_path(username: &str) -> Result<PathBuf> {
        let data_dir = dirs::data_dir()
            .ok_or_else(|| LastFmError::Http("Cannot determine XDG data directory".to_string()))?;

        let session_dir = data_dir.join("lastfm-edit").join("users").join(username);

        Ok(session_dir.join("session.json"))
    }

    /// Save a session to the XDG data directory.
    ///
    /// This creates the necessary directory structure and saves the session
    /// as JSON to `~/.local/share/lastfm-edit/users/{username}/session.json`
    ///
    /// # Arguments
    /// * `session` - The session to save
    ///
    /// # Returns
    /// Returns Ok(()) on success, or an error if the save fails.
    pub fn save_session(session: &LastFmEditSession) -> Result<()> {
        let session_path = Self::get_session_path(&session.username)?;

        // Create parent directories if they don't exist
        if let Some(parent) = session_path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                LastFmError::Http(format!("Failed to create session directory: {e}"))
            })?;
        }

        // Serialize session to JSON
        let session_json = session
            .to_json()
            .map_err(|e| LastFmError::Http(format!("Failed to serialize session: {e}")))?;

        // Write to file
        fs::write(&session_path, session_json)
            .map_err(|e| LastFmError::Http(format!("Failed to write session file: {e}")))?;

        log::debug!("Session saved to: {}", session_path.display());
        Ok(())
    }

    /// Load a session from the XDG data directory.
    ///
    /// Attempts to load a session from `~/.local/share/lastfm-edit/users/{username}/session.json`
    ///
    /// # Arguments
    /// * `username` - The Last.fm username
    ///
    /// # Returns
    /// Returns the loaded session on success, or an error if the file doesn't exist
    /// or cannot be parsed.
    pub fn load_session(username: &str) -> Result<LastFmEditSession> {
        let session_path = Self::get_session_path(username)?;

        if !session_path.exists() {
            return Err(LastFmError::Http(format!(
                "No saved session found for user: {username}"
            )));
        }

        // Read and parse session file
        let session_json = fs::read_to_string(&session_path)
            .map_err(|e| LastFmError::Http(format!("Failed to read session file: {e}")))?;

        let session = LastFmEditSession::from_json(&session_json)
            .map_err(|e| LastFmError::Http(format!("Failed to parse session JSON: {e}")))?;

        log::debug!("Session loaded from: {}", session_path.display());
        Ok(session)
    }

    /// Check if a saved session exists for the given username.
    ///
    /// # Arguments
    /// * `username` - The Last.fm username
    ///
    /// # Returns
    /// Returns true if a session file exists, false otherwise.
    pub fn session_exists(username: &str) -> bool {
        match Self::get_session_path(username) {
            Ok(path) => path.exists(),
            Err(_) => false,
        }
    }

    /// Remove a saved session for the given username.
    ///
    /// This deletes the session file from the XDG data directory.
    ///
    /// # Arguments
    /// * `username` - The Last.fm username
    ///
    /// # Returns
    /// Returns Ok(()) on success, or an error if the deletion fails.
    pub fn remove_session(username: &str) -> Result<()> {
        let session_path = Self::get_session_path(username)?;

        if session_path.exists() {
            fs::remove_file(&session_path)
                .map_err(|e| LastFmError::Http(format!("Failed to remove session file: {e}")))?;
            log::debug!("Session removed from: {}", session_path.display());
        }

        Ok(())
    }

    /// List all usernames that have saved sessions.
    ///
    /// Scans the XDG data directory for session files and returns the usernames.
    ///
    /// # Returns
    /// Returns a vector of usernames that have saved sessions.
    pub fn list_saved_users() -> Result<Vec<String>> {
        let data_dir = dirs::data_dir()
            .ok_or_else(|| LastFmError::Http("Cannot determine XDG data directory".to_string()))?;

        let users_dir = data_dir.join("lastfm-edit").join("users");

        if !users_dir.exists() {
            return Ok(Vec::new());
        }

        let mut users = Vec::new();
        let entries = fs::read_dir(&users_dir)
            .map_err(|e| LastFmError::Http(format!("Failed to read users directory: {e}")))?;

        for entry in entries {
            let entry = entry
                .map_err(|e| LastFmError::Http(format!("Failed to read directory entry: {e}")))?;

            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                let session_file = entry.path().join("session.json");
                if session_file.exists() {
                    if let Some(username) = entry.file_name().to_str() {
                        users.push(username.to_string());
                    }
                }
            }
        }

        Ok(users)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_path_generation() {
        let path = SessionPersistence::get_session_path("testuser").unwrap();
        assert!(path
            .to_string_lossy()
            .contains("lastfm-edit/users/testuser/session.json"));
    }

    #[test]
    fn test_session_exists_nonexistent() {
        let fake_username = format!("nonexistent_user_{}", std::process::id());
        assert!(!SessionPersistence::session_exists(&fake_username));
    }
}
