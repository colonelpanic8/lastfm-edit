use serde::{Deserialize, Serialize};

/// Serializable client session state that can be persisted and restored.
///
/// This contains all the authentication state needed to resume a Last.fm session
/// without requiring the user to log in again.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientSession {
    /// The authenticated username
    pub username: String,
    /// Session cookies required for authenticated requests
    pub session_cookies: Vec<String>,
    /// CSRF token for form submissions
    pub csrf_token: Option<String>,
    /// Base URL for the Last.fm instance
    pub base_url: String,
}

impl ClientSession {
    /// Create a new client session with the provided state
    pub fn new(
        username: String,
        session_cookies: Vec<String>,
        csrf_token: Option<String>,
        base_url: String,
    ) -> Self {
        Self {
            username,
            session_cookies,
            csrf_token,
            base_url,
        }
    }

    /// Check if this session appears to be valid
    ///
    /// This performs basic validation but doesn't guarantee the session
    /// is still active on the server.
    pub fn is_valid(&self) -> bool {
        !self.username.is_empty()
            && !self.session_cookies.is_empty()
            && self.csrf_token.is_some()
            && self
                .session_cookies
                .iter()
                .any(|cookie| cookie.starts_with("sessionid=") && cookie.len() > 50)
    }

    /// Serialize session to JSON string
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Deserialize session from JSON string
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_validity() {
        let valid_session = ClientSession::new(
            "testuser".to_string(),
            vec![
                "sessionid=.eJy1234567890abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ"
                    .to_string(),
            ],
            Some("csrf_token_123".to_string()),
            "https://www.last.fm".to_string(),
        );
        assert!(valid_session.is_valid());

        let invalid_session = ClientSession::new(
            "".to_string(),
            vec![],
            None,
            "https://www.last.fm".to_string(),
        );
        assert!(!invalid_session.is_valid());
    }

    #[test]
    fn test_session_serialization() {
        let session = ClientSession::new(
            "testuser".to_string(),
            vec![
                "sessionid=.test123".to_string(),
                "csrftoken=abc".to_string(),
            ],
            Some("csrf_token_123".to_string()),
            "https://www.last.fm".to_string(),
        );

        let json = session.to_json().unwrap();
        let restored_session = ClientSession::from_json(&json).unwrap();

        assert_eq!(session.username, restored_session.username);
        assert_eq!(session.session_cookies, restored_session.session_cookies);
        assert_eq!(session.csrf_token, restored_session.csrf_token);
        assert_eq!(session.base_url, restored_session.base_url);
    }
}
