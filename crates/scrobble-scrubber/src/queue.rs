//! The durable edit-intent queue: the seam between the planner (which conceives edits)
//! and the executor (which applies them).
//!
//! One event-sourced log (`queue.jsonl`) unifies what the old scrubber kept as two
//! things — "pending edits awaiting approval" and the execution backlog. Approval is
//! just the `AwaitingApproval → Ready` transition. Intents are *subject-level*; the
//! executor expands them to concrete scrobble instances at execution time and records
//! per-instance progress here, so a crash resumes exactly where it stopped.
//!
//! A parallel, smaller log (`pending_rules.jsonl`) holds provider-proposed rewrite rules
//! awaiting human approval.

use crate::subject::Subject;
use lastfm_edit::ScrobbleEdit;
use scrobble_store::ScrobbleId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

// =====================================================================================
// Edit intents
// =====================================================================================

/// One line in `queue.jsonl`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QueueEvent {
    pub id: Uuid,
    /// When the event was recorded (Unix seconds).
    pub at: u64,
    #[serde(flatten)]
    pub kind: QueueEventKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum QueueEventKind {
    /// A planner recorded the intent to edit every instance of `subject`.
    Created {
        subject: Subject,
        /// Metadata-level changes (`edit_all` is notional here — the executor expands
        /// to exact per-instance edits; nothing with `edit_all` reaches last.fm).
        proposed: Box<ScrobbleEdit>,
        provider: String,
        requires_approval: bool,
    },
    /// Human (or auto-policy at creation) released the intent for execution.
    Approved,
    /// Human declined; optionally also dismisses the subject from future suggestions.
    Rejected { dismiss_subject: bool },
    /// Un-reject a previously rejected intent, restoring it to its pre-rejection open
    /// state.
    Reinstated,
    /// Executor snapshot of the instances it will work through (may occur again on a
    /// later run if re-expansion finds new instances; ids accumulate).
    Expanded { instance_ids: Vec<ScrobbleId> },
    /// One instance was edited upstream and mirrored; `edit_id` cross-references the
    /// store's edit log.
    InstanceApplied {
        instance: ScrobbleId,
        edit_id: String,
    },
    /// One instance failed (retriable until attempts exhaust).
    InstanceFailed { instance: ScrobbleId, error: String },
    /// All instances applied (or none were live).
    Completed,
    /// Gave up (e.g. the store changed under the proposal).
    Abandoned { reason: String },
}

/// Execution status of one expanded instance.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum InstanceStatus {
    Pending,
    Applied { edit_id: String },
    Failed { attempts: u32, last_error: String },
}

/// Folded lifecycle state of an intent.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum IntentState {
    /// Needs human approval before the executor may touch it.
    AwaitingApproval,
    /// Released for execution; not yet started.
    Ready,
    /// Expansion happened; some instances may be done.
    InProgress,
    /// Terminal: everything applied.
    Applied,
    /// Terminal: human declined.
    Rejected { dismissed: bool },
    /// Terminal: executor gave up.
    Abandoned { reason: String },
}

impl IntentState {
    /// Open = still occupying its subject (planner must not re-suggest it).
    pub fn is_open(&self) -> bool {
        matches!(
            self,
            IntentState::AwaitingApproval | IntentState::Ready | IntentState::InProgress
        )
    }

    /// Whether the executor may work on it.
    pub fn is_executable(&self) -> bool {
        matches!(self, IntentState::Ready | IntentState::InProgress)
    }
}

/// Folded view of one intent's event history.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EditIntent {
    pub id: Uuid,
    pub subject: Subject,
    pub proposed: Box<ScrobbleEdit>,
    pub provider: String,
    pub requires_approval: bool,
    pub state: IntentState,
    /// Per-instance progress, populated once expanded.
    pub instances: BTreeMap<ScrobbleId, InstanceStatus>,
    pub created_at: u64,
    pub updated_at: u64,
}

impl EditIntent {
    pub fn done_count(&self) -> usize {
        self.instances
            .values()
            .filter(|s| matches!(s, InstanceStatus::Applied { .. }))
            .count()
    }

    pub fn failed_count(&self) -> usize {
        self.instances
            .values()
            .filter(|s| matches!(s, InstanceStatus::Failed { .. }))
            .count()
    }

