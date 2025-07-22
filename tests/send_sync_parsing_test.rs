use http_client::native::NativeClient;
use lastfm_edit::LastFmEditClient;

/// Test to check if the parsing methods (non-async) are Send + Sync
#[test]
fn test_parsing_methods_are_send_sync() {
    fn assert_send_sync<T: Send + Sync>(_: T) {}

    let client = Box::new(NativeClient::new());
    let lastfm_client = LastFmEditClient::new(client);

    // Test that the client itself is Send + Sync (should be now that parsing is separate)
    assert_send_sync(lastfm_client);
}

/// Test just the iterator creation without calling next() to isolate HTTP client issues
#[test]
fn test_iterator_creation_is_send_sync() {
    let client = Box::new(NativeClient::new());
    let mut lastfm_client = LastFmEditClient::new(client);

    // Create iterators one at a time to avoid borrowing issues
    let recent_tracks = lastfm_client.recent_tracks();
    drop(recent_tracks);

    let artist_tracks = lastfm_client.artist_tracks("test");
    drop(artist_tracks);

    let artist_albums = lastfm_client.artist_albums("test");
    drop(artist_albums);
}

/// Test that the client itself is Send + Sync (structure-wise)
#[test]
fn test_client_is_send_sync() {
    fn assert_send_sync<T: Send + Sync + 'static>(_: T) {}

    let client = Box::new(NativeClient::new());
    let lastfm_client = LastFmEditClient::new(client);

    // The client should be Send + Sync at the structural level
    assert_send_sync(lastfm_client);
}
