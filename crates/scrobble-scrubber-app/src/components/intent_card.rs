//! One edit intent, rendered as a reviewable card with diff, progress, and
//! context-appropriate actions.

use crate::core::BackendCommand;
use crate::model::{edit_diff, fmt_ts};
use crate::CoreSignal;
use dioxus::prelude::*;
use scrobble_scrubber::{work_status, EditIntent, InstanceStatus, IntentState, WorkStatus};
use tokio::sync::{mpsc, oneshot};

fn dispatch_action(
    backend: mpsc::Sender<BackendCommand>,
    label: &'static str,
    command: impl FnOnce(oneshot::Sender<Result<(), String>>) -> BackendCommand + 'static,
    mut pending: Signal<Option<&'static str>>,
    mut action_error: Signal<Option<String>>,
) {
    if pending.peek().is_some() {
        return;
    }
    pending.set(Some(label));
    action_error.set(None);
    let (completed, receiver) = oneshot::channel();
    let command = command(completed);
    spawn(async move {
        let result = match backend.send(command).await {
            Ok(()) => receiver
                .await
                .unwrap_or_else(|_| Err("the backend stopped unexpectedly".into())),
            Err(_) => Err("the backend is not available".into()),
        };
        pending.set(None);
        if let Err(error) = result {
            action_error.set(Some(error));
        }
    });
}

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
    let pending = use_signal(|| None::<&'static str>);
    let action_error = use_signal(|| None::<String>);
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
    let pending_count = intent.pending_count();
    let percent = (done * 100).checked_div(total).unwrap_or(0);
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
                    if pending_count > 0 {
                        span { ", {pending_count} pending" }
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
            if let Some(error) = action_error() {
                div { class: "banner danger", role: "alert", "{error}" }
            }
            match context {
                CardContext::Review => rsx! {
                    div { class: "actions",
                        button {
                            class: "btn primary",
                            disabled: pending().is_some(),
                            onclick: move |_| {
                                dispatch_action(
                                    backend.clone(),
                                    "Approving…",
                                    move |completed| BackendCommand::Approve { id, completed },
                                    pending,
                                    action_error,
                                );
                            },
                            "Approve"
                        }
                        button {
                            class: "btn",
                            disabled: pending().is_some(),
                            onclick: move |_| {
                                dispatch_action(
                                    backend_reject.clone(),
                                    "Rejecting…",
                                    move |completed| BackendCommand::Reject {
                                        id,
                                        dismiss: false,
                                        completed,
                                    },
                                    pending,
                                    action_error,
                                );
                            },
                            "Reject"
                        }
                        button {
                            class: "btn danger",
                            disabled: pending().is_some(),
                            onclick: move |_| {
                                dispatch_action(
                                    backend_dismiss.clone(),
                                    "Dismissing…",
                                    move |completed| BackendCommand::Reject {
                                        id,
                                        dismiss: true,
                                        completed,
                                    },
                                    pending,
                                    action_error,
                                );
                            },
                            "Reject + dismiss subject"
                        }
                        if let Some(label) = pending() {
                            span { class: "pill accent", "{label}" }
                        }
                    }
                },
                CardContext::WorkQueue => rsx! {
                    div { class: "actions",
                        button {
                            class: "btn danger",
                            title: "reject this accepted intent so the executor skips it",
                            disabled: pending().is_some(),
                            onclick: move |_| {
                                dispatch_action(
                                    backend_remove.clone(),
                                    "Removing…",
                                    move |completed| BackendCommand::Reject {
                                        id,
                                        dismiss: false,
                                        completed,
                                    },
                                    pending,
                                    action_error,
                                );
                            },
                            if let Some(label) = pending() { "{label}" } else { "Remove from queue" }
                        }
                    }
                },
                CardContext::History => {
                    if matches!(intent.state, IntentState::Rejected { .. }) {
                        rsx! {
                            div { class: "actions",
                                button {
                                    class: "btn",
                                    disabled: pending().is_some(),
                                    onclick: move |_| {
                                        dispatch_action(
                                            backend_reinstate.clone(),
                                            "Reinstating…",
                                            move |completed| BackendCommand::Reinstate { id, completed },
                                            pending,
                                            action_error,
                                        );
                                    },
                                    if let Some(label) = pending() { "{label}" } else { "Reinstate" }
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
