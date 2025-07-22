use http_client::native::NativeClient;
use lastfm_edit::{AsyncPaginatedIterator, LastFmEditClient};

/// Test that futures from client operations are Send.
/// This ensures they can be used across await boundaries in async contexts.
#[tokio::test]
async fn test_client_futures_are_send() {
    fn assert_send<T: Send>(_: T) {}

    let client = Box::new(NativeClient::new());
    let lastfm_client = LastFmEditClient::new(client);

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
#[tokio::test]
async fn test_iterator_futures_are_send() {
    fn assert_send<T: Send>(_: T) {}

    let client = Box::new(NativeClient::new());
    let lastfm_client = LastFmEditClient::new(client);

    // Test that recent tracks iterator next() future is Send
    let mut recent_tracks = lastfm_client.recent_tracks();
    let next_future = recent_tracks.next();
    assert_send(next_future);

    // Test that artist tracks iterator next() future is Send
    let mut artist_tracks = lastfm_client.artist_tracks("test");
    let next_future = artist_tracks.next();
    assert_send(next_future);

    // Test that artist albums iterator next() future is Send
    let mut artist_albums = lastfm_client.artist_albums("test");
    let next_future = artist_albums.next();
    assert_send(next_future);
}

/// Test that we can spawn tasks with these futures.
/// This is the most important practical test - futures must be Send to use with tokio::spawn.
#[tokio::test]
async fn test_futures_can_be_spawned() {
    let client = Box::new(NativeClient::new());
    let lastfm_client = LastFmEditClient::new(client);

    // This should compile if futures are Send
    let handle = tokio::spawn(async move {
        let _ = lastfm_client.get_recent_scrobbles(1).await;
        let _ = lastfm_client.get_artist_tracks_page("test", 1).await;
    });

    // Don't actually await the handle since it will fail without proper credentials
    handle.abort();
}

/// Test that iterator usage across await boundaries works.
/// This ensures the iterator future is Send.
#[tokio::test]
async fn test_iterator_across_await_boundaries() {
    let client = Box::new(NativeClient::new());
    let lastfm_client = LastFmEditClient::new(client);

    // This should compile if the iterator and its futures are Send
    let handle = tokio::spawn(async move {
        let mut recent_tracks = lastfm_client.recent_tracks();

        // Simulate some async work that might require Send
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;

        // Try to use the iterator - this requires the iterator future to be Send
        let _ = recent_tracks.next().await;
    });

    // Don't actually await since it will fail without credentials
    handle.abort();
}
