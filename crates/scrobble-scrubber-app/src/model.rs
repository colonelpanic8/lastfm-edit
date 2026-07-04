//! UI-side state types and pure functions (no dioxus imports — unit-testable as-is).

use chrono::{DateTime, Local};
use lastfm_edit::ScrobbleEdit;
use scrobble_scrubber::{ExecReport, PlanReport, ScrubberEvent, Subject};
use scrobble_store::{PauseReason, SyncEvent};
use std::collections::VecDeque;

pub const LOG_CAP: usize = 500;

#[derive(Clone, Debug)]
pub struct LogEntry {
    pub at: DateTime<Local>,
    pub icon: &'static str,
    pub summary: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum ExecStatus {
    #[default]
    Idle,
    Running,
    Paused {
        until: Option<u64>,
    },
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
#[derive(Clone, Debug, Default)]
pub struct UiState {
    pub log: VecDeque<LogEntry>,
    pub exec: ExecStatus,
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
        | ScrubberEvent::IntentReinstated { .. }
        | ScrubberEvent::IntentCompleted { .. }
        | ScrubberEvent::IntentExpanded { .. }
        | ScrubberEvent::EditApplied { .. }
        | ScrubberEvent::EditFailed { .. } => {
            state.queue_epoch += 1;
        }
        ScrubberEvent::ExecStarted => {
            state.exec = ExecStatus::Running;
        }
        ScrubberEvent::ExecCompleted { report } => {
            state.exec = ExecStatus::Idle;
            state.last_exec = Some(report.clone());
            state.queue_epoch += 1;
        }
        ScrubberEvent::ExecutorPaused { reason } => {
            let until = match reason {
                PauseReason::RateLimited { until_estimate } => *until_estimate,
                PauseReason::Backoff { .. } => None,
            };
            state.exec = ExecStatus::Paused { until };
        }
        ScrubberEvent::ExecutorResumed => {
            state.exec = ExecStatus::Running;
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
                "execute complete: {} applied, {} failed",
                report.instances_applied, report.instances_failed
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

    #[test]
    fn reduce_tracks_exec_lifecycle_and_rate_limits() {
        let mut state = UiState::default();
        reduce(&mut state, &ScrubberEvent::ExecStarted);
        assert_eq!(state.exec, ExecStatus::Running);

        reduce(
            &mut state,
            &ScrubberEvent::ExecutorPaused {
                reason: PauseReason::RateLimited {
                    until_estimate: Some(1000),
                },
            },
        );
        assert_eq!(state.exec, ExecStatus::Paused { until: Some(1000) });

        reduce(&mut state, &ScrubberEvent::ExecutorResumed);
        assert_eq!(state.exec, ExecStatus::Running);
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
