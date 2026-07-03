//! Tests for `RateLimitBehavior` (blocking vs. non-blocking rate-limit handling)
//! and `LastFmEditClientImpl::non_blocking()` sharing semantics.

use lastfm_edit::{
    ClientConfig, ClientEvent, ExactScrobbleEdit, LastFmEditClientImpl, LastFmEditSession,
    LastFmError, RateLimitBehavior, RateLimitState,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Response body matching the default "you're requesting too many pages" rate-limit pattern.
const RATE_LIMIT_BODY: &str = "<html><body><p>you're requesting too many pages</p></body></html>";

/// A scripted HTTP client that always returns the same canned response and counts requests.
#[derive(Debug)]
struct ScriptedClient {
    status: u16,
    body: String,
    requests: Arc<AtomicUsize>,
}

impl ScriptedClient {
    fn rate_limited() -> (Self, Arc<AtomicUsize>) {
        let requests = Arc::new(AtomicUsize::new(0));
        (
            Self {
                status: 200,
                body: RATE_LIMIT_BODY.to_string(),
                requests: requests.clone(),
            },
            requests,
        )
    }
}

#[async_trait::async_trait]
impl http_client::HttpClient for ScriptedClient {
    async fn send(
        &self,
        _req: http_client::Request,
    ) -> std::result::Result<http_client::Response, http_types::Error> {
        self.requests.fetch_add(1, Ordering::SeqCst);
        let mut response = http_types::Response::new(self.status);
        response.set_body(self.body.clone());
        Ok(response)
    }
}

fn create_test_session() -> LastFmEditSession {
    LastFmEditSession::new(
        "test_user".to_string(),
        vec!["sessionid=.test_session_id_12345".to_string()],
        Some("test_csrf_token".to_string()),
        "https://www.last.fm".to_string(),
    )
}

fn rate_limited_client(config: ClientConfig) -> (LastFmEditClientImpl, Arc<AtomicUsize>) {
    let (http_client, requests) = ScriptedClient::rate_limited();
    let client = LastFmEditClientImpl::from_session_with_client_config(
        Box::new(http_client),
        create_test_session(),
        config,
    );
    (client, requests)
}

fn sample_exact_edit() -> ExactScrobbleEdit {
    ExactScrobbleEdit::new(
        "Track".to_string(),
        "Album".to_string(),
        "Artist".to_string(),
        "Artist".to_string(),
        "New Track".to_string(),
        "Album".to_string(),
        "Artist".to_string(),
        "Artist".to_string(),
        1_640_995_200,
        false,
    )
}

#[test]
fn rate_limit_behavior_config_plumbing() {
    // Default is BlockAndRetry everywhere ClientConfig is constructed via Default.
    assert_eq!(
        ClientConfig::default().rate_limit_behavior,
        RateLimitBehavior::BlockAndRetry
    );
    assert_eq!(
        ClientConfig::for_testing().rate_limit_behavior,
        RateLimitBehavior::BlockAndRetry
    );
    assert_eq!(
        ClientConfig::with_retries_disabled().rate_limit_behavior,
        RateLimitBehavior::BlockAndRetry
    );
    assert_eq!(
        ClientConfig::minimal().rate_limit_behavior,
        RateLimitBehavior::BlockAndRetry
    );

    // Builder sets the behavior.
    let config =
        ClientConfig::for_testing().with_rate_limit_behavior(RateLimitBehavior::ReturnError);
    assert_eq!(config.rate_limit_behavior, RateLimitBehavior::ReturnError);
}

#[test_log::test(tokio::test)]
async fn block_and_retry_with_retries_disabled_still_errors() {
    // Current behavior preserved: with retries disabled, a pattern-detected rate limit
    // surfaces as Err(RateLimit) after a single request.
    let (client, requests) = rate_limited_client(ClientConfig::with_retries_disabled());

    let err = client.get_recent_tracks_page(1).await.unwrap_err();
    assert!(matches!(err, LastFmError::RateLimit { .. }));
    assert_eq!(requests.load(Ordering::SeqCst), 1);
}

#[test_log::test(tokio::test)]
async fn block_and_retry_mode_retries_internally() {
    // With retries enabled (zero-delay test config), BlockAndRetry drives the internal
    // retry loop: 1 initial attempt + max_retries retries.
    let (client, requests) = rate_limited_client(ClientConfig::for_testing());

    let err = client.get_recent_tracks_page(1).await.unwrap_err();
    assert!(matches!(err, LastFmError::RateLimit { .. }));
    assert_eq!(requests.load(Ordering::SeqCst), 4); // 1 + 3 retries
}

#[test_log::test(tokio::test)]
async fn return_error_mode_fails_fast_and_reports_state() {
    // Even with retries enabled in the config, ReturnError mode makes exactly one attempt
    // and surfaces Err(RateLimit) — no internal sleeping or retrying.
    let (client, requests) = rate_limited_client(
        ClientConfig::for_testing().with_rate_limit_behavior(RateLimitBehavior::ReturnError),
    );

    let err = client.get_recent_tracks_page(1).await.unwrap_err();
    assert!(matches!(err, LastFmError::RateLimit { .. }));
    assert_eq!(requests.load(Ordering::SeqCst), 1);

    // The shared rate-limit state still reflects the detection.
    let state = client.rate_limit_state();
    assert!(matches!(state, RateLimitState::RateLimited { .. }));
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    assert!(state.is_rate_limited_at(now));
}

#[test_log::test(tokio::test)]
async fn return_error_mode_propagates_rate_limit_from_edit_scrobble_single() {
    let (client, requests) = rate_limited_client(
        ClientConfig::for_testing().with_rate_limit_behavior(RateLimitBehavior::ReturnError),
    );

    let err = client
        .edit_scrobble_single(&sample_exact_edit(), 3)
        .await
        .unwrap_err();
    assert!(matches!(err, LastFmError::RateLimit { .. }));
    // Single attempt: only the CSRF-form GET was issued before the rate limit surfaced.
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    assert!(matches!(
        client.rate_limit_state(),
        RateLimitState::RateLimited { .. }
    ));
}

#[test_log::test(tokio::test)]
async fn block_and_retry_mode_folds_rate_limit_into_edit_response() {
    // Contrast with ReturnError: the blocking path reports a failed EditResponse
    // instead of an Err once retries are exhausted.
    let (client, _requests) = rate_limited_client(ClientConfig::for_testing());

    let response = client
        .edit_scrobble_single(&sample_exact_edit(), 3)
        .await
        .expect("blocking mode should not surface rate-limit errors");
    assert!(!response.success());
}

#[test_log::test(tokio::test)]
async fn return_error_mode_propagates_rate_limit_from_delete_scrobble() {
    let (client, requests) = rate_limited_client(
        ClientConfig::for_testing().with_rate_limit_behavior(RateLimitBehavior::ReturnError),
    );

    let err = client
        .delete_scrobble("Artist", "Track", 1_640_995_200)
        .await
        .unwrap_err();
    assert!(matches!(err, LastFmError::RateLimit { .. }));
    assert_eq!(requests.load(Ordering::SeqCst), 1);
}

#[test_log::test(tokio::test)]
async fn block_and_retry_mode_folds_rate_limit_into_delete_result() {
    // Contrast with ReturnError: the blocking path returns Ok(false) once retries
    // are exhausted.
    let (client, _requests) = rate_limited_client(ClientConfig::for_testing());

    let deleted = client
        .delete_scrobble("Artist", "Track", 1_640_995_200)
        .await
        .expect("blocking mode should not surface rate-limit errors");
    assert!(!deleted);
}

#[test_log::test(tokio::test)]
async fn non_blocking_clone_shares_broadcaster_and_session() {
    let (client, requests) = rate_limited_client(ClientConfig::default());
    let non_blocking = client.non_blocking();

    // Sessions are Arc-shared, so both clients report identical session state.
    assert_eq!(client.get_session(), non_blocking.get_session());
    assert_eq!(client.username(), non_blocking.username());

    // Subscribe on the ORIGINAL client, then trigger activity on the non-blocking clone.
    let mut events = client.subscribe();

    let err = non_blocking.get_recent_tracks_page(1).await.unwrap_err();
    assert!(matches!(err, LastFmError::RateLimit { .. }));
    // Non-blocking clone made exactly one attempt despite the parent's default retry config.
    assert_eq!(requests.load(Ordering::SeqCst), 1);

    // Events emitted by the clone are visible on the parent's subscription.
    let mut saw_request_started = false;
    let mut saw_rate_limited = false;
    while let Ok(event) = events.try_recv() {
        match event {
            ClientEvent::RequestStarted { .. } => saw_request_started = true,
            ClientEvent::RateLimited { .. } => saw_rate_limited = true,
            _ => {}
        }
    }
    assert!(saw_request_started);
    assert!(saw_rate_limited);

    // The shared rate-limit state is observable from both clients.
    assert!(matches!(
        client.rate_limit_state(),
        RateLimitState::RateLimited { .. }
    ));
    assert!(matches!(
        non_blocking.rate_limit_state(),
        RateLimitState::RateLimited { .. }
    ));
}
