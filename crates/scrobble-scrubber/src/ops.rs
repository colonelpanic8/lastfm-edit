//! Queue operations: the human-facing approval workflow over the durable state.

use crate::error::{Result, ScrubberError};
use crate::queue::{
    IntentState, PendingRuleState, QueueEvent, QueueEventKind, RuleEvent, RuleEventKind,
};
use crate::state::{DismissedEntry, ScrubberState};
use uuid::Uuid;

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Release an `AwaitingApproval` intent for execution.
pub async fn approve_intent(state: &dyn ScrubberState, id: Uuid) -> Result<()> {
    let queue = state.load_queue().await?;
    let intent = queue
        .iter()
        .find(|intent| intent.id == id)
        .ok_or_else(|| ScrubberError::NotFound(format!("intent {id}")))?;
    if intent.state != IntentState::AwaitingApproval {
        return Err(ScrubberError::InvalidState(format!(
            "intent {id} is {:?}, not AwaitingApproval",
            intent.state
        )));
    }
    state
        .append_queue_events(&[QueueEvent {
            id,
            at: now(),
            kind: QueueEventKind::Approved,
        }])
        .await
}

/// Decline an open intent; optionally dismiss its subject from all future suggestions.
pub async fn reject_intent(state: &dyn ScrubberState, id: Uuid, dismiss: bool) -> Result<()> {
    let queue = state.load_queue().await?;
    let intent = queue
        .iter()
        .find(|intent| intent.id == id)
        .ok_or_else(|| ScrubberError::NotFound(format!("intent {id}")))?;
    if !intent.state.is_open() {
        return Err(ScrubberError::InvalidState(format!(
            "intent {id} is {:?}, not open",
            intent.state
        )));
    }
    state
        .append_queue_events(&[QueueEvent {
            id,
            at: now(),
            kind: QueueEventKind::Rejected {
                dismiss_subject: dismiss,
            },
        }])
        .await?;
    if dismiss {
        state
            .append_dismissed(&[DismissedEntry {
                subject: intent.subject.clone(),
                at: now(),
                reason: "rejected-intent".into(),
                active: true,
            }])
            .await?;
    }
    Ok(())
}

/// Un-reject a rejected intent, restoring it to its pre-rejection open state; if the
/// rejection also dismissed the subject, the dismissal is lifted too.
pub async fn reinstate_intent(state: &dyn ScrubberState, id: Uuid) -> Result<()> {
    let queue = state.load_queue().await?;
    let intent = queue
        .iter()
        .find(|intent| intent.id == id)
        .ok_or_else(|| ScrubberError::NotFound(format!("intent {id}")))?;
    let dismissed = match intent.state {
        IntentState::Rejected { dismissed } => dismissed,
        _ => {
            return Err(ScrubberError::InvalidState(format!(
                "intent {id} is {:?}, not Rejected",
                intent.state
            )));
        }
    };
    if dismissed {
        state
            .append_dismissed(&[DismissedEntry {
                subject: intent.subject.clone(),
                at: now(),
                reason: "reinstated-intent".into(),
                active: false,
            }])
            .await?;
    }
    state
        .append_queue_events(&[QueueEvent {
            id,
            at: now(),
            kind: QueueEventKind::Reinstated,
        }])
        .await
}

/// Approve a proposed rule: merge it into the active rule set (which invalidates the
/// rules provider's planning coverage via the rules hash) and record the approval.
pub async fn approve_pending_rule(state: &dyn ScrubberState, id: Uuid) -> Result<()> {
    let pending = state.load_pending_rules().await?;
    let proposal = pending
        .iter()
        .find(|rule| rule.id == id)
        .ok_or_else(|| ScrubberError::NotFound(format!("pending rule {id}")))?;
    if proposal.state != PendingRuleState::Open {
        return Err(ScrubberError::InvalidState(format!(
            "pending rule {id} is {:?}, not Open",
            proposal.state
        )));
    }
    let mut rules = state.load_rules().await?;
    rules.push((*proposal.rule).clone());
    state.save_rules(&rules).await?;
    state
        .append_rule_events(&[RuleEvent {
            id,
            at: now(),
            kind: RuleEventKind::Approved,
        }])
        .await
}

/// Decline a proposed rule.
pub async fn reject_pending_rule(state: &dyn ScrubberState, id: Uuid) -> Result<()> {
    let pending = state.load_pending_rules().await?;
    let proposal = pending
        .iter()
        .find(|rule| rule.id == id)
        .ok_or_else(|| ScrubberError::NotFound(format!("pending rule {id}")))?;
    if proposal.state != PendingRuleState::Open {
        return Err(ScrubberError::InvalidState(format!(
            "pending rule {id} is {:?}, not Open",
            proposal.state
        )));
    }
    state
        .append_rule_events(&[RuleEvent {
            id,
            at: now(),
            kind: RuleEventKind::Rejected,
        }])
        .await
}
