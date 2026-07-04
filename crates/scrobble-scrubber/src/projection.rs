//! Derived, in-memory projections over the durable queue.
//!
//! The persisted [`IntentState`] flattens two orthogonal questions — "has a human
//! decided?" and "how far has the executor gotten?" — into one lifecycle enum, because
//! that is what folds cleanly from the event log. UIs want the questions separated.
//! Everything here is a pure function of already-folded [`EditIntent`]s: no I/O, no new
//! persisted formats, and nothing that can drift from `fold_queue`'s semantics.

use crate::queue::{EditIntent, IntentState};

/// Has a human decided about this intent?
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReviewStatus {
    /// Awaiting a human verdict.
    NeedsReview,
    /// Released for execution (explicitly, by auto-policy, or moot because the executor
    /// already ran it to a terminal state).
    Accepted,
    /// Human declined; `dismissed` = the subject is also suppressed from future
    /// suggestions.
    Declined { dismissed: bool },
}

/// How far has the executor gotten with an intent?
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkStatus {
    /// Not started (whether or not it is even approved yet).
    Queued,
    /// Expansion happened; counts reflect per-instance progress so far.
    Partial {
        applied: usize,
        failed: usize,
        pending: usize,
    },
    /// Everything applied.
    Done,
    /// The executor stopped trying.
    GaveUp { reason: String },
}

/// Which side of the human decision an intent sits on.
pub fn review_status(intent: &EditIntent) -> ReviewStatus {
    match &intent.state {
        IntentState::AwaitingApproval => ReviewStatus::NeedsReview,
        IntentState::Rejected { dismissed } => ReviewStatus::Declined {
            dismissed: *dismissed,
        },
        // Ready/InProgress/Applied/Abandoned all imply the intent was released to the
        // executor — a decision (human or auto-policy) already happened.
        IntentState::Ready
        | IntentState::InProgress
        | IntentState::Applied
        | IntentState::Abandoned { .. } => ReviewStatus::Accepted,
    }
}

/// Execution progress, or `None` for declined intents (progress is moot — the executor
/// will never touch them).
pub fn work_status(intent: &EditIntent) -> Option<WorkStatus> {
    match &intent.state {
        IntentState::Rejected { .. } => None,
        // An unapproved intent still has a well-defined place in the eventual workload.
        IntentState::AwaitingApproval | IntentState::Ready => Some(WorkStatus::Queued),
        IntentState::InProgress => Some(WorkStatus::Partial {
            applied: intent.done_count(),
            failed: intent.failed_count(),
            pending: intent.pending_count(),
        }),
        IntentState::Applied => Some(WorkStatus::Done),
        IntentState::Abandoned { reason } => Some(WorkStatus::GaveUp {
            reason: reason.clone(),
        }),
    }
}

/// The exact order the executor will process intents: executable only, `InProgress`
/// (resume) before `Ready` (start), oldest-first within each group.
///
/// This is the single source of ordering truth — the executor's `run_once` drains
/// intents in precisely this order, so any UI rendering "what happens next" from this
/// function cannot drift from reality.
pub fn execution_order(intents: Vec<EditIntent>) -> Vec<EditIntent> {
    let mut executable: Vec<_> = intents
        .into_iter()
        .filter(|i| i.state.is_executable())
        .collect();
    // Stable sort: `fold_queue` yields first-created order, which this preserves within
    // each group.
    executable.sort_by_key(|i| match i.state {
        IntentState::InProgress => 0u8,
        _ => 1,
    });
    executable
}

/// One executable intent as a work-queue row.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkItem {
    pub intent: EditIntent,
    pub applied: usize,
    pub failed: usize,
    /// Instances known to be `Pending` — see the honesty caveat on [`WorkQueueView`].
    pub known_pending: usize,
    /// Whether the executor has expanded this intent against the store yet.
    pub expanded: bool,
}

/// The executor's upcoming workload, in [`execution_order`].
///
/// Honesty caveat: `Ready` intents have empty `instances` until the executor expands
/// them against the *live* store, so their real instance counts are unknowable here.
/// `known_pending_total` therefore counts only instances already recorded as `Pending`;
/// `unexpanded_intents` counts executable intents whose workload is still a mystery.
/// Render both rather than pretending the total is exact.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkQueueView {
    pub items: Vec<WorkItem>,
    pub known_pending_total: usize,
    pub unexpanded_intents: usize,
}

/// Project the folded queue into the executor's upcoming workload.
pub fn work_queue_view(intents: Vec<EditIntent>) -> WorkQueueView {
    let items: Vec<WorkItem> = execution_order(intents)
        .into_iter()
        .map(|intent| WorkItem {
            applied: intent.done_count(),
            failed: intent.failed_count(),
            known_pending: intent.pending_count(),
            expanded: !intent.instances.is_empty(),
            intent,
        })
        .collect();
    let known_pending_total = items.iter().map(|item| item.known_pending).sum();
    let unexpanded_intents = items.iter().filter(|item| !item.expanded).count();
    WorkQueueView {
        items,
        known_pending_total,
        unexpanded_intents,
    }
}

#[cfg(test)]
mod tests {
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
}
