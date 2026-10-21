use super::*;

#[test]
fn test_session_validity() {
    let valid_session = LastFmEditSession::new(
            "testuser".to_string(),
            vec!["sessionid=.eJy1234567890abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890".to_string()],
            Some("csrf_token_123".to_string()),
            "https://www.last.fm".to_string(),
        );
    assert!(valid_session.is_valid());

    let invalid_session = LastFmEditSession::new(
        "".to_string(),
        vec![],
        None,
        "https://www.last.fm".to_string(),
    );
    assert!(!invalid_session.is_valid());
}

#[test]
fn test_session_serialization() {
    let session = LastFmEditSession::new(
        "testuser".to_string(),
        vec![
            "sessionid=.test123".to_string(),
            "csrftoken=abc".to_string(),
        ],
        Some("csrf_token_123".to_string()),
        "https://www.last.fm".to_string(),
    );

    let json = session.to_json().unwrap();
    let restored_session = LastFmEditSession::from_json(&json).unwrap();

    assert_eq!(session.username, restored_session.username);
    assert_eq!(session.cookies, restored_session.cookies);
    assert_eq!(session.csrf_token, restored_session.csrf_token);
    assert_eq!(session.base_url, restored_session.base_url);
}
