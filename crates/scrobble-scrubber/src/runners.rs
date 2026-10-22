//! Live local runners sharing a store. A held OS lock, rather than a PID or heartbeat,
//! establishes liveness; each runner owns its own atomic status snapshot.

use crate::{EditStep, ExecEnded, ScrubberEvent};
use scrobble_store::durable::atomic_write;
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RunnerKind {
    Desktop,
    Daemon,
    CommandLine,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RunnerActivity {
    #[default]
    Idle,
    Planning,
    Syncing,
    Stopped,
    Starting,
    Editing,
    RateLimited {
        until: Option<u64>,
    },
    Sleeping {
        until: u64,
    },
    Finished {
        ended: ExecEnded,
    },
    Failed,
}

impl RunnerActivity {
    pub fn owns_edits(&self) -> bool {
        matches!(self, Self::Editing | Self::RateLimited { .. })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunnerStatus {
    pub id: Uuid,
    pub pid: u32,
    pub kind: RunnerKind,
    pub started_at: u64,
    pub updated_at: u64,
    pub activity: RunnerActivity,
    pub subject: Option<String>,
    pub step: Option<EditStep>,
    pub applied: u64,
    pub failed: u64,
    pub last_error: Option<String>,
}

#[derive(Debug)]
pub struct RunnerRegistration {
    path: PathBuf,
    _lease: File,
    status: Mutex<RunnerStatus>,
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl RunnerRegistration {
    pub fn register(store_root: &Path, kind: RunnerKind) -> io::Result<Self> {
        let directory = store_root.join("runners");
        std::fs::create_dir_all(&directory)?;
        std::fs::write(directory.join(".gitignore"), "*\n")?;
        let id = Uuid::new_v4();
        let path = directory.join(format!("{id}.json"));
        let lease = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(path.with_extension("live"))?;
        lease.try_lock().map_err(io::Error::other)?;
        let timestamp = now();
        let registration = Self {
            path,
            _lease: lease,
            status: Mutex::new(RunnerStatus {
                id,
                pid: std::process::id(),
                kind,
                started_at: timestamp,
                updated_at: timestamp,
                activity: RunnerActivity::Idle,
                subject: None,
                step: None,
                applied: 0,
                failed: 0,
                last_error: None,
            }),
        };
        registration.publish(&registration.status.lock().unwrap())?;
        Ok(registration)
    }

    pub fn id(&self) -> Uuid {
        self.status.lock().unwrap().id
    }

    fn publish(&self, status: &RunnerStatus) -> io::Result<()> {
        atomic_write(&self.path, &serde_json::to_vec(status)?)
    }

    pub(crate) fn observe(&self, event: &ScrubberEvent) -> io::Result<()> {
        let mut status = self.status.lock().unwrap();
        match event {
            ScrubberEvent::ExecStarted => {
                status.activity = RunnerActivity::Starting;
                status.subject = None;
                status.step = None;
                status.applied = 0;
                status.failed = 0;
                status.last_error = None;
            }
            ScrubberEvent::ExecAcquired => status.activity = RunnerActivity::Editing,
            ScrubberEvent::IntentStarted { subject, .. } => {
                status.subject = Some(subject.to_string());
                status.step = None;
            }
            ScrubberEvent::EditProgress { subject, step, .. } => {
                status.subject = Some(subject.to_string());
                status.step = Some(step.clone());
            }
            ScrubberEvent::EditApplied { .. } => status.applied += 1,
            ScrubberEvent::EditFailed { subject, error, .. } => {
                status.failed += 1;
                status.last_error = Some(format!("{subject}: {error}"));
            }
            ScrubberEvent::ExecutorPaused { reason } => {
                let until = match reason {
                    scrobble_store::PauseReason::RateLimited { until_estimate } => *until_estimate,
                    scrobble_store::PauseReason::Backoff { .. } => None,
                };
                status.activity = RunnerActivity::RateLimited { until };
            }
            ScrubberEvent::ExecutorResumed => status.activity = RunnerActivity::Editing,
            ScrubberEvent::ExecCompleted { report } => {
                status.activity = RunnerActivity::Finished {
                    ended: report.ended.clone(),
                };
                status.applied = report.instances_applied;
                status.failed = report.instances_failed;
                status.step = None;
            }
            ScrubberEvent::ExecFailed { error, .. } => {
                status.activity = RunnerActivity::Failed;
                status.last_error = Some(error.clone());
                status.step = None;
            }
            ScrubberEvent::CycleStarted { .. }
            | ScrubberEvent::Sync(scrobble_store::SyncEvent::SyncStarted { .. })
                if !status.activity.owns_edits() =>
            {
                status.activity = RunnerActivity::Syncing;
            }
            ScrubberEvent::Sync(scrobble_store::SyncEvent::SyncCompleted { .. })
                if status.activity == RunnerActivity::Syncing =>
            {
                status.activity = RunnerActivity::Idle;
            }
            ScrubberEvent::Sync(scrobble_store::SyncEvent::SyncFailed { error }) => {
                status.last_error = Some(error.clone());
                if status.activity == RunnerActivity::Syncing {
                    status.activity = RunnerActivity::Failed;
                }
            }
            ScrubberEvent::Stopped { .. } => status.activity = RunnerActivity::Stopped,
            ScrubberEvent::PlanStarted { .. } if !status.activity.owns_edits() => {
                status.activity = RunnerActivity::Planning;
            }
            ScrubberEvent::PlanCompleted { .. } if status.activity == RunnerActivity::Planning => {
                status.activity = RunnerActivity::Idle;
            }
            ScrubberEvent::Sleeping { seconds } => {
                status.activity = RunnerActivity::Sleeping {
                    until: now().saturating_add(*seconds),
                };
            }
            ScrubberEvent::Error { error } => {
                status.last_error = Some(error.clone());
            }
            _ => return Ok(()),
        }
        status.updated_at = now();
        self.publish(&status)
    }
}

impl Drop for RunnerRegistration {
    fn drop(&mut self) {
        for extension in ["json", "lock", "live"] {
            let _ = std::fs::remove_file(self.path.with_extension(extension));
        }
    }
}

pub fn running_instances(store_root: &Path) -> io::Result<Vec<RunnerStatus>> {
    let directory = store_root.join("runners");
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut runners = Vec::new();
    for entry in entries {
        let path = entry?.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let lease = match OpenOptions::new()
            .read(true)
            .write(true)
            .open(path.with_extension("live"))
        {
            Ok(lease) => lease,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        match lease.try_lock() {
            Ok(()) => {
                // A crashed runner leaves files behind, but its OS lock is released.
                for extension in ["json", "lock", "live"] {
                    let _ = std::fs::remove_file(path.with_extension(extension));
                }
            }
            Err(TryLockError::WouldBlock) => match std::fs::read(&path) {
                Ok(bytes) => runners.push(serde_json::from_slice::<RunnerStatus>(&bytes)?),
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            },
            Err(TryLockError::Error(error)) => return Err(error),
        }
    }
    runners.sort_by_key(|runner| (runner.started_at, runner.id));
    Ok(runners)
}

#[cfg(test)]
#[path = "runners_tests.rs"]
mod tests;
