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
        other => panic!("expected an in-flight pass, got {other:?}"),
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
fn reduce_bumps_sync_epoch_on_sync_progress() {
    use scrobble_store::{SyncEvent, SyncStats};
    let mut state = UiState::default();
    assert_eq!(state.sync_epoch, 0);
    reduce(
        &mut state,
        &ScrubberEvent::Sync(SyncEvent::ScrobblesDiscovered {
            new: 3,
            updated: 0,
            oldest: Some(1),
            newest: Some(2),
        }),
    );
    assert_eq!(state.sync_epoch, 1);
    reduce(
        &mut state,
        &ScrubberEvent::Sync(SyncEvent::SyncCompleted {
            stats: SyncStats::default(),
        }),
    );
    assert_eq!(state.sync_epoch, 2);
    // A failure updates status but must not bump the feed epoch.
    reduce(
        &mut state,
        &ScrubberEvent::Sync(SyncEvent::SyncFailed {
            error: "boom".into(),
        }),
    );
    assert_eq!(state.sync_epoch, 2);
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

#[test]
fn execution_failure_stops_controls_and_keeps_progress() {
    let mut state = UiState::default();
    let id = Uuid::new_v4();
    reduce(&mut state, &ScrubberEvent::ExecStarted);
    reduce(&mut state, &applied(id));
    reduce(
        &mut state,
        &ScrubberEvent::ExecFailed {
            error: "Last.fm session expired".into(),
        },
    );
    assert!(!state.pass.is_active());
    match &state.pass {
        PassState::Failed { progress, error } => {
            assert_eq!(progress.applied, 1);
            assert_eq!(error, "Last.fm session expired");
        }
        other => panic!("expected a visible execution failure, got {other:?}"),
    }
    reduce(&mut state, &ScrubberEvent::ExecStarted);
    assert!(state.pass.is_active());
    assert_eq!(progress(&state).applied, 0);
}

#[test]
fn execution_shows_track_before_expansion_and_each_network_step() {
    let mut state = UiState::default();
    let id = Uuid::new_v4();
    reduce(&mut state, &ScrubberEvent::ExecStarted);
    reduce(
        &mut state,
        &ScrubberEvent::IntentStarted {
            id,
            subject: subject(),
        },
    );
    assert_eq!(
        progress(&state).current.as_ref().unwrap().subject,
        subject().to_string()
    );
    for step in [
        EditStep::ReadingDetails,
        EditStep::Recovering,
        EditStep::Sending,
    ] {
        reduce(
            &mut state,
            &ScrubberEvent::EditProgress {
                intent: id,
                subject: subject(),
                instance: instance(),
                step: step.clone(),
            },
        );
        let current = progress(&state).current_edit.as_ref().unwrap();
        assert_eq!(current.instance, instance());
        assert_eq!(current.step, step);
        assert!(state.execution_updated_at.is_some());
    }
}

#[test]
fn edit_limit_is_optional_and_invalid_values_never_mean_unlimited() {
    assert_eq!(parse_edit_limit(""), Ok(None));
    assert_eq!(parse_edit_limit("  "), Ok(None));
    assert_eq!(parse_edit_limit(" 20 "), Ok(Some(20)));
    for invalid in ["0", "-1", "1.5", "4294967296", "twenty"] {
        assert!(parse_edit_limit(invalid).is_err(), "accepted {invalid}");
    }
    assert_eq!(
        ended_label(&ExecEnded::BudgetExhausted),
        "edit limit reached"
    );
}

#[test]
fn pacing_countdown_explains_the_wait() {
    let step = EditStep::Pacing { until: 102 };
    assert_eq!(
        edit_step_label(&step, 100),
        "Waiting 2s before the next edit"
    );
    assert_eq!(
        edit_step_label(&step, 103),
        "Waiting 0s before the next edit"
    );
}
