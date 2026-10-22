//! UI-side state types and pure functions (no dioxus imports — unit-testable as-is).

use chrono::{DateTime, Local};
use lastfm_edit::ScrobbleEdit;
use scrobble_scrubber::{
    EditStep, ExecEnded, ExecFailureKind, ExecReport, PlanReport, ScrubberEvent, Subject,
};
use scrobble_store::{PauseReason, ScrobbleId, SyncEvent};
use std::collections::VecDeque;
use uuid::Uuid;

pub const LOG_CAP: usize = 500;

#[derive(Clone, Debug)]
pub struct LogEntry {
    pub at: DateTime<Local>,
    pub icon: &'static str,
    pub summary: String,
    pub is_error: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EditFailure {
    pub subject: String,
    pub instance: ScrobbleId,
    pub error: String,
}

/// The intent an execute pass is currently working through.
#[derive(Clone, Debug, PartialEq)]
pub struct CurrentIntent {
    pub id: Uuid,
    /// Subject rendered via `Display` ("Artist — Track [Album]").
    pub subject: String,
    pub instances: usize,
}

/// Live counters for the in-flight execute pass.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PassProgress {
    pub applied: u64,
    pub failed: u64,
    pub intents_done: u64,
    pub current: Option<CurrentIntent>,
    pub current_edit: Option<CurrentEdit>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CurrentEdit {
    pub instance: ScrobbleId,
    pub step: EditStep,
}

/// Executor state machine. Progress survives pause/resume within a pass, so a
/// rate-limit pause doesn't erase what the pass already did.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum PassState {
    #[default]
    Idle,
    Running(PassProgress),
    Paused {
        progress: PassProgress,
        until: Option<u64>,
    },
    Failed {
        progress: PassProgress,
        error: String,
        kind: ExecFailureKind,
    },
}

impl PassState {
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Running(_) | Self::Paused { .. })
    }

    /// The live progress, whether running or paused.
    fn progress_mut(&mut self) -> Option<&mut PassProgress> {
        match self {
            PassState::Idle | PassState::Failed { .. } => None,
            PassState::Running(progress) | PassState::Paused { progress, .. } => Some(progress),
        }
    }
}

