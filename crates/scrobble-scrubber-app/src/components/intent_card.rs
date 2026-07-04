//! One edit intent, rendered as a reviewable card with diff, progress, and
//! context-appropriate actions.

use crate::core::BackendCommand;
use crate::model::{edit_diff, fmt_ts};
use crate::CoreSignal;
use dioxus::prelude::*;
use scrobble_scrubber::{work_status, EditIntent, InstanceStatus, IntentState, WorkStatus};

/// Which view the card is rendered in; picks the action row and state wording.
#[derive(Clone, Copy, PartialEq)]
pub enum CardContext {
    Review,
    WorkQueue,
    History,
}

#[component]
pub fn IntentCard(intent: EditIntent, context: CardContext) -> Element {
    let core = use_context::<CoreSignal>();
    let Some(Ok(core)) = core.read().clone() else {
        return rsx! {};
    };
    // Queue mutations go through the backend channel (not the actor's serial command
    // channel) so they stay responsive while an execute pass is running.
    let backend = core.backend.clone();
    let backend_reject = backend.clone();
    let backend_dismiss = backend.clone();
    let backend_remove = backend.clone();
    let backend_reinstate = backend.clone();
    let id = intent.id;

    let diffs = edit_diff(&intent.subject, &intent.proposed);
    let (state_class, state_label) = state_pill(&intent, context);
    let created = fmt_ts(intent.created_at);
    let updated = fmt_ts(intent.updated_at);
    let subject_line = intent.subject.to_string();

    let total = intent.instances.len();
    let done = intent.done_count();
    let failed = intent.failed_count();
    let pending = intent.pending_count();
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
                    if pending > 0 {
                        span { ", {pending} pending" }
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
            match context {
                CardContext::Review => rsx! {
                    div { class: "actions",
                        button {
                            class: "btn primary",
                            onclick: move |_| {
                                let _ = backend.try_send(BackendCommand::Approve(id));
                            },
                            "Approve"
                        }
                        button {
                            class: "btn",
                            onclick: move |_| {
                                let _ = backend_reject
                                    .try_send(BackendCommand::Reject {
                                        id,
                                        dismiss: false,
                                    });
                            },
                            "Reject"
                        }
                        button {
                            class: "btn danger",
                            onclick: move |_| {
                                let _ = backend_dismiss
                                    .try_send(BackendCommand::Reject {
                                        id,
                                        dismiss: true,
                                    });
                            },
                            "Reject + dismiss subject"
                        }
                    }
                },
                CardContext::WorkQueue => rsx! {
                    div { class: "actions",
                        button {
                            class: "btn danger",
                            title: "reject this accepted intent so the executor skips it",
                            onclick: move |_| {
                                let _ = backend_remove
                                    .try_send(BackendCommand::Reject {
                                        id,
                                        dismiss: false,
                                    });
                            },
                            "Remove from queue"
                        }
                    }
                },
                CardContext::History => {
                    if matches!(intent.state, IntentState::Rejected { .. }) {
                        rsx! {
                            div { class: "actions",
                                button {
                                    class: "btn",
                                    onclick: move |_| {
                                        let _ = backend_reinstate
                                            .try_send(BackendCommand::Reinstate(id));
                                    },
                                    "Reinstate"
                                }
                            }
                        }
                    } else {
                        rsx! {}
                    }
                }
            }
        }
    }
}

/// State pill wording per context, derived from the projection axes where that reads
/// better than the raw lifecycle state.
fn state_pill(intent: &EditIntent, context: CardContext) -> (&'static str, String) {
    match context {
        CardContext::Review => ("warn", "needs review".to_string()),
        CardContext::WorkQueue => match work_status(intent) {
            Some(WorkStatus::Partial {
                applied,
                failed,
                pending,
            }) => {
                let total = applied + failed + pending;
                ("accent", format!("partial — {applied}/{total}"))
            }
            _ => ("accent", "queued".to_string()),
        },
        CardContext::History => match &intent.state {
            IntentState::Applied => ("ok", "applied".to_string()),
            IntentState::Rejected { dismissed: true } => {
                ("danger", "rejected + dismissed".to_string())
            }
            IntentState::Rejected { dismissed: false } => ("danger", "rejected".to_string()),
            IntentState::Abandoned { .. } => ("danger", "abandoned".to_string()),
            // Non-terminal states shouldn't reach History; fall back to the raw name.
            IntentState::AwaitingApproval => ("warn", "awaiting approval".to_string()),
            IntentState::Ready => ("accent", "ready".to_string()),
            IntentState::InProgress => ("accent", "in progress".to_string()),
        },
    }
}