    /// Known-`Pending` instances. Zero until the executor expands the intent, so this
    /// undercounts a `Ready` intent's real workload (see [`crate::projection`]).
    pub fn pending_count(&self) -> usize {
        self.instances
            .values()
            .filter(|s| matches!(s, InstanceStatus::Pending))
            .count()
    }
}

/// Fold an ordered event stream into intents, in first-created order.
///
/// Terminal states are last-write-wins by event order; events for unknown ids are
/// ignored with a warning (torn logs, partial merges).
pub fn fold_queue(events: impl IntoIterator<Item = QueueEvent>) -> Vec<EditIntent> {
    let mut order: Vec<Uuid> = Vec::new();
    let mut intents: BTreeMap<Uuid, EditIntent> = BTreeMap::new();

    for event in events {
        match event.kind {
            QueueEventKind::Created {
                subject,
                proposed,
                provider,
                requires_approval,
            } => {
                if intents.contains_key(&event.id) {
                    log::warn!("queue: duplicate Created for {}; ignoring", event.id);
                    continue;
                }
                order.push(event.id);
                intents.insert(
                    event.id,
                    EditIntent {
                        id: event.id,
                        subject,
                        proposed,
                        provider,
                        requires_approval,
                        state: if requires_approval {
                            IntentState::AwaitingApproval
                        } else {
                            IntentState::Ready
                        },
                        instances: BTreeMap::new(),
                        created_at: event.at,
                        updated_at: event.at,
                    },
                );
            }
            kind => {
                let Some(intent) = intents.get_mut(&event.id) else {
                    log::warn!("queue: event for unknown intent {}; ignoring", event.id);
                    continue;
                };
                intent.updated_at = event.at;
                match kind {
                    QueueEventKind::Approved => {
                        if intent.state == IntentState::AwaitingApproval {
                            intent.state = IntentState::Ready;
                        }
                    }
                    QueueEventKind::Rejected { dismiss_subject } => {
                        intent.state = IntentState::Rejected {
                            dismissed: dismiss_subject,
                        };
                    }
                    QueueEventKind::Reinstated => {
                        if matches!(intent.state, IntentState::Rejected { .. }) {
                            intent.state = if intent.requires_approval {
                                IntentState::AwaitingApproval
                            } else {
                                IntentState::Ready
                            };
                        } else {
                            log::warn!(
                                "queue: Reinstated for non-rejected intent {}; ignoring",
                                event.id
                            );
                        }
                    }
                    QueueEventKind::Expanded { instance_ids } => {
                        for id in instance_ids {
                            intent
                                .instances
                                .entry(id)
                                .or_insert(InstanceStatus::Pending);
                        }
                        if intent.state.is_executable() {
                            intent.state = IntentState::InProgress;
                        }
                    }
                    QueueEventKind::InstanceApplied { instance, edit_id } => {
                        intent
                            .instances
                            .insert(instance, InstanceStatus::Applied { edit_id });
                    }
                    QueueEventKind::InstanceFailed { instance, error } => {
                        let attempts = match intent.instances.get(&instance) {
                            Some(InstanceStatus::Failed { attempts, .. }) => attempts + 1,
                            _ => 1,
                        };
                        intent.instances.insert(
                            instance,
                            InstanceStatus::Failed {
                                attempts,
                                last_error: error,
                            },
                        );
                    }
                    QueueEventKind::Completed => {
                        intent.state = IntentState::Applied;
                    }
                    QueueEventKind::Abandoned { reason } => {
                        intent.state = IntentState::Abandoned { reason };
                    }
                    QueueEventKind::Created { .. } => unreachable!("handled above"),
                }
            }
        }
    }

    order
        .into_iter()
        .filter_map(|id| intents.remove(&id))
        .collect()
}

// =====================================================================================
// Pending rule proposals
// =====================================================================================

/// One line in `pending_rules.jsonl`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RuleEvent {
    pub id: Uuid,
    pub at: u64,
    #[serde(flatten)]
    pub kind: RuleEventKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum RuleEventKind {
    Created {
        rule: Box<crate::rewrite::RewriteRule>,
        motivation: String,
        provider: String,
        /// The subject that prompted the proposal, when known.
        example: Option<Subject>,
    },
    /// Approval merges the rule into the active set (done by the caller; this event is
    /// the durable record).
    Approved,
    Rejected,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PendingRuleState {
    Open,
    Approved,
    Rejected,
}

/// Folded view of one proposed rule.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PendingRule {
    pub id: Uuid,
    pub rule: Box<crate::rewrite::RewriteRule>,
    pub motivation: String,
    pub provider: String,
    pub example: Option<Subject>,
    pub state: PendingRuleState,
    pub created_at: u64,
    pub updated_at: u64,
}

