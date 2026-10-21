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
#[path = "projection_tests.rs"]
mod tests;
