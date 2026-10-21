use super::*;

#[test]
fn test_session_path_generation() {
    let path = SessionPersistence::get_session_path("testuser").unwrap();
    assert!(path
        .to_string_lossy()
        .contains("lastfm-edit/users/testuser/session.json"));
}

#[test]
fn test_session_exists_nonexistent() {
    let fake_username = format!("nonexistent_user_{}", std::process::id());
    assert!(!SessionPersistence::session_exists(&fake_username));
}

#[test]
fn explicit_data_dir_keeps_sessions_under_the_app_private_root() {
    let private_files = PathBuf::from("/data/user/0/org.example.app/files");
    let manager = SessionManager::with_data_dir("lastfm-edit", &private_files);

    assert_eq!(
        manager.get_session_path("mobile-user").unwrap(),
        private_files.join("lastfm-edit/users/mobile-user/session.json")
    );
}
