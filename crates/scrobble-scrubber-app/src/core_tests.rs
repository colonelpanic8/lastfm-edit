use super::{default_config_path, default_store_root, parse_pass_field};
use std::path::Path;

const ENTRY: &str = "secret\nuser: IvanMalison\nurl: https://last.fm\napi-key: key\n";

#[test]
fn parses_password_from_first_line() {
    assert_eq!(parse_pass_field(ENTRY, None).as_deref(), Some("secret"));
}

#[test]
fn parses_named_fields_case_insensitively() {
    assert_eq!(
        parse_pass_field(ENTRY, Some("USER")).as_deref(),
        Some("IvanMalison")
    );
    assert_eq!(
        parse_pass_field(ENTRY, Some("api-key")).as_deref(),
        Some("key")
    );
}

#[test]
fn mobile_paths_stay_beneath_the_private_files_directory() {
    let files_dir = Path::new("/data/user/0/org.example.app/files");

    assert_eq!(
        default_config_path(Some(files_dir)).unwrap(),
        files_dir.join("scrobble-scrubber/config.toml")
    );
    assert_eq!(
        default_store_root("listener", Some(files_dir)).unwrap(),
        files_dir.join("scrobble-store/listener")
    );
}