/// Where the continuous sync → plan → execute loop is within a cycle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CyclePhase {
    Running,
    Sleeping { seconds: u64 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CycleInfo {
    pub n: u64,
    pub phase: CyclePhase,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum PlanStatus {
    #[default]
    Idle,
    Planning {
        subjects: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum SyncStatus {
    /// No API key configured; sync controls disabled.
    Unavailable,
    #[default]
    Idle,
    Syncing,
    RateLimited {
        until: Option<u64>,
    },
}

/// The reducible UI state: everything the event stream drives, as plain data.
///
/// The broadcast bus is lossy under lag (cap 4096), so the pass counters here can
/// drift from reality; `ExecCompleted`'s authoritative report and the queue reload
/// self-heal at pass end.
#[derive(Clone, Debug, Default)]
pub struct UiState {
    pub log: VecDeque<LogEntry>,
    pub pass: PassState,
    /// Present while continuous mode has emitted cycle events.
    pub continuous: Option<CycleInfo>,
    pub plan: PlanStatus,
    pub last_plan: Option<PlanReport>,
    pub last_exec: Option<ExecReport>,
    pub execution_started_at: Option<i64>,
    pub execution_updated_at: Option<i64>,
    pub execution_stop_requested: bool,
    pub last_edit_failure: Option<EditFailure>,
    pub sync: SyncStatus,
    /// Bumped whenever queue contents may have changed; views reload on change.
    pub queue_epoch: u64,
    /// Bumped when a sync completes or new scrobbles are discovered; the Sync view's
    /// recent-scrobbles feed reloads (resetting to the first page) on change.
    pub sync_epoch: u64,
    pub error: Option<String>,
}

impl UiState {
    pub fn fail_execution(&mut self, error: String, kind: ExecFailureKind) {
        let progress = match std::mem::take(&mut self.pass) {
            PassState::Running(progress)
            | PassState::Paused { progress, .. }
            | PassState::Failed { progress, .. } => progress,
            PassState::Idle => PassProgress::default(),
        };
        self.pass = PassState::Failed {
            progress,
            error,
            kind,
        };
        self.execution_stop_requested = false;
        self.queue_epoch += 1;
    }
}

/// Apply one scrubber event to the UI state. Pure; the signal write-back lives in the
/// event pump.
pub fn reduce(state: &mut UiState, event: &ScrubberEvent) {
    match event {
        ScrubberEvent::PlanStarted { .. } => {
            state.plan = PlanStatus::Planning { subjects: 0 };
        }
        ScrubberEvent::SubjectsFound { count, .. } => {
            if let PlanStatus::Planning { subjects } = &mut state.plan {
                *subjects += *count as u64;
            }
        }
        ScrubberEvent::PlanCompleted { report } => {
            state.plan = PlanStatus::Idle;
            state.last_plan = Some(report.clone());
            state.queue_epoch += 1;
        }
        ScrubberEvent::IntentQueued { .. }
        | ScrubberEvent::IntentApproved { .. }
        | ScrubberEvent::IntentRejected { .. }
        | ScrubberEvent::IntentReinstated { .. } => {
            state.queue_epoch += 1;
        }
        ScrubberEvent::ExecStarted => {
            state.pass = PassState::Running(PassProgress::default());
            state.execution_stop_requested = false;
            state.last_edit_failure = None;
            let now = Local::now().timestamp();
            state.execution_started_at = Some(now);
            state.execution_updated_at = Some(now);
        }
        ScrubberEvent::IntentStarted { id, subject } => {
            if let Some(progress) = state.pass.progress_mut() {
                progress.current = Some(CurrentIntent {
                    id: *id,
                    subject: subject.to_string(),
                    instances: 0,
                });
                progress.current_edit = None;
            }
            state.execution_updated_at = Some(Local::now().timestamp());
        }
        ScrubberEvent::EditProgress { instance, step, .. } => {
            if let Some(progress) = state.pass.progress_mut() {
                progress.current_edit = Some(CurrentEdit {
                    instance: instance.clone(),
                    step: step.clone(),
                });
            }
            state.execution_updated_at = Some(Local::now().timestamp());
        }
        ScrubberEvent::IntentExpanded {
            id,
            subject,
            instances,
        } => {
            if let Some(progress) = state.pass.progress_mut() {
                progress.current = Some(CurrentIntent {
                    id: *id,
                    subject: subject.to_string(),
                    instances: *instances,
                });
            }
            state.queue_epoch += 1;
        }
        ScrubberEvent::EditApplied { .. } => {
            state.execution_updated_at = Some(Local::now().timestamp());
            if let Some(progress) = state.pass.progress_mut() {
                progress.applied += 1;
            }
            state.queue_epoch += 1;
        }
        ScrubberEvent::EditFailed {
            subject,
            instance,
            error,
            ..
        } => {
            state.execution_updated_at = Some(Local::now().timestamp());
            state.last_edit_failure = Some(EditFailure {
                subject: subject.to_string(),
                instance: instance.clone(),
                error: error.clone(),
            });
            if let Some(progress) = state.pass.progress_mut() {
                progress.failed += 1;
            }
            state.queue_epoch += 1;
        }
        ScrubberEvent::IntentCompleted { .. } => {
            if let Some(progress) = state.pass.progress_mut() {
                progress.intents_done += 1;
                // The executor is serial, so the completed intent is always the current
                // one; clear unconditionally rather than matching ids.
                progress.current = None;
                progress.current_edit = None;
            }
            state.queue_epoch += 1;
        }
        ScrubberEvent::ExecCompleted { report } => {
            state.pass = PassState::Idle;
            state.execution_stop_requested = false;
            state.last_exec = Some(report.clone());
            state.queue_epoch += 1;
        }
        ScrubberEvent::ExecFailed { error, kind } => {
            state.fail_execution(error.clone(), *kind);
        }
        ScrubberEvent::Stopped { reason } if state.pass.is_active() => {
            state.fail_execution(
                format!("The scrubber backend stopped: {reason}. Reopen the app to reconnect."),
                ExecFailureKind::Other,
            );
        }
        ScrubberEvent::ExecutorPaused { reason } => {
            let until = match reason {
                PauseReason::RateLimited { until_estimate } => *until_estimate,
                PauseReason::Backoff { .. } => None,
            };
            // Carry the pass's progress into Paused instead of dropping it.
            let progress = match std::mem::take(&mut state.pass) {
                PassState::Running(progress) | PassState::Paused { progress, .. } => progress,
                PassState::Idle | PassState::Failed { .. } => PassProgress::default(),
            };
            state.pass = PassState::Paused { progress, until };
        }
        ScrubberEvent::ExecutorResumed => {
            let progress = match std::mem::take(&mut state.pass) {
                PassState::Running(progress) | PassState::Paused { progress, .. } => progress,
                PassState::Idle | PassState::Failed { .. } => PassProgress::default(),
            };
            state.pass = PassState::Running(progress);
        }
        ScrubberEvent::CycleStarted { n } => {
            state.continuous = Some(CycleInfo {
                n: *n,
                phase: CyclePhase::Running,
            });
        }
        ScrubberEvent::CycleCompleted { n } => {
            // Phase stays Running: the Sleeping event follows immediately.
            state.continuous = Some(CycleInfo {
                n: *n,
                phase: CyclePhase::Running,
            });
        }
        ScrubberEvent::Sleeping { seconds } => {
            if let Some(info) = &mut state.continuous {
                info.phase = CyclePhase::Sleeping { seconds: *seconds };
            }
        }
        ScrubberEvent::Error { error } => {
            if matches!(&state.pass, PassState::Failed { error: stopped, .. } if stopped == error) {
                return;
            }
            state.error = Some(error.clone());
        }
        ScrubberEvent::Sync(sync) => match sync {
            SyncEvent::SyncStarted { .. } => state.sync = SyncStatus::Syncing,
            SyncEvent::SyncCompleted { .. } => {
                state.sync = SyncStatus::Idle;
                // New scrobbles may have landed; the Sync feed should refresh.
                state.sync_epoch += 1;
            }
            SyncEvent::SyncFailed { .. } => {
                state.sync = SyncStatus::Idle;
            }
            SyncEvent::ScrobblesDiscovered { .. } => {
                state.sync_epoch += 1;
            }
            SyncEvent::SyncPaused {
                reason: PauseReason::RateLimited { until_estimate },
            } => {
                state.sync = SyncStatus::RateLimited {
                    until: *until_estimate,
                };
            }
            SyncEvent::SyncResumed => state.sync = SyncStatus::Syncing,
            _ => {}
        },
        _ => {}
    }

    if let Some((icon, summary)) = event_summary(event) {
        state.log.push_back(LogEntry {
            at: Local::now(),
            icon,
            summary,
            is_error: matches!(
                event,
                ScrubberEvent::EditFailed { .. }
                    | ScrubberEvent::ExecFailed { .. }
                    | ScrubberEvent::Error { .. }
                    | ScrubberEvent::Sync(SyncEvent::SyncFailed { .. })
            ),
        });
        while state.log.len() > LOG_CAP {
            state.log.pop_front();
        }
    }
}

/// One-line rendering of an event for the activity log. `None` = too noisy to log.
pub fn event_summary(event: &ScrubberEvent) -> Option<(&'static str, String)> {
    Some(match event {
        ScrubberEvent::PlanStarted { feed } => ("▶", format!("planning {feed}")),
        ScrubberEvent::SubjectsFound { count, .. } if *count > 0 => {
            ("·", format!("analyzing {count} subject(s)"))
        }
        ScrubberEvent::SubjectsFound { .. } | ScrubberEvent::SubjectAnalyzed { .. } => return None,
        ScrubberEvent::SuggestionReported {
            subject, summary, ..
        } => ("◇", format!("would edit {subject}: {summary}")),
        ScrubberEvent::IntentQueued { subject, state, .. } => (
            "＋",
            format!(
                "queued {subject}{}",
                if matches!(state, scrobble_scrubber::IntentState::AwaitingApproval) {
                    " (awaiting approval)"
                } else {
                    ""
                }
            ),
        ),
        ScrubberEvent::IntentApproved { id } => ("✓", format!("approved {id}")),
        ScrubberEvent::IntentRejected { id, dismissed } => (
            "✗",
            format!(
                "rejected {id}{}",
                if *dismissed { " (dismissed)" } else { "" }
            ),
        ),
        ScrubberEvent::IntentReinstated { id } => ("↩", format!("reinstated {id}")),
        ScrubberEvent::PendingRuleCreated { provider, .. } => {
            ("§", format!("rule proposed by {provider}"))
        }
        ScrubberEvent::CoverageAdvanced { .. } => return None,
        ScrubberEvent::PlanCompleted { report } => (
            "✔",
            format!(
                "plan complete: {} subjects, {} ready, {} awaiting approval",
                report.subjects_seen, report.queued_ready, report.queued_awaiting_approval
            ),
        ),
        ScrubberEvent::ExecStarted => ("▶", "executing queue".to_string()),
        ScrubberEvent::IntentStarted { subject, .. } => {
            ("⇉", format!("finding scrobbles for {subject}"))
        }
        ScrubberEvent::EditProgress { subject, step, .. } => {
            let action = edit_step_label(step, Local::now().timestamp());
            ("…", format!("{action}: {subject}"))
        }
        ScrubberEvent::IntentExpanded {
            subject, instances, ..
        } => ("⇉", format!("{subject}: {instances} instance(s)")),
        ScrubberEvent::EditApplied { subject, .. } => ("✎", format!("edited {subject}")),
        ScrubberEvent::EditFailed { subject, error, .. } => {
            ("‼", format!("FAILED {subject}: {error}"))
        }
        ScrubberEvent::IntentCompleted { id, state } => ("✔", format!("intent {id} → {state:?}")),
        ScrubberEvent::ExecutorPaused {
            reason: PauseReason::RateLimited { until_estimate },
        } => (
            "⏸",
            match until_estimate {
                Some(until) => format!("rate limited until {}", fmt_ts(*until)),
                None => "rate limited".to_string(),
            },
        ),
        ScrubberEvent::ExecutorPaused { .. } => ("⏸", "paused".to_string()),
        ScrubberEvent::ExecutorResumed => ("⏵", "resumed".to_string()),
        ScrubberEvent::ExecCompleted { report } => (
            "✔",
            format!(
                "pass ended: {} — {} applied, {} failed",
                ended_label(&report.ended),
                report.instances_applied,
                report.instances_failed
            ),
        ),
        ScrubberEvent::ExecFailed { error, .. } => ("‼", format!("execution stopped: {error}")),
        ScrubberEvent::IdentityKeyProcessed { .. } => return None,
        ScrubberEvent::IdentityRoundCompleted { report } => (
            "♫",
            format!(
                "identity round: {} keys, {} resolved, {} ambiguous, {} not found",
                report.keys_attempted, report.resolved, report.ambiguous, report.not_found
            ),
        ),
        ScrubberEvent::CycleStarted { n } => ("↻", format!("cycle {n}")),
        ScrubberEvent::CycleCompleted { .. } => return None,
        ScrubberEvent::Sleeping { seconds } => ("z", format!("idle for {seconds}s")),
        ScrubberEvent::Stopped { reason } => ("■", format!("stopped: {reason}")),
        ScrubberEvent::Error { error } => ("‼", format!("error: {error}")),
        ScrubberEvent::Sync(sync) => match sync {
            SyncEvent::SyncStarted { mode } => ("☁", format!("sync started ({mode:?})")),
            SyncEvent::ScrobblesDiscovered { new, .. } if *new > 0 => {
                ("☁", format!("discovered {new} new scrobble(s)"))
            }
            SyncEvent::SyncCompleted { stats } => {
                ("☁", format!("sync complete: {} new", stats.scrobbles_new))
            }
            SyncEvent::SyncFailed { error } => ("‼", format!("sync failed: {error}")),
            _ => return None,
        },
    })
}

/// Human label for why an execute pass returned.
pub fn ended_label(ended: &ExecEnded) -> &'static str {
    match ended {
        ExecEnded::Completed => "completed",
        ExecEnded::Busy => {
            "another Scrubber process is editing this library; no edits started here"
        }
        ExecEnded::Deferred => {
            "stopped because Last.fm is still rate limiting; remaining edits are queued"
        }
        ExecEnded::Cancelled => "cancelled",
        ExecEnded::BudgetExhausted => "edit limit reached",
    }
}

pub fn edit_step_label(step: &EditStep, now: i64) -> String {
    match step {
        EditStep::ReadingDetails => "Fetching scrobble details from Last.fm".into(),
        EditStep::Recovering => "Checking a previously interrupted edit".into(),
        EditStep::Sending => "Sending edit to Last.fm".into(),
        EditStep::Pacing { until } => {
            let left = (*until as i64 - now).max(0);
            format!("Waiting {left}s before the next edit")
        }
    }
}

pub fn parse_edit_limit(input: &str) -> Result<Option<u32>, &'static str> {
    if input.trim().is_empty() {
        return Ok(None);
    }
    match input.trim().parse() {
        Ok(value) if value > 0 => Ok(Some(value)),
        _ => Err("Enter a positive number, or leave blank for no limit."),
    }
}

pub fn execution_failure_copy(kind: ExecFailureKind) -> (&'static str, &'static str) {
    match kind {
        ExecFailureKind::Authentication => (
            "Last.fm sign-in required",
            "Last.fm no longer accepts the saved session. Sign in again before retrying.",
        ),
        ExecFailureKind::Network => (
            "Could not reach Last.fm",
            "Check your connection and retry. Last.fm may also be temporarily unavailable.",
        ),
        ExecFailureKind::Other => (
            "The run stopped because of an error",
            "Check the details below before retrying. If the same error returns, keep these details for troubleshooting.",
        ),
    }
}

/// A changed field in a proposed edit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldDiff {
    pub field: &'static str,
    pub from: String,
    pub to: String,
}

/// Extract the changed fields of a proposal for display, falling back to the subject for
/// missing originals.
pub fn edit_diff(subject: &Subject, edit: &ScrobbleEdit) -> Vec<FieldDiff> {
    let mut diffs = Vec::new();

    let from_track = edit
        .track_name_original
        .clone()
        .unwrap_or_else(|| subject.track.clone());
    if let Some(to) = &edit.track_name {
        if *to != from_track {
            diffs.push(FieldDiff {
                field: "track",
                from: from_track,
                to: to.clone(),
            });
        }
    }

    if edit.artist_name != edit.artist_name_original {
        diffs.push(FieldDiff {
            field: "artist",
            from: edit.artist_name_original.clone(),
            to: edit.artist_name.clone(),
        });
    }

    let from_album = edit
        .album_name_original
        .clone()
        .or_else(|| subject.album.clone());
    if let (Some(from), Some(to)) = (&from_album, &edit.album_name) {
        if to != from {
            diffs.push(FieldDiff {
                field: "album",
                from: from.clone(),
                to: to.clone(),
            });
        }
    }

    let from_album_artist = edit
        .album_artist_name_original
        .clone()
        .or_else(|| subject.album_artist.clone());
    if let (Some(from), Some(to)) = (&from_album_artist, &edit.album_artist_name) {
        if to != from {
            diffs.push(FieldDiff {
                field: "album artist",
                from: from.clone(),
                to: to.clone(),
            });
        }
    }

    diffs
}

pub fn fmt_ts(ts: u64) -> String {
    DateTime::from_timestamp(ts as i64, 0)
        .map(|dt| {
            dt.with_timezone(&Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|| ts.to_string())
}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;
