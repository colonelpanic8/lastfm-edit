use super::*;
use crate::{ExecReport, ScrubberEventBus, Subject};
use scrobble_store::durable::try_lease;
use std::io::{BufRead, Write};
use std::process::{Command, Stdio};
use std::sync::Arc;

#[test]
fn lists_independent_instances_and_removes_clean_exits() {
    let root = tempfile::tempdir().unwrap();
    let desktop = RunnerRegistration::register(root.path(), RunnerKind::Desktop).unwrap();
    let daemon = RunnerRegistration::register(root.path(), RunnerKind::Daemon).unwrap();
    let statuses = running_instances(root.path()).unwrap();
    assert_eq!(statuses.len(), 2);
    assert_ne!(desktop.id(), daemon.id());
    drop(desktop);
    let statuses = running_instances(root.path()).unwrap();
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].id, daemon.id());
    assert_eq!(statuses[0].kind, RunnerKind::Daemon);
    let other_store = tempfile::tempdir().unwrap();
    assert!(running_instances(other_store.path()).unwrap().is_empty());
}

#[test]
fn status_tracks_actual_acquisition_progress_waits_and_authoritative_completion() {
    let root = tempfile::tempdir().unwrap();
    let runner = Arc::new(RunnerRegistration::register(root.path(), RunnerKind::Daemon).unwrap());
    let bus = ScrubberEventBus::new().with_runner(runner);
    bus.emit(ScrubberEvent::ExecStarted);
    assert!(!running_instances(root.path()).unwrap()[0]
        .activity
        .owns_edits());
    let lease = try_lease(&root.path().join("edits/execution.lock"))
        .unwrap()
        .unwrap();
    bus.emit(ScrubberEvent::ExecAcquired);
    let subject = Subject {
        artist: "Artist".into(),
        track: "Track".into(),
        album: None,
        album_artist: None,
    };
    let id = Uuid::new_v4();
    bus.emit(ScrubberEvent::IntentStarted {
        id,
        subject: subject.clone(),
    });
    bus.emit(ScrubberEvent::EditFailed {
        intent: id,
        subject: subject.clone(),
        instance: scrobble_store::ScrobbleId::new(1, "Artist", "Track"),
        error: "missing forms".into(),
    });
    bus.emit(ScrubberEvent::ExecutorPaused {
        reason: scrobble_store::PauseReason::RateLimited {
            until_estimate: Some(100),
        },
    });
    let status = running_instances(root.path()).unwrap().remove(0);
    assert_eq!(status.subject, Some(subject.to_string()));
    assert_eq!(status.failed, 1);
    assert!(status.last_error.unwrap().contains("missing forms"));
    assert_eq!(
        status.activity,
        RunnerActivity::RateLimited { until: Some(100) }
    );
    assert!(status.activity.owns_edits());
    drop(lease);
    bus.emit(ScrubberEvent::ExecCompleted {
        report: ExecReport {
            instances_applied: 2,
            instances_failed: 1,
            ..Default::default()
        },
    });
    let status = running_instances(root.path()).unwrap().remove(0);
    assert_eq!(status.applied, 2);
    assert!(!status.activity.owns_edits());
}

#[test]
fn corrupted_live_status_is_reported_instead_of_looking_idle() {
    let root = tempfile::tempdir().unwrap();
    let runner = RunnerRegistration::register(root.path(), RunnerKind::Desktop).unwrap();
    std::fs::write(&runner.path, "incomplete").unwrap();
    assert!(running_instances(root.path()).is_err());
}

#[test]
fn killed_process_disappears_without_a_heartbeat_timeout() {
    let root = tempfile::tempdir().unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "runners::tests::child_runner",
            "--ignored",
            "--nocapture",
        ])
        .env("SCRUBBER_TEST_RUNNER_ROOT", root.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = io::BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    loop {
        line.clear();
        assert_ne!(
            stdout.read_line(&mut line).unwrap(),
            0,
            "child exited before registration"
        );
        if line.contains("RUNNER_READY") {
            break;
        }
    }
    let status = running_instances(root.path()).unwrap().remove(0);
    assert_eq!(status.pid, child.id());
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(running_instances(root.path()).unwrap().is_empty());
    assert!(!root
        .path()
        .join("runners")
        .join(format!("{}.json", status.id))
        .exists());
}

#[test]
#[ignore = "subprocess helper for killed_process_disappears_without_a_heartbeat_timeout"]
fn child_runner() {
    let root = std::env::var_os("SCRUBBER_TEST_RUNNER_ROOT").unwrap();
    let _runner = RunnerRegistration::register(Path::new(&root), RunnerKind::CommandLine).unwrap();
    println!("RUNNER_READY");
    io::stdout().flush().unwrap();
    let mut line = String::new();
    io::stdin().read_line(&mut line).unwrap();
}
