//! History: terminal outcomes — applied, rejected, and abandoned intents.

use crate::components::{CardContext, IntentCard};
use crate::views::use_queue;
use dioxus::prelude::*;
use scrobble_scrubber::{EditIntent, IntentState};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Applied,
    Rejected,
    Abandoned,
}

const TABS: [(Tab, &str); 3] = [
    (Tab::Applied, "Applied"),
    (Tab::Rejected, "Rejected"),
    (Tab::Abandoned, "Abandoned"),
];

fn matches(tab: Tab, state: &IntentState) -> bool {
    match tab {
        Tab::Applied => *state == IntentState::Applied,
        Tab::Rejected => matches!(state, IntentState::Rejected { .. }),
        Tab::Abandoned => matches!(state, IntentState::Abandoned { .. }),
    }
}

#[component]
pub fn History() -> Element {
    let queue = use_queue();
    let mut tab = use_signal(|| Tab::Applied);

    let queue_read = queue.read();
    let Some(intents) = &*queue_read else {
        return rsx! {
            div { class: "page",
                h1 { "History" }
                div { class: "card muted", "loading…" }
            }
        };
    };

    let count = |t: Tab| intents.iter().filter(|i| matches(t, &i.state)).count();

    let mut visible: Vec<&EditIntent> = intents
        .iter()
        .filter(|i| matches(tab(), &i.state))
        .collect();
    visible.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

    let empty_label = match tab() {
        Tab::Applied => "nothing applied yet",
        Tab::Rejected => "nothing rejected",
        Tab::Abandoned => "nothing abandoned",
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
            if visible.is_empty() {
                div { class: "card muted", "{empty_label}" }
            }
            for intent in visible {
                IntentCard { intent: intent.clone(), context: CardContext::History }
            }
        }
    }
}
