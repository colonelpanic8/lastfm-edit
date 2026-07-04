//! Queue review: filterable list of edit intents with approve/reject actions.

use crate::components::IntentCard;
use crate::{CoreSignal, UiSignal};
use dioxus::prelude::*;
use scrobble_scrubber::{EditIntent, IntentState, ScrubberCommand, ScrubberState};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Filter {
    Awaiting,
    Ready,
    InProgress,
    Applied,
    Rejected,
    Abandoned,
}

const FILTERS: [(Filter, &str); 6] = [
    (Filter::Awaiting, "Awaiting"),
    (Filter::Ready, "Ready"),
    (Filter::InProgress, "In progress"),
    (Filter::Applied, "Applied"),
    (Filter::Rejected, "Rejected"),
    (Filter::Abandoned, "Abandoned"),
];

fn matches(filter: Filter, state: &IntentState) -> bool {
    match filter {
        Filter::Awaiting => *state == IntentState::AwaitingApproval,
        Filter::Ready => *state == IntentState::Ready,
        Filter::InProgress => *state == IntentState::InProgress,
        Filter::Applied => *state == IntentState::Applied,
        Filter::Rejected => matches!(state, IntentState::Rejected { .. }),
        Filter::Abandoned => matches!(state, IntentState::Abandoned { .. }),
    }
}

#[component]
pub fn Queue() -> Element {
    let core = use_context::<CoreSignal>();
    let ui = use_context::<UiSignal>();
    let mut filter = use_signal(|| Filter::Awaiting);

    // Reload only when the queue actually changes, not on every log event.
    let epoch = use_memo(move || ui.read().queue_epoch);
    let queue = use_resource(move || async move {
        let _reload_on = epoch();
        let Some(Ok(core)) = core.read().clone() else {
            return Vec::new();
        };
        match core.state.load_queue().await {
            Ok(intents) => {
                tracing::debug!(count = intents.len(), "queue loaded");
                intents
            }
            Err(error) => {
                tracing::warn!(%error, "load_queue failed");
                Vec::new()
            }
        }
    });

    let queue_read = queue.read();
    let Some(intents) = &*queue_read else {
        return rsx! {
            div { class: "page",
                h1 { "Queue" }
                div { class: "card muted", "loading…" }
            }
        };
    };

    let count = |f: Filter| intents.iter().filter(|i| matches(f, &i.state)).count();
    let awaiting_count = count(Filter::Awaiting);

    let mut visible: Vec<&EditIntent> = intents
        .iter()
        .filter(|i| matches(filter(), &i.state))
        .collect();
    visible.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

    rsx! {
        div { class: "page",
            h1 { "Queue" }
            div { class: "tabs",
                for (f , label) in FILTERS {
                    {
                        let n = count(f);
                        let class = if filter() == f { "tab active" } else { "tab" };
                        rsx! {
                            button {
                                class,
                                onclick: move |_| filter.set(f),
                                "{label}"
                                span { class: "count", "{n}" }
                            }
                        }
                    }
                }
            }
            if filter() == Filter::Awaiting && awaiting_count > 0 {
                div { class: "row", style: "margin-bottom: 12px;",
                    button {
                        class: "btn primary",
                        onclick: move |_| {
                            let Some(Ok(core)) = core.read().clone() else { return };
                            if let Some(intents) = &*queue.read() {
                                for intent in intents
                                    .iter()
                                    .filter(|i| i.state == IntentState::AwaitingApproval)
                                {
                                    let _ = core
                                        .handle
                                        .try_send(ScrubberCommand::Approve(intent.id));
                                }
                            }
                        },
                        "Approve all awaiting ({awaiting_count})"
                    }
                }
            }
            if visible.is_empty() {
                div { class: "card muted", "nothing here" }
            }
            for intent in visible {
                IntentCard { intent: intent.clone() }
            }
        }
    }
}
