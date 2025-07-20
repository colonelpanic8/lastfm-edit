use thiserror::Error;

#[derive(Error, Debug)]
pub enum LastFmError {
    #[error("HTTP error: {0}")]
    Http(String),

    #[error("Authentication failed: {0}")]
    Auth(String),

    #[error("CSRF token not found")]
    CsrfNotFound,

    #[error("Failed to parse response: {0}")]
    Parse(String),

    #[error("Rate limited, retry after {retry_after} seconds")]
    RateLimit { retry_after: u64 },

    #[error("Edit failed: {0}")]
    EditFailed(String),
}
