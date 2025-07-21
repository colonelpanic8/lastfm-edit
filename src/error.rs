use thiserror::Error;

/// Error types for Last.fm operations.
///
/// This enum covers all possible errors that can occur when interacting with Last.fm,
/// including network issues, authentication failures, parsing errors, and rate limiting.
///
/// # Error Handling Examples
///
/// ```rust,no_run
/// use lastfm_edit::{LastFmEditClient, LastFmError};
///
/// #[tokio::main]
/// async fn main() {
///     let mut client = LastFmEditClient::new(Box::new(http_client::native::NativeClient::new()));
///
///     match client.login("username", "password").await {
///         Ok(()) => println!("Login successful"),
///         Err(LastFmError::Auth(msg)) => eprintln!("Authentication failed: {}", msg),
///         Err(LastFmError::RateLimit { retry_after }) => {
///             eprintln!("Rate limited, retry in {} seconds", retry_after);
///         }
///         Err(LastFmError::Http(msg)) => eprintln!("Network error: {}", msg),
///         Err(e) => eprintln!("Other error: {}", e),
///     }
/// }
/// ```
///
/// # Automatic Retry
///
/// Some operations like [`LastFmEditClient::edit_scrobble_with_retry`](crate::LastFmEditClient::edit_scrobble_with_retry)
/// automatically handle rate limiting errors by waiting and retrying:
///
/// ```rust,no_run
/// # use lastfm_edit::{LastFmEditClient, ScrobbleEdit};
/// # tokio_test::block_on(async {
/// let mut client = LastFmEditClient::new(Box::new(http_client::native::NativeClient::new()));
/// // client.login(...).await?;
///
/// let edit = ScrobbleEdit::from_track_info("Track", "Album", "Artist", 1640995200);
///
/// // This will automatically retry on rate limits up to 3 times
/// match client.edit_scrobble_with_retry(&edit, 3).await {
///     Ok(response) => println!("Edit completed: {:?}", response),
///     Err(e) => eprintln!("Edit failed after retries: {}", e),
/// }
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// # });
/// ```
#[derive(Error, Debug)]
pub enum LastFmError {
    /// HTTP/network related errors.
    ///
    /// This includes connection failures, timeouts, DNS errors, and other
    /// low-level networking issues.
    #[error("HTTP error: {0}")]
    Http(String),

    /// Authentication failures.
    ///
    /// This occurs when login credentials are invalid, sessions expire,
    /// or authentication is required but not provided.
    ///
    /// # Common Causes
    /// - Invalid username/password
    /// - Expired session cookies
    /// - Account locked or suspended
    /// - Two-factor authentication required
    #[error("Authentication failed: {0}")]
    Auth(String),

    /// CSRF token not found in response.
    ///
    /// This typically indicates that Last.fm's page structure has changed
    /// or that the request was blocked.
    #[error("CSRF token not found")]
    CsrfNotFound,

    /// Failed to parse Last.fm's response.
    ///
    /// This can happen when Last.fm changes their HTML structure or
    /// returns unexpected data formats.
    #[error("Failed to parse response: {0}")]
    Parse(String),

    /// Rate limiting from Last.fm.
    ///
    /// Last.fm has rate limits to prevent abuse. When hit, the client
    /// should wait before making more requests.
    ///
    /// The `retry_after` field indicates how many seconds to wait before
    /// the next request attempt.
    #[error("Rate limited, retry after {retry_after} seconds")]
    RateLimit {
        /// Number of seconds to wait before retrying
        retry_after: u64,
    },

    /// Scrobble edit operation failed.
    ///
    /// This is returned when an edit request is properly formatted and sent,
    /// but Last.fm rejects it for business logic reasons.
    #[error("Edit failed: {0}")]
    EditFailed(String),

    /// File system I/O errors.
    ///
    /// This can occur when saving debug responses or other file operations.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
