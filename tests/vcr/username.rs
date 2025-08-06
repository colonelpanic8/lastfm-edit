use lastfm_edit::{LastFmEditClientImpl, LastFmEditSession};

#[test_log::test(tokio::test)]
async fn test_username() {
    // Create a client with a test session (no HTTP requests needed for username())
    let session = LastFmEditSession::new(
        "TestUser".to_string(),
        vec!["test_cookie=value".to_string()],
        Some("test_csrf_token".to_string()),
        "https://www.last.fm".to_string(),
    );

    let http_client = Box::new(http_client::native::NativeClient::new());
    let client = LastFmEditClientImpl::from_session(http_client, session);

    // Test that we can get the username from the client
    let username = client.username();
    assert_eq!(username, "TestUser", "Username should match expected value");
}
