use super::*;

fn queued(edit_id: &str, at: u64) -> EditLogEvent {
    EditLogEvent {
        edit_id: edit_id.to_string(),
        at,
        kind: EditEventKind::Queued {
            op: EditOp::Delete {
                artist: "A".into(),
                track: "x".into(),
                uts: 100,
            },
            target_ids: vec![ScrobbleId::new(100, "A", "x")],
        },
    }
}

#[test]
fn folding_replays_lifecycle() {
    let events = vec![
        queued("e1", 10),
        EditLogEvent {
            edit_id: "e1".into(),
            at: 11,
            kind: EditEventKind::AttemptFailed {
                error: "boom".into(),
            },
        },
        queued("e2", 12),
        EditLogEvent {
            edit_id: "e1".into(),
            at: 13,
            kind: EditEventKind::Applied { result_ids: vec![] },
        },
    ];
    let entries = fold_edit_log(events);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].edit_id, "e1");
    assert!(matches!(entries[0].state, EditState::Applied { .. }));
    assert_eq!(entries[0].created_at, 10);
    assert_eq!(entries[0].updated_at, 13);
    assert!(entries[1].state.is_pending());
}

#[test]
fn attempts_accumulate_and_orphans_are_ignored() {
    let events = vec![
        EditLogEvent {
            edit_id: "orphan".into(),
            at: 1,
            kind: EditEventKind::AttemptFailed {
                error: "no queued".into(),
            },
        },
        queued("e1", 10),
        EditLogEvent {
            edit_id: "e1".into(),
            at: 11,
            kind: EditEventKind::AttemptFailed { error: "a".into() },
        },
        EditLogEvent {
            edit_id: "e1".into(),
            at: 12,
            kind: EditEventKind::AttemptFailed { error: "b".into() },
        },
    ];
    let entries = fold_edit_log(events);
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].state,
        EditState::Pending {
            attempts: 2,
            last_error: Some("b".into())
        }
    );
}

#[test]
fn event_lines_round_trip_as_json() {
    let event = queued("e1", 10);
    let line = serde_json::to_string(&event).unwrap();
    assert!(line.contains("\"event\":\"queued\""));
    let back: EditLogEvent = serde_json::from_str(&line).unwrap();
    assert_eq!(back, event);
}

#[test]
fn edit_ids_are_unique_for_identical_ops() {
    let op = EditOp::Delete {
        artist: "A".into(),
        track: "x".into(),
        uts: 100,
    };
    assert_ne!(new_edit_id(&op, 5), new_edit_id(&op, 5));
}
