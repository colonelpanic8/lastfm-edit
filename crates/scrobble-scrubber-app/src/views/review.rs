//! Review: the human inbox — every intent awaiting a verdict, newest first.

use crate::components::{CardContext, IntentCard};
use crate::core::BackendCommand;
use crate::views::use_queue;
use crate::CoreSignal;
use dioxus::prelude::*;
use scrobble_scrubber::{review_status, EditIntent, ReviewStatus};

#[component]
pub fn Review() -> Element {
    let core = use_context::<CoreSignal>();
    let queue = use_queue();
    let mut approve_progress = use_signal(|| None::<(usize, usize)>);
    let mut approve_error = use_signal(|| None::<String>);

    let queue_read = queue.read();
    let Some(intents) = &*queue_read else {
        return rsx! {
            div { class: "page",
                h1 { "Review" }
                div { class: "card muted", "loading…" }
            }
        };
    };

    let mut pending: Vec<&EditIntent> = intents
        .iter()
        .filter(|i| review_status(i) == ReviewStatus::NeedsReview)
        .collect();
    pending.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    let count = pending.len();
    let header = format!("{count} proposals need review");

    rsx! {
        div { class: "page",
            h1 { "Review" }
            if count == 0 {
                div { class: "card muted", "inbox zero — nothing needs review" }
            } else {
                div { class: "row", style: "margin-bottom: 12px;",
                    span { class: "headline-count", "{header}" }
                    button {
                        class: "btn primary",
                        disabled: approve_progress().is_some(),
                        onclick: move |_| {
                            let Some(Ok(core)) = core.read().clone() else { return };
                            if let Some(intents) = &*queue.read() {
                                let ids: Vec<_> = intents
                                    .iter()
                                    .filter(|i| review_status(i) == ReviewStatus::NeedsReview)
                                    .map(|intent| intent.id)
                                    .collect();
                                let total = ids.len();
                                let backend = core.backend.clone();
                                approve_progress.set(Some((0, total)));
                                approve_error.set(None);
                                spawn(async move {
                                    for (index, id) in ids.into_iter().enumerate() {
                                        let (completed, receiver) = tokio::sync::oneshot::channel();
                                        if backend
                                            .send(BackendCommand::Approve { id, completed })
                                            .await
                                            .is_err()
                                        {
                                            approve_error.set(Some("the backend is not available".into()));
                                            break;
                                        }
                                        match receiver.await {
                                            Ok(Ok(())) => approve_progress.set(Some((index + 1, total))),
                                            Ok(Err(error)) => {
                                                approve_error.set(Some(error));
                                                break;
                                            }
                                            Err(_) => {
                                                approve_error.set(Some("the backend stopped unexpectedly".into()));
                                                break;
                                            }
                                        }
                                    }
                                    approve_progress.set(None);
                                });
                            }
                        },
                        if let Some((done, total)) = approve_progress() {
                            "Approving… {done}/{total}"
                        } else {
                            "Approve all ({count})"
                        }
                    }
                }
                if let Some(error) = approve_error() {
                    div { class: "banner danger", role: "alert", "{error}" }
                }
                for intent in pending {
                    IntentCard {
                        key: "{intent.id}",
                        intent: intent.clone(),
                        context: CardContext::Review,
                    }
                }
            }
        }
    }
}
