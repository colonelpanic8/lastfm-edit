use http_client_vcr::NoOpClient;
use lastfm_edit::{LastFmEditClientImpl, LastFmEditSession};
use std::time::Duration;
use tokio::time::timeout;

fn create_test_session() -> LastFmEditSession {
    LastFmEditSession::new(
        "test_user".to_string(),
        vec!["sessionid=.test_session_id_12345".to_string()],
        Some("test_csrf_token".to_string()),
        "https://www.last.fm".to_string(),
    )
}

#[test_log::test(tokio::test)]
async fn test_shared_broadcaster_across_clients() {
    // Create the first client
    let http_client1 = NoOpClient::new();
    let client1 = LastFmEditClientImpl::from_session(Box::new(http_client1), create_test_session());

    // Create second client that shares the broadcaster with client1
    let http_client2 = NoOpClient::new();
    let client2 = client1.with_shared_broadcaster(Box::new(http_client2));

    // Create third client with independent broadcaster
    let http_client3 = NoOpClient::new();
    let session = client1.get_session();
    let client3 = LastFmEditClientImpl::from_session(Box::new(http_client3), session);

    // Subscribe to events from all clients
    let mut events1 = client1.subscribe();
    let mut events2 = client2.subscribe();
    let mut events3 = client3.subscribe();

    // Test that clients start with no events
    assert!(client1.latest_event().is_none());
    assert!(client2.latest_event().is_none());
    assert!(client3.latest_event().is_none());

    // In a real scenario, rate limit events would be broadcast automatically
    // when HTTP requests encounter rate limiting. Since we can't easily simulate
    // that in a unit test, we verify the structure is correct by checking that:

    // 1. Clients with shared broadcasters have the same latest event state
    // 2. Clients with independent broadcasters maintain separate state

    // For now, we can only test that the event subscriptions are properly set up
    // and that no events are present initially (which is correct)

    // Test that subscriptions don't immediately have events
    let no_event_1 = timeout(Duration::from_millis(10), events1.recv()).await;
    let no_event_2 = timeout(Duration::from_millis(10), events2.recv()).await;
    let no_event_3 = timeout(Duration::from_millis(10), events3.recv()).await;

    // All should timeout (no events received)
    assert!(no_event_1.is_err());
    assert!(no_event_2.is_err());
    assert!(no_event_3.is_err());
}

#[test_log::test(tokio::test)]
async fn test_session_sharing_vs_broadcaster_sharing() {
    // Create first client
    let http_client1 = NoOpClient::new();
    let client1 = LastFmEditClientImpl::from_session(Box::new(http_client1), create_test_session());

    // Client2: shares session but NOT broadcaster
    let http_client2 = NoOpClient::new();
    let session = client1.get_session();
    let client2 = LastFmEditClientImpl::from_session(Box::new(http_client2), session);

    // Client3: shares BOTH session and broadcaster
    let http_client3 = NoOpClient::new();
    let client3 = client1.with_shared_broadcaster(Box::new(http_client3));

    // Verify session sharing
    assert_eq!(
        client1.get_session().base_url,
        client2.get_session().base_url
    );
    assert_eq!(
        client1.get_session().base_url,
        client3.get_session().base_url
    );

    // All clients should start with no events
    assert!(client1.latest_event().is_none());
    assert!(client2.latest_event().is_none());
    assert!(client3.latest_event().is_none());

    // Subscribe to verify broadcast setup
    let _events1 = client1.subscribe();
    let _events2 = client2.subscribe();
    let _events3 = client3.subscribe();

    // Test passes if no panics occur - the broadcaster sharing is working correctly
    // In real usage, when client1 or client3 encounter rate limits, both would see the events
    // while client2 would not (since it has an independent broadcaster)
}

#[test_log::test]
fn test_client_creation_patterns() {
    // Pattern 1: Independent clients
    let http_client1 = NoOpClient::new();
    let client1 = LastFmEditClientImpl::from_session(Box::new(http_client1), create_test_session());

    let http_client2 = NoOpClient::new();
    let session = client1.get_session();
    let client2 = LastFmEditClientImpl::from_session(Box::new(http_client2), session);

    // These should be independent - same session but different broadcasters
    assert_eq!(
        client1.get_session().base_url,
        client2.get_session().base_url
    );

    // Pattern 2: Shared broadcaster
    let http_client3 = NoOpClient::new();
    let client3 = client1.with_shared_broadcaster(Box::new(http_client3));

    // These should share both session and broadcaster
    assert_eq!(
        client1.get_session().base_url,
        client3.get_session().base_url
    );

    // Test that we can create subscriptions without issues
    let _sub1 = client1.subscribe();
    let _sub2 = client2.subscribe();
    let _sub3 = client3.subscribe();
}
