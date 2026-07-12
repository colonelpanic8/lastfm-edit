//! History: terminal outcomes — applied, rejected, and abandoned intents, plus the edit log.

use crate::components::{CardContext, IntentCard};
use crate::model::fmt_ts;
use crate::views::use_queue;
use crate::{CoreSignal, UiSignal};
use dioxus::prelude::*;
use scrobble_scrubber::{EditIntent, IntentState};
use scrobble_store::{EditLogEntry, EditOp, EditState, Storage};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Applied,
    Rejected,
    Abandoned,
    EditLog,
}

const TABS: [(Tab, &str); 4] = [
    (Tab::Applied, "Applied"),
    (Tab::Rejected, "Rejected"),
    (Tab::Abandoned, "Abandoned"),
    (Tab::EditLog, "Edit Log"),
];

fn matches(tab: Tab, state: &IntentState) -> bool {
    match tab {
        Tab::Applied => *state == IntentState::Applied,
        Tab::Rejected => matches!(state, IntentState::Rejected { .. }),
        Tab::Abandoned => matches!(state, IntentState::Abandoned { .. }),
        Tab::EditLog => false,
    }
}

/// The `(class, label)` for an edit-log state pill.
fn state_pill(state: &EditState) -> (&'static str, &'static str) {
    match state {
        EditState::Pending { .. } => ("accent", "pending"),
        EditState::Applied { .. } => ("ok", "applied"),
        EditState::Abandoned { .. } => ("danger", "abandoned"),
    }
}

/// One-line op title and the changed-field diffs (field, old, new) for an edit-log entry.
fn op_summary(op: &EditOp) -> (String, Vec<(&'static str, String, String)>) {
    match op {
        EditOp::Delete { artist, track, .. } => (format!("Delete: {artist} — {track}"), Vec::new()),
        EditOp::Edit(edit) => {
            let mut diffs = Vec::new();
            let mut push = |field: &'static str, from: &str, to: &str| {
                if from != to {
                    diffs.push((field, from.to_string(), to.to_string()));
                }
            };
            push("artist", &edit.artist_name_original, &edit.artist_name);
            push("track", &edit.track_name_original, &edit.track_name);
            push("album", &edit.album_name_original, &edit.album_name);
            push(
                "album artist",
                &edit.album_artist_name_original,
                &edit.album_artist_name,
            );
            (
                format!("Edit: {} — {}", edit.artist_name, edit.track_name),
                diffs,
            )
        }
    }
}

#[component]
pub fn History() -> Element {
    let core = use_context::<CoreSignal>();
    let ui = use_context::<UiSignal>();
    let queue = use_queue();
    let mut tab = use_signal(|| Tab::Applied);

    let epoch = use_memo(move || ui.read().queue_epoch);
    let edit_log = use_resource(move || async move {
        let _reload_on = epoch();
        let Some(Ok(core)) = core.read().clone() else {
            return Vec::new();
        };
        core.store.load_edit_log().await.unwrap_or_default()
    });

    let queue_read = queue.read();
    let Some(intents) = &*queue_read else {
        return rsx! {
            div { class: "page",
                h1 { "History" }
                div { class: "card muted", "loading…" }
            }
        };
    };

    let log_read = edit_log.read();
    let log_entries = log_read.as_deref().unwrap_or(&[]);

    let count = |t: Tab| match t {
        Tab::EditLog => log_entries.len(),
        _ => intents.iter().filter(|i| matches(t, &i.state)).count(),
    };

    let mut visible: Vec<&EditIntent> = intents
        .iter()
        .filter(|i| matches(tab(), &i.state))
        .collect();
    visible.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

    let mut log_visible: Vec<&EditLogEntry> = log_entries.iter().collect();
    log_visible.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

    let empty_label = match tab() {
        Tab::Applied => "nothing applied yet",
        Tab::Rejected => "nothing rejected",
        Tab::Abandoned => "nothing abandoned",
        Tab::EditLog => "no mirrored edits yet",
    };

    rsx! {
        div { class: "page",
            h1 { "History" }
            div { class: "tabs",
                for (t , label) in TABS {
                    {
                        let n = count(t);
                        let class = if tab() == t { "tab active" } else { "tab" };
                        rsx! {
                            button {
                                class,
                                onclick: move |_| tab.set(t),
                                "{label}"
                                span { class: "count", "{n}" }
                            }
                        }
                    }
                }
            }
            if tab() == Tab::EditLog {
                if log_visible.is_empty() {
                    div { class: "card muted", "{empty_label}" }
                }
                for entry in log_visible {
                    {
                        let (title, diffs) = op_summary(&entry.op);
                        let (pill_class, pill_label) = state_pill(&entry.state);
                        let updated = fmt_ts(entry.updated_at);
                        rsx! {
                            div { class: "intent",
                                div { class: "subject", "{title}" }
                                div { class: "meta",
                                    span { class: "pill {pill_class}", "{pill_label}" }
                                    span { "updated {updated}" }
                                }
                                if !diffs.is_empty() {
                                    div { class: "diff",
                                        for (field , from , to) in diffs {
                                            div { class: "diff-row",
                                                span { class: "field", "{field}" }
                                                span { class: "diff-from", "{from}" }
                                                span { class: "diff-arrow", "→" }
                                                span { class: "diff-to", "{to}" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            } else {
                if visible.is_empty() {
                    div { class: "card muted", "{empty_label}" }
                }
                for intent in visible {
                    IntentCard { intent: intent.clone(), context: CardContext::History }
                }
            }
        }
    }
}
