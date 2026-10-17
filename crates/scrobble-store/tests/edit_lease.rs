use scrobble_store::{FsStorage, MemoryStorage, Storage};
use std::process::Command;

#[test]
fn edit_lease_excludes_independent_handles_and_releases_on_drop() {
    let dir = tempfile::tempdir().unwrap();
    let first = FsStorage::open(dir.path()).unwrap();
    let second = FsStorage::open(dir.path()).unwrap();
    let lease = first.try_lease_edits().unwrap().unwrap();
    assert!(first.try_lease_edits().unwrap().is_none());
    assert!(second.try_lease_edits().unwrap().is_none());
    drop(lease);
    assert!(second.try_lease_edits().unwrap().is_some());

    let memory = MemoryStorage::new();
    let lease = memory.try_lease_edits().unwrap().unwrap();
    assert!(memory.try_lease_edits().unwrap().is_none());
    drop(lease);
    assert!(memory.try_lease_edits().unwrap().is_some());
}

#[test]
fn edit_lease_child_process() {
    let Some(path) = std::env::var_os("SCROBBLE_LEASE_TEST_STORE") else {
        return;
    };
    let expected = std::env::var("SCROBBLE_LEASE_TEST_AVAILABLE").unwrap() == "yes";
    let store = FsStorage::open(path).unwrap();
    assert_eq!(store.try_lease_edits().unwrap().is_some(), expected);
}

#[test]
fn edit_lease_excludes_another_process() {
    let dir = tempfile::tempdir().unwrap();
    let store = FsStorage::open(dir.path()).unwrap();
    let run_child = |available| {
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "edit_lease_child_process"])
            .env("SCROBBLE_LEASE_TEST_STORE", dir.path())
            .env("SCROBBLE_LEASE_TEST_AVAILABLE", available)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
    };
    let lease = store.try_lease_edits().unwrap().unwrap();
    run_child("no");
    drop(lease);
    run_child("yes");
    assert!(store.try_lease_edits().unwrap().is_some());
}
