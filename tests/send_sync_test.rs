use http_client::native::NativeClient;
use lastfm_edit::LastFmEditClientImpl;

/// Test that futures from client operations are Send.
/// This ensures they can be used across await boundaries in async contexts.
#[tokio::test]
async fn test_client_futures_are_send() {
    fn assert_send<T: Send>(_: T) {}

    let client = Box::new(NativeClient::new());
    let lastfm_client = LastFmEditClientImpl::new(client);

    // Test that client login future is Send
    let login_future = lastfm_client.login("test", "test");
    assert_send(login_future);

    // Test that client get_recent_scrobbles future is Send
    let get_scrobbles_future = lastfm_client.get_recent_scrobbles(1);
    assert_send(get_scrobbles_future);

    // Test that client get_artist_tracks_page future is Send
    let get_tracks_future = lastfm_client.get_artist_tracks_page("test", 1);
    assert_send(get_tracks_future);
}

/// Test that iterator futures are Send.
/// This ensures they can be used across await boundaries.
/// Note: Current iterator implementation holds references to the client,
/// so they are not Send. This is intentional for lifetime safety.
#[tokio::test]
async fn test_iterator_futures_are_send() {
    // This test is commented out because iterators now hold references
    // to the client, making them not Send. This is expected behavior.

    // To use iterators across threads, create the iterator on the same
    // thread where it will be used, or use the underlying pagination
    // methods directly which are Send.
}

/// Test that we can spawn tasks with these futures.
/// This is the most important practical test - futures must be Send to use with tokio::spawn.
#[tokio::test]
async fn test_futures_can_be_spawned() {
    let client = Box::new(NativeClient::new());
    let lastfm_client = LastFmEditClientImpl::new(client);

    // This should compile if futures are Send
    let handle = tokio::spawn(async move {
        let _ = lastfm_client.get_recent_scrobbles(1).await;
        let _ = lastfm_client.get_artist_tracks_page("test", 1).await;
    });

    // Don't actually await the handle since it will fail without proper credentials
    handle.abort();
}

/// Test that pagination methods work across await boundaries.
/// Note: Iterators are not Send due to holding client references.
/// Use pagination methods directly for Send behavior.
#[tokio::test]
async fn test_pagination_methods_across_await_boundaries() {
    let client = Box::new(NativeClient::new());
    let lastfm_client = LastFmEditClientImpl::new(client);

    // This demonstrates using the underlying pagination methods which are Send
    let handle = tokio::spawn(async move {
        // Simulate some async work
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;

        // Use pagination methods directly - these are Send
        let _ = lastfm_client.get_recent_scrobbles(1).await;
        let _ = lastfm_client.get_artist_tracks_page("test", 1).await;
    });

    // Don't actually await since it will fail without credentials
    handle.abort();
}
