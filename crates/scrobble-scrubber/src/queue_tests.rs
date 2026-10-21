use super::*;
use crate::rewrite::create_no_op_edit;

#[test]
fn late_executor_events_do_not_override_rejection() {
    let id = Uuid::new_v4();
    let queue = fold_queue([
        created(id, 1, false),
        event(
            id,
            2,
            QueueEventKind::Rejected {
                dismiss_subject: false,
            },
        ),
        event(id, 3, QueueEventKind::Completed),
        event(
            id,
            4,
            QueueEventKind::Abandoned {
                reason: "late".into(),
            },
        ),
    ]);
    assert_eq!(queue[0].state, IntentState::Rejected { dismissed: false });
}

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
            rationale: None,
            sources: BTreeMap::new(),
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
