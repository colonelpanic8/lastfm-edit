//! Edit log data model: append-only events, folded into per-edit state.

use crate::id::ScrobbleId;
use lastfm_edit::ExactScrobbleEdit;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicU64, Ordering};

/// A mirrored operation. Always exact and single-scrobble (see module docs on `edits`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EditOp {
    Edit(ExactScrobbleEdit),
    Delete {
        artist: String,
        track: String,
        uts: u64,
    },
}

/// One line in `edits/log.jsonl`. The log is append-only; the sequence of events for an
/// `edit_id` folds into its current [`EditState`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EditLogEvent {
    pub edit_id: String,
    /// When the event was recorded (Unix seconds).
    pub at: u64,
    #[serde(flatten)]
    pub kind: EditEventKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum EditEventKind {
    /// Durable intent, written before touching Last.fm.
    Queued {
        op: EditOp,
        target_ids: Vec<ScrobbleId>,
    },
    /// An upstream attempt failed (retriable).
    AttemptFailed { error: String },
    /// Applied upstream and mirrored locally.
    Applied { result_ids: Vec<ScrobbleId> },
    /// Explicitly given up on; terminal.
    Abandoned { reason: String },
}

/// Folded state of one edit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum EditState {
    Pending {
        attempts: u32,
        last_error: Option<String>,
    },
    Applied {
        result_ids: Vec<ScrobbleId>,
    },
    Abandoned {
        reason: String,
    },
}

impl EditState {
    pub fn is_pending(&self) -> bool {
        matches!(self, EditState::Pending { .. })
    }
}

/// Folded view of one edit's event history.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EditLogEntry {
    pub edit_id: String,
    pub op: EditOp,
    pub target_ids: Vec<ScrobbleId>,
    pub state: EditState,
    pub created_at: u64,
    pub updated_at: u64,
}

/// Fold an ordered event stream into per-edit entries (in first-queued order).
pub fn fold_edit_log(events: impl IntoIterator<Item = EditLogEvent>) -> Vec<EditLogEntry> {
    let mut order: Vec<String> = Vec::new();
    let mut entries: std::collections::HashMap<String, EditLogEntry> =
        std::collections::HashMap::new();
    for event in events {
        match event.kind {
            EditEventKind::Queued { op, target_ids } => {
                if entries.contains_key(&event.edit_id) {
                    log::warn!("edit log: duplicate Queued for {}; ignoring", event.edit_id);
                    continue;
                }
                order.push(event.edit_id.clone());
                entries.insert(
                    event.edit_id.clone(),
                    EditLogEntry {
                        edit_id: event.edit_id,
                        op,
                        target_ids,
                        state: EditState::Pending {
                            attempts: 0,
                            last_error: None,
                        },
                        created_at: event.at,
                        updated_at: event.at,
                    },
                );
            }
            kind => {
                let Some(entry) = entries.get_mut(&event.edit_id) else {
                    log::warn!(
                        "edit log: event for unknown edit {}; ignoring",
                        event.edit_id
                    );
                    continue;
                };
                entry.updated_at = event.at;
                match kind {
                    EditEventKind::AttemptFailed { error } => {
                        if let EditState::Pending {
                            attempts,
                            last_error,
                        } = &mut entry.state
                        {
                            *attempts += 1;
                            *last_error = Some(error);
                        }
                    }
                    EditEventKind::Applied { result_ids } => {
                        entry.state = EditState::Applied { result_ids };
                    }
                    EditEventKind::Abandoned { reason } => {
                        entry.state = EditState::Abandoned { reason };
                    }
                    EditEventKind::Queued { .. } => unreachable!("handled above"),
                }
            }
        }
    }
    order
        .into_iter()
        .filter_map(|id| entries.remove(&id))
        .collect()
}

/// Build a unique edit id: creation time + content hash + a process-local counter (the
/// counter disambiguates identical ops queued within the same second).
pub(crate) fn new_edit_id(op: &EditOp, at: u64) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut hasher = Sha256::new();
    hasher.update(serde_json::to_string(op).unwrap_or_default().as_bytes());
    let digest = hasher.finalize();
    let hex8: String = digest[..4].iter().map(|b| format!("{b:02x}")).collect();
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("e{at}-{hex8}-{seq}")
}

#[cfg(test)]
mod tests {
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
}
