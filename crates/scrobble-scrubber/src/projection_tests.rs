use super::*;
use crate::queue::InstanceStatus;
use crate::rewrite::create_no_op_edit;
use crate::subject::Subject;
use scrobble_store::ScrobbleId;
use std::collections::BTreeMap;
use uuid::Uuid;

fn subject() -> Subject {
    Subject {
        artist: "A".into(),
        track: "x".into(),
        album: Some("Album".into()),
        album_artist: None,
    }
}

fn intent(created_at: u64, state: IntentState) -> EditIntent {
    let track = subject().representative_track(1, Some(100));
    EditIntent {
        id: Uuid::new_v4(),
        subject: subject(),
        proposed: Box::new(create_no_op_edit(&track)),
        provider: "rewrite_rules".into(),
        requires_approval: false,
        rationale: None,
        sources: Default::default(),
        state,
        instances: BTreeMap::new(),
        created_at,
        updated_at: created_at,
    }
}

fn with_instances(mut intent: EditIntent, statuses: Vec<InstanceStatus>) -> EditIntent {
    for (n, status) in statuses.into_iter().enumerate() {
        intent
            .instances
            .insert(ScrobbleId::new(n as u64, "A", "x"), status);
    }
    intent
}

fn every_state() -> Vec<IntentState> {
    vec![
        IntentState::AwaitingApproval,
        IntentState::Ready,
        IntentState::InProgress,
        IntentState::Applied,
        IntentState::Rejected { dismissed: true },
        IntentState::Rejected { dismissed: false },
        IntentState::Abandoned {
            reason: "why".into(),
        },
    ]
}

#[test]
fn review_status_covers_every_state() {
    for state in every_state() {
        let expected = match &state {
            IntentState::AwaitingApproval => ReviewStatus::NeedsReview,
            IntentState::Rejected { dismissed } => ReviewStatus::Declined {
                dismissed: *dismissed,
            },
            _ => ReviewStatus::Accepted,
        };
        assert_eq!(
            review_status(&intent(1, state.clone())),
            expected,
            "{state:?}"
        );
    }
}

#[test]
fn work_status_covers_every_state() {
    for state in every_state() {
        let subject = intent(1, state.clone());
        let expected = match &state {
            IntentState::Rejected { .. } => None,
            IntentState::AwaitingApproval | IntentState::Ready => Some(WorkStatus::Queued),
            IntentState::InProgress => Some(WorkStatus::Partial {
                applied: 0,
                failed: 0,
                pending: 0,
            }),
            IntentState::Applied => Some(WorkStatus::Done),
            IntentState::Abandoned { reason } => Some(WorkStatus::GaveUp {
                reason: reason.clone(),
            }),
        };
        assert_eq!(work_status(&subject), expected, "{state:?}");
    }
}

#[test]
fn work_status_partial_reports_per_instance_counts() {
    let subject = with_instances(
        intent(1, IntentState::InProgress),
        vec![
            InstanceStatus::Applied {
                edit_id: "e1".into(),
            },
            InstanceStatus::Failed {
                attempts: 2,
                last_error: "boom".into(),
            },
            InstanceStatus::Pending,
            InstanceStatus::Pending,
        ],
    );
    assert_eq!(
        work_status(&subject),
        Some(WorkStatus::Partial {
            applied: 1,
            failed: 1,
            pending: 2,
        })
    );
}

#[test]
fn execution_order_resumes_in_progress_first_and_is_stable_within_groups() {
    let ready_old = intent(1, IntentState::Ready);
    let in_progress_new = intent(10, IntentState::InProgress);
    let ready_new = intent(20, IntentState::Ready);
    let in_progress_newest = intent(30, IntentState::InProgress);

    let ordered = execution_order(vec![
        ready_old.clone(),
        in_progress_new.clone(),
        ready_new.clone(),
        in_progress_newest.clone(),
    ]);
    let ids: Vec<Uuid> = ordered.iter().map(|i| i.id).collect();
    assert_eq!(
        ids,
        vec![
            in_progress_new.id,
            in_progress_newest.id,
            ready_old.id,
            ready_new.id
        ],
        "InProgress before Ready; input (creation) order preserved within each group"
    );
}

#[test]
fn execution_order_excludes_non_executable_states() {
    let ordered = execution_order(vec![
        intent(1, IntentState::AwaitingApproval),
        intent(2, IntentState::Applied),
        intent(3, IntentState::Rejected { dismissed: false }),
        intent(
            4,
            IntentState::Abandoned {
                reason: "why".into(),
            },
        ),
    ]);
    assert!(ordered.is_empty());
}

#[test]
fn work_queue_view_counts_only_known_pending_and_flags_unexpanded() {
    // An unexpanded Ready intent: workload unknown until the executor expands it.
    let unexpanded = intent(1, IntentState::Ready);
    // A partially-done InProgress intent with known per-instance state.
    let partial = with_instances(
        intent(2, IntentState::InProgress),
        vec![
            InstanceStatus::Applied {
                edit_id: "e1".into(),
            },
            InstanceStatus::Pending,
            InstanceStatus::Pending,
            InstanceStatus::Failed {
                attempts: 1,
                last_error: "boom".into(),
            },
        ],
    );
    // Non-executable intents must not appear at all.
    let done = intent(3, IntentState::Applied);

    let view = work_queue_view(vec![unexpanded.clone(), partial.clone(), done]);
    assert_eq!(view.items.len(), 2);
    // Execution order: the InProgress intent leads.
    assert_eq!(view.items[0].intent.id, partial.id);
    assert_eq!(view.items[0].applied, 1);
    assert_eq!(view.items[0].failed, 1);
    assert_eq!(view.items[0].known_pending, 2);
    assert!(view.items[0].expanded);
    assert_eq!(view.items[1].intent.id, unexpanded.id);
    assert_eq!(view.items[1].known_pending, 0);
    assert!(!view.items[1].expanded);
    // Honesty: the total admits only what is known.
    assert_eq!(view.known_pending_total, 2);
    assert_eq!(view.unexpanded_intents, 1);
}
