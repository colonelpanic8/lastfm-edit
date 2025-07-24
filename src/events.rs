//! # Rate Limiting Events
//!
//! This module provides a broadcast channel system for emitting rate limiting events
//! that consumers can listen to and react appropriately.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

/// Events emitted by the Last.fm client when rate limiting occurs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RateLimitEvent {
    /// Rate limit detected with the number of seconds to wait before retrying.
    Detected {
        /// The timestamp when the rate limit was detected
        timestamp: DateTime<Utc>,
        /// Number of seconds to wait before retrying
        retry_after: u64,
        /// The URL that was rate limited (if available)
        url: Option<String>,
        /// The HTTP status code that indicated rate limiting
        status_code: Option<u16>,
        /// The response body pattern that matched rate limiting (if detected via content)
        matched_pattern: Option<String>,
    },
    /// Rate limit retry is about to begin
    RetryStarting {
        /// The timestamp when the retry is starting
        timestamp: DateTime<Utc>,
        /// The delay in seconds before this retry
        delay_seconds: u64,
        /// The retry attempt number (1-based)
        attempt: u32,
        /// Maximum number of retry attempts
        max_attempts: u32,
    },
    /// Rate limit retry completed successfully
    RetrySucceeded {
        /// The timestamp when the retry succeeded
        timestamp: DateTime<Utc>,
        /// The retry attempt number that succeeded (1-based)
        attempt: u32,
        /// Total delay time in seconds across all retries
        total_delay: u64,
    },
    /// Rate limit retries exhausted
    RetriesExhausted {
        /// The timestamp when retries were exhausted
        timestamp: DateTime<Utc>,
        /// The final retry attempt number that failed (1-based)
        final_attempt: u32,
        /// Total delay time in seconds across all failed retries
        total_delay: u64,
    },
}

/// A handle for receiving rate limiting events from the Last.fm client.
///
/// This receiver can be used to monitor and react to rate limiting events
/// emitted by the client during its operations.
///
/// # Examples
///
/// ```rust,no_run
/// use lastfm_edit::{LastFmEditClientImpl, RateLimitEvent};
/// use tokio::sync::broadcast::error::RecvError;
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     let http_client = http_client::native::NativeClient::new();
///     let client = LastFmEditClientImpl::new(Box::new(http_client));
///     
///     // Get a receiver for rate limit events
///     let mut rate_limit_receiver = client.rate_limit_events();
///     
///     // Spawn a task to handle rate limit events
///     let event_handler = tokio::spawn(async move {
///         loop {
///             match rate_limit_receiver.recv().await {
///                 Ok(RateLimitEvent::Detected { retry_after, url, .. }) => {
///                     println!("Rate limit detected! Retry after {} seconds. URL: {:?}",
///                              retry_after, url);
///                 }
///                 Ok(RateLimitEvent::RetryStarting { delay_seconds, attempt, .. }) => {
///                     println!("Starting retry attempt {} after {} seconds",
///                              attempt, delay_seconds);
///                 }
///                 Ok(RateLimitEvent::RetrySucceeded { attempt, total_delay, .. }) => {
///                     println!("Retry attempt {} succeeded after {} total seconds",
///                              attempt, total_delay);
///                 }
///                 Ok(RateLimitEvent::RetriesExhausted { final_attempt, total_delay, .. }) => {
///                     println!("All {} retry attempts failed after {} total seconds",
///                              final_attempt, total_delay);
///                 }
///                 Err(RecvError::Closed) => {
///                     println!("Rate limit event channel closed");
///                     break;
///                 }
///                 Err(RecvError::Lagged(skipped)) => {
///                     println!("Rate limit event receiver lagged, {} events skipped", skipped);
///                 }
///             }
///         }
///     });
///     
///     // Use the client normally...
///     client.login("username", "password").await?;
///     
///     // The event handler will automatically receive and process rate limit events
///     
///     Ok(())
/// }
/// ```
pub type RateLimitEventReceiver = broadcast::Receiver<RateLimitEvent>;

/// A handle for sending rate limiting events.
///
/// This is used internally by the Last.fm client to emit events when rate limiting occurs.
pub type RateLimitEventSender = broadcast::Sender<RateLimitEvent>;

/// Creates a new broadcast channel for rate limiting events.
///
/// Returns a tuple of (sender, receiver) where the sender is used internally
/// by the client to emit events, and the receiver can be used by consumers
/// to listen for events.
///
/// The channel has a default capacity of 100 events.
pub fn create_rate_limit_channel() -> (RateLimitEventSender, RateLimitEventReceiver) {
    broadcast::channel(100)
}

/// Helper trait for emitting rate limiting events.
///
/// This trait provides convenient methods for emitting different types of
/// rate limiting events through a broadcast sender.
pub trait RateLimitEventEmitter {
    /// Emit a rate limit detected event.
    fn emit_rate_limit_detected(
        &self,
        retry_after: u64,
        url: Option<String>,
        status_code: Option<u16>,
        matched_pattern: Option<String>,
    );

    /// Emit a retry starting event.
    fn emit_retry_starting(&self, delay_seconds: u64, attempt: u32, max_attempts: u32);

    /// Emit a retry succeeded event.
    fn emit_retry_succeeded(&self, attempt: u32, total_delay: u64);

    /// Emit a retries exhausted event.
    fn emit_retries_exhausted(&self, final_attempt: u32, total_delay: u64);
}

impl RateLimitEventEmitter for Option<RateLimitEventSender> {
    fn emit_rate_limit_detected(
        &self,
        retry_after: u64,
        url: Option<String>,
        status_code: Option<u16>,
        matched_pattern: Option<String>,
    ) {
        if let Some(sender) = self {
            let event = RateLimitEvent::Detected {
                timestamp: Utc::now(),
                retry_after,
                url,
                status_code,
                matched_pattern,
            };
            let _ = sender.send(event); // Ignore send errors (no receivers)
        }
    }

    fn emit_retry_starting(&self, delay_seconds: u64, attempt: u32, max_attempts: u32) {
        if let Some(sender) = self {
            let event = RateLimitEvent::RetryStarting {
                timestamp: Utc::now(),
                delay_seconds,
                attempt,
                max_attempts,
            };
            let _ = sender.send(event); // Ignore send errors (no receivers)
        }
    }

    fn emit_retry_succeeded(&self, attempt: u32, total_delay: u64) {
        if let Some(sender) = self {
            let event = RateLimitEvent::RetrySucceeded {
                timestamp: Utc::now(),
                attempt,
                total_delay,
            };
            let _ = sender.send(event); // Ignore send errors (no receivers)
        }
    }

    fn emit_retries_exhausted(&self, final_attempt: u32, total_delay: u64) {
        if let Some(sender) = self {
            let event = RateLimitEvent::RetriesExhausted {
                timestamp: Utc::now(),
                final_attempt,
                total_delay,
            };
            let _ = sender.send(event); // Ignore send errors (no receivers)
        }
    }
}
