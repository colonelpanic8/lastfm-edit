//! Typed events emitted by the planner and executor, so CLIs/UIs can render progress —
//! including *why* nothing is moving (rate-limit pauses).
//!
//! The taxonomy is kept morally compatible with the original scrobble-scrubber's
//! `ScrubberEventType` to ease migrating its Dioxus app later.

use crate::feed::ScrubFeed;
use crate::queue::IntentState;
use crate::subject::Subject;
use scrobble_store::{PauseReason, ScrobbleId, SyncEvent};
use serde::{Deserialize, Serialize};
use std::ops::Range;
use tokio::sync::broadcast;
use uuid::Uuid;

/// Summary of one planning pass.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanReport {
    pub subjects_seen: u64,
    pub suggestions: u64,
    pub queued_ready: u64,
    pub queued_awaiting_approval: u64,
    pub rules_proposed: u64,
    /// Dry-run: suggestions reported but not enqueued.
    pub reported_only: u64,
}

/// Why an execution pass returned.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecEnded {
    /// The pass drained every executable intent it set out to.
    #[default]
    Completed,
    /// Sustained rate limiting: the pass stopped early, leaving work `InProgress`.
    Deferred,
    /// The cancel handle was flipped mid-pass.
    Cancelled,
    /// The per-pass edit budget ran out with work remaining.
    BudgetExhausted,
}

/// Summary of one execution pass.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecReport {
    pub intents_processed: u64,
    pub intents_completed: u64,
    pub intents_abandoned: u64,
    pub instances_applied: u64,
    pub instances_failed: u64,
    /// Bus-only type, so this is additive: old payloads deserialize as `Completed`.
    #[serde(default)]
    pub ended: ExecEnded,
}

/// Events broadcast by the scrubber's components.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ScrubberEvent {
    // ---- planner ----------------------------------------------------------------
    PlanStarted {
        feed: ScrubFeed,
    },
    SubjectsFound {
        count: usize,
        batch_range: Option<Range<u64>>,
    },
    SubjectAnalyzed {
        subject: Subject,
        suggestions: usize,
    },
    /// Dry-run: what *would* have been queued.
    SuggestionReported {
        subject: Subject,
        provider: String,
        summary: String,
    },
    IntentQueued {
        id: Uuid,
        subject: Subject,
        provider: String,
        state: IntentState,
    },
    /// A human released an intent for execution.
    IntentApproved {
        id: Uuid,
    },
    /// A human declined an intent.
    IntentRejected {
        id: Uuid,
        dismissed: bool,
    },
    /// A human reinstated a previously rejected intent.
    IntentReinstated {
        id: Uuid,
    },
    PendingRuleCreated {
        id: Uuid,
        provider: String,
    },
    CoverageAdvanced {
        provider: String,
        range: Range<u64>,
    },
    PlanCompleted {
        report: PlanReport,
    },

    // ---- executor ---------------------------------------------------------------
    ExecStarted,
    IntentExpanded {
        id: Uuid,
        subject: Subject,
        instances: usize,
    },
    EditApplied {
        intent: Uuid,
        subject: Subject,
        instance: ScrobbleId,
        edit_id: String,
    },
    EditFailed {
        intent: Uuid,
        subject: Subject,
        instance: ScrobbleId,
        error: String,
    },
    IntentCompleted {
        id: Uuid,
        state: IntentState,
    },
    ExecutorPaused {
        reason: PauseReason,
    },
    ExecutorResumed,
    ExecCompleted {
        report: ExecReport,
    },

    // ---- lifecycle / continuous mode ---------------------------------------------
    CycleStarted {
        n: u64,
    },
    CycleCompleted {
        n: u64,
    },
    Sleeping {
        seconds: u64,
    },
    Stopped {
        reason: String,
    },
    Error {
        error: String,
    },
    /// Forwarded store/sync/mirrored-edit events (one ordered stream for consumers).
    Sync(SyncEvent),
}

/// Receiver half for [`ScrubberEvent`] subscriptions.
pub type ScrubberEventReceiver = broadcast::Receiver<ScrubberEvent>;

/// Broadcast sender with subscribe/emit conveniences, shared by planner and executor.
#[derive(Clone, Debug)]
pub struct ScrubberEventBus {
    tx: broadcast::Sender<ScrubberEvent>,
}

impl ScrubberEventBus {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(4096);
        Self { tx }
    }

    pub fn subscribe(&self) -> ScrubberEventReceiver {
        self.tx.subscribe()
    }

    /// Emit an event; delivery is best-effort (no subscribers is not an error).
    pub fn emit(&self, event: ScrubberEvent) {
        let _ = self.tx.send(event);
    }
}

impl Default for ScrubberEventBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_report_without_ended_deserializes_as_completed() {
        // A payload from before `ended` existed must fold to the default.
        let json = r#"{
            "intents_processed": 2,
            "intents_completed": 1,
            "intents_abandoned": 0,
            "instances_applied": 3,
            "instances_failed": 1
        }"#;
        let report: ExecReport = serde_json::from_str(json).unwrap();
        assert_eq!(report.ended, ExecEnded::Completed);
        assert_eq!(report.intents_processed, 2);
    }
}
