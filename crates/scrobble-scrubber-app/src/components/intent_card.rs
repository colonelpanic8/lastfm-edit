//! One edit intent, rendered as a reviewable card with diff, progress, and actions.

use crate::model::{edit_diff, fmt_ts};
use crate::CoreSignal;
use dioxus::prelude::*;
use scrobble_scrubber::{EditIntent, InstanceStatus, IntentState, ScrubberCommand};

#[component]
pub fn IntentCard(intent: EditIntent) -> Element {
    let core = use_context::<CoreSignal>();
    let Some(Ok(core)) = core.read().clone() else {
        return rsx! {};
    };
    let handle = core.handle.clone();
    let handle_reject = handle.clone();
    let handle_dismiss = handle.clone();
    let handle_reinstate = handle.clone();
    let id = intent.id;

    let diffs = edit_diff(&intent.subject, &intent.proposed);
    let (state_class, state_label) = state_pill(&intent.state);
    let created = fmt_ts(intent.created_at);
    let updated = fmt_ts(intent.updated_at);
    let subject_line = intent.subject.to_string();

    let total = intent.instances.len();
    let done = intent.done_count();
    let failed = intent.failed_count();
    let percent = if total > 0 { done * 100 / total } else { 0 };
    let failures: Vec<(String, u32, String)> = intent
        .instances
        .iter()
        .filter_map(|(sid, status)| match status {
            InstanceStatus::Failed {
                attempts,
                last_error,
            } => Some((sid.to_string(), *attempts, last_error.clone())),
            _ => None,
        })
        .collect();

    rsx! {
        div { class: "intent",
            div { class: "subject", "{subject_line}" }
            div { class: "meta",
                span { class: "pill {state_class}", "{state_label}" }
                span { class: "pill", "{intent.provider}" }
                if intent.requires_approval {
                    span { class: "pill accent", "needs approval" }
                }
                span { "created {created}" }
                span { "updated {updated}" }
            }
            if diffs.is_empty() {
                div { class: "muted", "no field changes (metadata-only intent)" }
            } else {
                div { class: "diff",
                    for diff in diffs {
                        div { class: "diff-row",
                            span { class: "field", "{diff.field}" }
                            span { class: "diff-from", "{diff.from}" }
                            span { class: "diff-arrow", "→" }
                            span { class: "diff-to", "{diff.to}" }
                        }
                    }
                }
            }
            if total > 0 {
                div { class: "progress",
                    "{done}/{total} applied"
                    if failed > 0 {
                        span { ", {failed} failed" }
                    }
                    div { class: "bar",
                        div { class: "fill", style: "width: {percent}%;" }
                    }
                }
            }
            for (sid , attempts , error) in failures {
                div { class: "banner danger mono",
                    "{sid}: {error} (attempt {attempts})"
                }
            }
            if let IntentState::Abandoned { reason } = &intent.state {
                div { class: "banner warn", "abandoned: {reason}" }
            }
            match intent.state {
                IntentState::AwaitingApproval => rsx! {
                    div { class: "actions",
                        button {
                            class: "btn primary",
                            onclick: move |_| {
                                let _ = handle.try_send(ScrubberCommand::Approve(id));
                            },
                            "Approve"
                        }
                        button {
                            class: "btn",
                            onclick: move |_| {
                                let _ = handle_reject
                                    .try_send(ScrubberCommand::Reject {
                                        id,
                                        dismiss: false,
                                    });
                            },
                            "Reject"
                        }
                        button {
                            class: "btn danger",
                            onclick: move |_| {
                                let _ = handle_dismiss
                                    .try_send(ScrubberCommand::Reject {
                                        id,
                                        dismiss: true,
                                    });
                            },
                            "Reject + dismiss subject"
                        }
                    }
                },
                IntentState::Ready | IntentState::InProgress => rsx! {
                    div { class: "actions",
                        button {
                            class: "btn danger",
                            onclick: move |_| {
                                let _ = handle_reject
                                    .try_send(ScrubberCommand::Reject {
                                        id,
                                        dismiss: false,
                                    });
                            },
                            "Reject"
                        }
                    }
                },
                IntentState::Rejected { .. } => rsx! {
                    div { class: "actions",
                        button {
                            class: "btn",
                            onclick: move |_| {
                                let _ = handle_reinstate.try_send(ScrubberCommand::Reinstate(id));
                            },
                            "Reinstate"
                        }
                    }
                },
                _ => rsx! {},
            }
        }
    }
}

fn state_pill(state: &IntentState) -> (&'static str, &'static str) {
    match state {
        IntentState::AwaitingApproval => ("warn", "awaiting approval"),
        IntentState::Ready => ("accent", "ready"),
        IntentState::InProgress => ("accent", "in progress"),
        IntentState::Applied => ("ok", "applied"),
        IntentState::Rejected { dismissed: false } => ("danger", "rejected"),
        IntentState::Rejected { dismissed: true } => ("danger", "rejected + dismissed"),
        IntentState::Abandoned { .. } => ("danger", "abandoned"),
    }
}
