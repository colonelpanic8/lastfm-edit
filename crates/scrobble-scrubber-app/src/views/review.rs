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
                        onclick: move |_| {
                            let Some(Ok(core)) = core.read().clone() else { return };
                            if let Some(intents) = &*queue.read() {
                                for intent in intents
                                    .iter()
                                    .filter(|i| review_status(i) == ReviewStatus::NeedsReview)
                                {
                                    let _ = core
                                        .backend
                                        .try_send(BackendCommand::Approve(intent.id));
                                }
                            }
                        },
                        "Approve all ({count})"
                    }
                }
                for intent in pending {
                    IntentCard { intent: intent.clone(), context: CardContext::Review }
                }
            }
        }
    }
}
