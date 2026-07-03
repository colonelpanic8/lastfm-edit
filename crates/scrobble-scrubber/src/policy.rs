//! Policy: how suggestions become queue entries (or don't).

/// How the planner disposes of provider suggestions.
#[derive(Clone, Copy, Debug, Default)]
pub struct Policy {
    /// Report suggestions as events without queueing anything.
    pub dry_run: bool,
    /// Force every edit suggestion through human approval, regardless of what the
    /// suggesting rule/provider says.
    pub require_confirmation_all: bool,
    /// Merge provider-proposed rewrite rules straight into the active set instead of
    /// pending them for approval. Off by default — new rules re-scan history.
    pub auto_approve_rules: bool,
}

/// Disposition of one edit suggestion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditDecision {
    /// Dry run: emit an event, queue nothing.
    Report,
    /// Queue an intent; `requires_approval` decides whether it starts AwaitingApproval
    /// or Ready.
    Queue { requires_approval: bool },
}

impl Policy {
    pub fn decide_edit(&self, suggestion_requires_confirmation: bool) -> EditDecision {
        if self.dry_run {
            EditDecision::Report
        } else {
            EditDecision::Queue {
                requires_approval: suggestion_requires_confirmation
                    || self.require_confirmation_all,
            }
        }
    }
}