/// Fold the pending-rules event stream, in first-created order.
pub fn fold_pending_rules(events: impl IntoIterator<Item = RuleEvent>) -> Vec<PendingRule> {
    let mut order: Vec<Uuid> = Vec::new();
    let mut rules: BTreeMap<Uuid, PendingRule> = BTreeMap::new();
    for event in events {
        match event.kind {
            RuleEventKind::Created {
                rule,
                motivation,
                provider,
                example,
            } => {
                if rules.contains_key(&event.id) {
                    log::warn!(
                        "pending rules: duplicate Created for {}; ignoring",
                        event.id
                    );
                    continue;
                }
                order.push(event.id);
                rules.insert(
                    event.id,
                    PendingRule {
                        id: event.id,
                        rule,
                        motivation,
                        provider,
                        example,
                        state: PendingRuleState::Open,
                        created_at: event.at,
                        updated_at: event.at,
                    },
                );
            }
            kind => {
                let Some(pending) = rules.get_mut(&event.id) else {
                    log::warn!(
                        "pending rules: event for unknown rule {}; ignoring",
                        event.id
                    );
                    continue;
                };
                pending.updated_at = event.at;
                pending.state = match kind {
                    RuleEventKind::Approved => PendingRuleState::Approved,
                    RuleEventKind::Rejected => PendingRuleState::Rejected,
                    RuleEventKind::Created { .. } => unreachable!("handled above"),
                };
            }
        }
    }
    order
        .into_iter()
        .filter_map(|id| rules.remove(&id))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rewrite::create_no_op_edit;

    fn subject() -> Subject {
        Subject {
            artist: "A".into(),
            track: "x".into(),
            album: Some("Album".into()),
            album_artist: None,
        }
    }

    fn created(id: Uuid, at: u64, requires_approval: bool) -> QueueEvent {
        let track = subject().representative_track(1, Some(100));
        QueueEvent {
            id,
            at,
            kind: QueueEventKind::Created {
                subject: subject(),
                proposed: Box::new(create_no_op_edit(&track)),
                provider: "rewrite_rules".into(),
                requires_approval,
            },
        }
    }

    fn event(id: Uuid, at: u64, kind: QueueEventKind) -> QueueEvent {
        QueueEvent { id, at, kind }
    }

    #[test]
    fn lifecycle_awaiting_to_applied() {
        let id = Uuid::new_v4();
        let sid = |n: u64| ScrobbleId::new(n, "A", "x");
        let intents = fold_queue(vec![
            created(id, 1, true),
            event(id, 2, QueueEventKind::Approved),
            event(
                id,
                3,
                QueueEventKind::Expanded {
                    instance_ids: vec![sid(10), sid(20)],
                },
            ),
            event(
                id,
                4,
                QueueEventKind::InstanceApplied {
                    instance: sid(10),
                    edit_id: "e1".into(),
                },
            ),
            event(
                id,
                5,
                QueueEventKind::InstanceFailed {
                    instance: sid(20),
                    error: "boom".into(),
                },
            ),
            event(
                id,
                6,
                QueueEventKind::InstanceApplied {
                    instance: sid(20),
                    edit_id: "e2".into(),
                },
            ),
            event(id, 7, QueueEventKind::Completed),
        ]);
        assert_eq!(intents.len(), 1);
        let intent = &intents[0];
        assert_eq!(intent.state, IntentState::Applied);
        assert_eq!(intent.done_count(), 2);
        assert_eq!(intent.failed_count(), 0);
        assert_eq!(intent.created_at, 1);
        assert_eq!(intent.updated_at, 7);
    }

    #[test]
    fn auto_approved_starts_ready_and_failures_accumulate_attempts() {
        let id = Uuid::new_v4();
        let sid = ScrobbleId::new(10, "A", "x");
        let intents = fold_queue(vec![
            created(id, 1, false),
            event(
                id,
                2,
                QueueEventKind::Expanded {
                    instance_ids: vec![sid.clone()],
                },
            ),
            event(
                id,
                3,
                QueueEventKind::InstanceFailed {
                    instance: sid.clone(),
                    error: "a".into(),
                },
            ),
            event(
                id,
                4,
                QueueEventKind::InstanceFailed {
                    instance: sid.clone(),
                    error: "b".into(),
                },
            ),
        ]);
        let intent = &intents[0];
        assert_eq!(intent.state, IntentState::InProgress);
        assert!(intent.state.is_executable());
        assert_eq!(
            intent.instances[&sid],
            InstanceStatus::Failed {
                attempts: 2,
                last_error: "b".into()
            }
        );
    }

    #[test]
    fn rejected_is_terminal_and_orphans_ignored() {
        let id = Uuid::new_v4();
        let intents = fold_queue(vec![
            event(
                Uuid::new_v4(),
                0,
                QueueEventKind::Approved, // orphan: no Created
            ),
            created(id, 1, true),
            event(
                id,
                2,
                QueueEventKind::Rejected {
                    dismiss_subject: true,
                },
            ),
        ]);
        assert_eq!(intents.len(), 1);
        assert_eq!(intents[0].state, IntentState::Rejected { dismissed: true });
        assert!(!intents[0].state.is_open());
    }

    #[test]
    fn reinstate_restores_rejected_intent() {
        let needs_approval = Uuid::new_v4();
        let auto = Uuid::new_v4();
        let intents = fold_queue(vec![
            created(needs_approval, 1, true),
            event(
                needs_approval,
                2,
                QueueEventKind::Rejected {
                    dismiss_subject: true,
                },
            ),
            event(needs_approval, 3, QueueEventKind::Reinstated),
            created(auto, 4, false),
            event(
                auto,
                5,
                QueueEventKind::Rejected {
                    dismiss_subject: false,
                },
            ),
            event(auto, 6, QueueEventKind::Reinstated),
        ]);
        assert_eq!(intents.len(), 2);
        assert_eq!(intents[0].state, IntentState::AwaitingApproval);
        assert_eq!(intents[1].state, IntentState::Ready);
    }

    #[test]
    fn re_expansion_accumulates_instances_without_resetting_progress() {
        let id = Uuid::new_v4();
        let sid = |n: u64| ScrobbleId::new(n, "A", "x");
        let intents = fold_queue(vec![
            created(id, 1, false),
            event(
                id,
                2,
                QueueEventKind::Expanded {
                    instance_ids: vec![sid(10)],
                },
            ),
            event(
                id,
                3,
                QueueEventKind::InstanceApplied {
                    instance: sid(10),
                    edit_id: "e1".into(),
                },
            ),
            // A later run discovers a new instance.
            event(
                id,
                4,
                QueueEventKind::Expanded {
                    instance_ids: vec![sid(10), sid(30)],
                },
            ),
        ]);
        let intent = &intents[0];
        assert_eq!(intent.instances.len(), 2);
        assert!(matches!(
            intent.instances[&sid(10)],
            InstanceStatus::Applied { .. }
        ));
        assert_eq!(intent.instances[&sid(30)], InstanceStatus::Pending);
    }

    #[test]
    fn events_serialize_as_flat_jsonl_lines() {
        let id = Uuid::new_v4();
        let line = serde_json::to_string(&created(id, 1, true)).unwrap();
        assert!(line.contains("\"event\":\"created\""));
        let back: QueueEvent = serde_json::from_str(&line).unwrap();
        assert!(matches!(back.kind, QueueEventKind::Created { .. }));
    }

    #[test]
    fn pending_rules_fold() {
        let id = Uuid::new_v4();
        let rule_event = |at: u64, kind: RuleEventKind| RuleEvent { id, at, kind };
        let rules = fold_pending_rules(vec![
            rule_event(
                1,
                RuleEventKind::Created {
                    rule: Box::new(crate::rewrite::RewriteRule::new().with_name("r")),
                    motivation: "why".into(),
                    provider: "openai".into(),
                    example: Some(subject()),
                },
            ),
            rule_event(2, RuleEventKind::Approved),
        ]);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].state, PendingRuleState::Approved);
    }
}
