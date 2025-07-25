//! Event system for monitoring HTTP client activity.
//!
//! This module provides comprehensive event broadcasting for observing internal
//! HTTP client operations, including request lifecycle, rate limiting detection,
//! and scrobble editing operations.

use crate::edit::ExactScrobbleEdit;
use tokio::sync::{broadcast, watch};

/// Request information for client events
#[derive(Clone, Debug)]
pub struct RequestInfo {
    /// The HTTP method (GET, POST, etc.)
    pub method: String,
    /// The full URI being requested
    pub uri: String,
    /// Query parameters as key-value pairs
    pub query_params: Vec<(String, String)>,
    /// Path without query parameters
    pub path: String,
}

impl RequestInfo {
    /// Create RequestInfo from a URL string and method
    pub fn from_url_and_method(url: &str, method: &str) -> Self {
        // Parse URL manually to avoid adding dependencies
        let (path, query_params) = if let Some(query_start) = url.find('?') {
            let path = url[..query_start].to_string();
            let query_string = &url[query_start + 1..];

            let query_params: Vec<(String, String)> = query_string
                .split('&')
                .filter_map(|pair| {
                    if let Some(eq_pos) = pair.find('=') {
                        let key = &pair[..eq_pos];
                        let value = &pair[eq_pos + 1..];
                        Some((key.to_string(), value.to_string()))
                    } else if !pair.is_empty() {
                        Some((pair.to_string(), String::new()))
                    } else {
                        None
                    }
                })
                .collect();

            (path, query_params)
        } else {
            (url.to_string(), Vec::new())
        };

        // Extract just the path part if it's a full URL
        let path = if path.starts_with("http://") || path.starts_with("https://") {
            if let Some(third_slash) = path[8..].find('/') {
                path[8 + third_slash..].to_string()
            } else {
                "/".to_string()
            }
        } else {
            path
        };

        Self {
            method: method.to_string(),
            uri: url.to_string(),
            query_params,
            path,
        }
    }

    /// Get a short description of the request for logging
    pub fn short_description(&self) -> String {
        let mut desc = format!("{} {}", self.method, self.path);
        if !self.query_params.is_empty() {
            let params: Vec<String> = self
                .query_params
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            if params.len() <= 2 {
                desc.push_str(&format!("?{}", params.join("&")));
            } else {
                desc.push_str(&format!("?{}...", params[0]));
            }
        }
        desc
    }
}

/// Type of rate limiting detected
#[derive(Clone, Debug)]
pub enum RateLimitType {
    /// HTTP 429 Too Many Requests
    Http429,
    /// HTTP 403 Forbidden (likely rate limiting)
    Http403,
    /// Rate limit patterns detected in response body
    ResponsePattern,
}

/// Event type to describe internal HTTP client activity
#[derive(Clone, Debug)]
pub enum ClientEvent {
    /// Request started
    RequestStarted {
        /// Request details
        request: RequestInfo,
    },
    /// Request completed successfully
    RequestCompleted {
        /// Request details
        request: RequestInfo,
        /// HTTP status code
        status_code: u16,
        /// Duration of the request in milliseconds
        duration_ms: u64,
    },
    /// Rate limiting detected with backoff duration in seconds
    RateLimited {
        /// Duration to wait in seconds
        delay_seconds: u64,
        /// Request that triggered the rate limit (if available)
        request: Option<RequestInfo>,
        /// Type of rate limiting detected
        rate_limit_type: RateLimitType,
    },
    /// Scrobble edit attempt completed
    EditAttempted {
        /// The exact scrobble edit that was attempted
        edit: ExactScrobbleEdit,
        /// Whether the edit was successful
        success: bool,
        /// Optional error message if the edit failed
        error_message: Option<String>,
        /// Duration of the edit operation in milliseconds
        duration_ms: u64,
    },
}

/// Type alias for the broadcast receiver
pub type ClientEventReceiver = broadcast::Receiver<ClientEvent>;

/// Type alias for the watch receiver
pub type ClientEventWatcher = watch::Receiver<Option<ClientEvent>>;

/// Shared event broadcasting state that persists across client clones
#[derive(Clone)]
pub struct SharedEventBroadcaster {
    event_tx: broadcast::Sender<ClientEvent>,
    last_event_tx: watch::Sender<Option<ClientEvent>>,
}

impl SharedEventBroadcaster {
    /// Create a new shared event broadcaster
    pub fn new() -> Self {
        let (event_tx, _) = broadcast::channel(100);
        let (last_event_tx, _) = watch::channel(None);

        Self {
            event_tx,
            last_event_tx,
        }
    }

    /// Broadcast an event to all subscribers
    pub fn broadcast_event(&self, event: ClientEvent) {
        let _ = self.event_tx.send(event.clone());
        let _ = self.last_event_tx.send(Some(event));
    }

    /// Subscribe to events
    pub fn subscribe(&self) -> ClientEventReceiver {
        self.event_tx.subscribe()
    }

    /// Get the latest event
    pub fn latest_event(&self) -> Option<ClientEvent> {
        self.last_event_tx.borrow().clone()
    }
}

impl Default for SharedEventBroadcaster {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for SharedEventBroadcaster {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedEventBroadcaster")
            .field("subscribers", &self.event_tx.receiver_count())
            .finish()
    }
}
