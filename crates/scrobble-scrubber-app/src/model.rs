//! UI-side state types and pure functions (no dioxus imports — unit-testable as-is).

use chrono::{DateTime, Local};
use lastfm_edit::ScrobbleEdit;
use scrobble_scrubber::{ExecEnded, ExecReport, PlanReport, ScrubberEvent, Subject};
use scrobble_store::{PauseReason, SyncEvent};
use std::collections::VecDeque;
use uuid::Uuid;

pub const LOG_CAP: usize = 500;

#[derive(Clone, Debug)]
pub struct LogEntry {
    pub at: DateTime<Local>,
    pub icon: &'static str,
    pub summary: String,
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
}

impl PassState {
    /// The live progress, whether running or paused.
    fn progress_mut(&mut self) -> Option<&mut PassProgress> {
        match self {
            PassState::Idle => None,
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
    pub sync: SyncStatus,
    /// Bumped whenever queue contents may have changed; views reload on change.
    pub queue_epoch: u64,
    pub error: Option<String>,
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
            if let Some(progress) = state.pass.progress_mut() {
                progress.applied += 1;
            }
            state.queue_epoch += 1;
        }
        ScrubberEvent::EditFailed { .. } => {
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
            }
            state.queue_epoch += 1;
        }
        ScrubberEvent::ExecCompleted { report } => {
            state.pass = PassState::Idle;
            state.last_exec = Some(report.clone());
            state.queue_epoch += 1;
        }
        ScrubberEvent::ExecutorPaused { reason } => {
            let until = match reason {
                PauseReason::RateLimited { until_estimate } => *until_estimate,
                PauseReason::Backoff { .. } => None,
            };
            // Carry the pass's progress into Paused instead of dropping it.
            let progress = match std::mem::take(&mut state.pass) {
                PassState::Running(progress) | PassState::Paused { progress, .. } => progress,
                PassState::Idle => PassProgress::default(),
            };
            state.pass = PassState::Paused { progress, until };
        }
        ScrubberEvent::ExecutorResumed => {
            let progress = match std::mem::take(&mut state.pass) {
                PassState::Running(progress) | PassState::Paused { progress, .. } => progress,
                PassState::Idle => PassProgress::default(),
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
            state.error = Some(error.clone());
        }
        ScrubberEvent::Sync(sync) => match sync {
            SyncEvent::SyncStarted { .. } => state.sync = SyncStatus::Syncing,
            SyncEvent::SyncCompleted { .. } | SyncEvent::SyncFailed { .. } => {
                state.sync = SyncStatus::Idle;
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
        ExecEnded::Deferred => "deferred (rate limited)",
        ExecEnded::Cancelled => "cancelled",
        ExecEnded::BudgetExhausted => "budget exhausted",
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
mod tests {
    use super::*;
    use scrobble_scrubber::ScrubFeed;

    fn subject() -> Subject {
        Subject {
            artist: "Queen".into(),
            track: "You And I - Remastered 2011".into(),
            album: Some("A Day at the Races".into()),
            album_artist: None,
        }
    }

    fn proposal() -> ScrobbleEdit {
        ScrobbleEdit {
            track_name_original: Some("You And I - Remastered 2011".into()),
            album_name_original: Some("A Day at the Races".into()),
            artist_name_original: "Queen".into(),
            album_artist_name_original: None,
            track_name: Some("You And I".into()),
            album_name: Some("A Day at the Races".into()),
            artist_name: "Queen".into(),
            album_artist_name: None,
            timestamp: None,
            edit_all: true,
        }
    }

    #[test]
    fn diff_shows_only_changed_fields() {
        let diffs = edit_diff(&subject(), &proposal());
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].field, "track");
        assert_eq!(diffs[0].from, "You And I - Remastered 2011");
        assert_eq!(diffs[0].to, "You And I");
    }

    #[test]
    fn diff_handles_album_and_artist_changes() {
        let mut edit = proposal();
        edit.album_name = Some("A Day at the Races (Clean)".into());
        edit.artist_name = "Queen!".into();
        let diffs = edit_diff(&subject(), &edit);
        let fields: Vec<_> = diffs.iter().map(|d| d.field).collect();
        assert_eq!(fields, vec!["track", "artist", "album"]);
    }

    fn instance() -> scrobble_store::ScrobbleId {
        scrobble_store::ScrobbleId::new(1000, "Queen", "You And I - Remastered 2011")
    }

    fn applied(intent: uuid::Uuid) -> ScrubberEvent {
        ScrubberEvent::EditApplied {
            intent,
            subject: subject(),
            instance: instance(),
            edit_id: "e1".into(),
        }
    }

    fn progress(state: &UiState) -> &PassProgress {
        match &state.pass {
            PassState::Running(progress) | PassState::Paused { progress, .. } => progress,
            PassState::Idle => panic!("expected an in-flight pass, got Idle"),
        }
    }

    #[test]
    fn reduce_tracks_full_pass_lifecycle() {
        let mut state = UiState::default();
        let id = uuid::Uuid::new_v4();

        reduce(&mut state, &ScrubberEvent::ExecStarted);
        assert_eq!(state.pass, PassState::Running(PassProgress::default()));

        reduce(
            &mut state,
            &ScrubberEvent::IntentExpanded {
                id,
                subject: subject(),
                instances: 2,
            },
        );
        let current = progress(&state).current.clone().expect("current intent");
        assert_eq!(current.id, id);
        assert_eq!(current.instances, 2);
        assert_eq!(current.subject, subject().to_string());

        reduce(&mut state, &applied(id));
        reduce(&mut state, &applied(id));
        assert_eq!(progress(&state).applied, 2);
        assert_eq!(progress(&state).failed, 0);
        assert_eq!(progress(&state).intents_done, 0);

        reduce(
            &mut state,
            &ScrubberEvent::IntentCompleted {
                id,
                state: scrobble_scrubber::IntentState::Applied,
            },
        );
        assert_eq!(progress(&state).intents_done, 1);
        assert_eq!(progress(&state).current, None);

        let report = ExecReport {
            instances_applied: 2,
            ended: ExecEnded::Completed,
            ..ExecReport::default()
        };
        reduce(
            &mut state,
            &ScrubberEvent::ExecCompleted {
                report: report.clone(),
            },
        );
        assert_eq!(state.pass, PassState::Idle);
        assert_eq!(state.last_exec, Some(report));
    }

    #[test]
    fn reduce_pause_preserves_progress_and_resume_restores_it() {
        let mut state = UiState::default();
        let id = uuid::Uuid::new_v4();
        reduce(&mut state, &ScrubberEvent::ExecStarted);
        reduce(&mut state, &applied(id));
        reduce(
            &mut state,
            &ScrubberEvent::EditFailed {
                intent: id,
                subject: subject(),
                instance: instance(),
                error: "boom".into(),
            },
        );

        reduce(
            &mut state,
            &ScrubberEvent::ExecutorPaused {
                reason: PauseReason::RateLimited {
                    until_estimate: Some(1000),
                },
            },
        );
        match &state.pass {
            PassState::Paused { progress, until } => {
                assert_eq!(*until, Some(1000));
                assert_eq!(progress.applied, 1);
                assert_eq!(progress.failed, 1);
            }
            other => panic!("expected Paused, got {other:?}"),
        }

        // Counters keep advancing while paused (in-flight edits can still land).
        reduce(&mut state, &applied(id));
        assert_eq!(progress(&state).applied, 2);

        reduce(&mut state, &ScrubberEvent::ExecutorResumed);
        match &state.pass {
            PassState::Running(p) => {
                assert_eq!(p.applied, 2);
                assert_eq!(p.failed, 1);
            }
            other => panic!("expected Running, got {other:?}"),
        }
    }

    #[test]
    fn reduce_tracks_continuous_cycles() {
        let mut state = UiState::default();
        assert_eq!(state.continuous, None);

        reduce(&mut state, &ScrubberEvent::CycleStarted { n: 3 });
        assert_eq!(
            state.continuous,
            Some(CycleInfo {
                n: 3,
                phase: CyclePhase::Running,
            })
        );

        reduce(&mut state, &ScrubberEvent::CycleCompleted { n: 3 });
        reduce(&mut state, &ScrubberEvent::Sleeping { seconds: 300 });
        assert_eq!(
            state.continuous,
            Some(CycleInfo {
                n: 3,
                phase: CyclePhase::Sleeping { seconds: 300 },
            })
        );
    }

    #[test]
    fn exec_completed_exposes_ended_reason() {
        let mut state = UiState::default();
        reduce(&mut state, &ScrubberEvent::ExecStarted);
        reduce(
            &mut state,
            &ScrubberEvent::ExecCompleted {
                report: ExecReport {
                    ended: ExecEnded::Deferred,
                    ..ExecReport::default()
                },
            },
        );
        assert_eq!(
            state.last_exec.as_ref().map(|r| r.ended.clone()),
            Some(ExecEnded::Deferred)
        );
    }

    #[test]
    fn reduce_bumps_queue_epoch_on_queue_changes() {
        let mut state = UiState::default();
        let before = state.queue_epoch;
        reduce(
            &mut state,
            &ScrubberEvent::IntentApproved {
                id: uuid::Uuid::new_v4(),
            },
        );
        assert_eq!(state.queue_epoch, before + 1);
    }

    #[test]
    fn log_is_bounded() {
        let mut state = UiState::default();
        for _ in 0..(LOG_CAP + 50) {
            reduce(
                &mut state,
                &ScrubberEvent::PlanStarted {
                    feed: ScrubFeed::Incremental { window: None },
                },
            );
        }
        assert_eq!(state.log.len(), LOG_CAP);
    }
}
