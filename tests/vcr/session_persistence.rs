use lastfm_edit::{LastFmEditClientImpl, SessionManager};

#[test_log::test(tokio::test)]
async fn test_session_manager_save_load() {
    // Create a temporary session manager for testing
    let session_manager = SessionManager::new("lastfm-edit-test");

    let username = "test_user";

    // Clean up any existing session first
    let _ = session_manager.remove_session(username);

    // Test that no session exists initially
    assert!(
        !session_manager.session_exists(username),
        "No session should exist initially"
    );

    // Create a dummy session to save
    let test_session = lastfm_edit::LastFmEditSession::new(
        username.to_string(),
        vec!["test_cookie=value".to_string()],
        Some("test_csrf_token".to_string()),
        "https://www.last.fm".to_string(),
    );

    // Test saving the session
    session_manager
        .save_session(&test_session)
        .expect("Should be able to save session");

    // Test that session exists after saving
    assert!(
        session_manager.session_exists(username),
        "Session should exist after saving"
    );

    // Test loading the session
    let loaded_session = session_manager
        .load_session(username)
        .expect("Should be able to load saved session");

    // Verify loaded session matches what we saved
    assert_eq!(loaded_session.username, username);
    assert_eq!(
        loaded_session.cookies,
        vec!["test_cookie=value".to_string()]
    );
    assert_eq!(
        loaded_session.csrf_token,
        Some("test_csrf_token".to_string())
    );
    assert_eq!(loaded_session.base_url, "https://www.last.fm");

    // Test listing saved users
    let saved_users = session_manager
        .list_saved_users()
        .expect("Should be able to list saved users");
    assert!(
        saved_users.contains(&username.to_string()),
        "Username should be in saved users list"
    );

    // Clean up
    session_manager
        .remove_session(username)
        .expect("Should be able to remove session");

    // Verify session is removed
    assert!(
        !session_manager.session_exists(username),
        "Session should be removed"
    );
}

#[test_log::test(tokio::test)]
async fn test_session_manager_custom_app_name() {
    let custom_app = "my-custom-app";
    let session_manager = SessionManager::new(custom_app);

    // Test that app name is set correctly
    assert_eq!(session_manager.app_name(), custom_app);

    let username = "test_user_custom";

    // Test getting session path includes custom app name
    let session_path = session_manager
        .get_session_path(username)
        .expect("Should be able to get session path");

    let path_str = session_path.to_string_lossy();
    assert!(
        path_str.contains(custom_app),
        "Session path should contain custom app name"
    );
    assert!(
        path_str.contains(username),
        "Session path should contain username"
    );
    assert!(
        path_str.ends_with("session.json"),
        "Session path should end with session.json"
    );
}

#[test_log::test(tokio::test)]
async fn test_client_from_session() {
    // Test creating a client from a saved session
    let session = lastfm_edit::LastFmEditSession::new(
        "test_user".to_string(),
        vec!["test_cookie=value".to_string()],
        Some("test_csrf_token".to_string()),
        "https://www.last.fm".to_string(),
    );

    // Create client from session
    let http_client = Box::new(http_client::native::NativeClient::new());
    let client = LastFmEditClientImpl::from_session(http_client, session.clone());

    // Verify client has correct session data
    let client_session = client.get_session();
    assert_eq!(client_session.username, session.username);
    assert_eq!(client_session.cookies, session.cookies);
    assert_eq!(client_session.csrf_token, session.csrf_token);
    assert_eq!(client_session.base_url, session.base_url);
}
