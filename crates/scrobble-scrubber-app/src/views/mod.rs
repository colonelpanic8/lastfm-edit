//! View shell: four-page navigation without a router.

pub mod dashboard;
pub mod history;
pub mod review;
pub mod work_queue;

use crate::{CoreSignal, UiSignal};
use dioxus::prelude::*;
use scrobble_scrubber::{review_status, EditIntent, ReviewStatus, ScrubberState};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ActiveView {
    Dashboard,
    Review,
    WorkQueue,
    History,
}

/// Load the durable queue, reloading whenever the queue epoch bumps (not on every log
/// event). Each caller gets its own resource instance; that matches the pre-split
/// behavior where each view loaded independently.
pub(crate) fn use_queue() -> Resource<Vec<EditIntent>> {
    let core = use_context::<CoreSignal>();
    let ui = use_context::<UiSignal>();
    let epoch = use_memo(move || ui.read().queue_epoch);
    use_resource(move || async move {
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
    })
}

#[component]
pub fn Shell() -> Element {
    // Dev hook: SCRUBBER_APP_VIEW=review opens on the review page ("queue" is the
    // legacy alias for the work queue).
    let active = use_signal(|| match std::env::var("SCRUBBER_APP_VIEW").as_deref() {
        Ok("review") => ActiveView::Review,
        Ok("work") | Ok("work-queue") | Ok("queue") => ActiveView::WorkQueue,
        Ok("history") => ActiveView::History,
        _ => ActiveView::Dashboard,
    });
    let core = use_context::<CoreSignal>();

    rsx! {
        div { class: "shell",
            Nav { active }
            main { class: "content",
                match &*core.read() {
                    None => rsx! {
                        div { class: "page", div { class: "card muted", "starting…" } }
                    },
                    Some(Err(error)) => rsx! {
                        div { class: "page",
                            h1 { "Cannot start" }
                            div { class: "banner danger", "{error}" }
                        }
                    },
                    Some(Ok(_)) => rsx! {
                        match active() {
                            ActiveView::Dashboard => rsx! { dashboard::Dashboard {} },
                            ActiveView::Review => rsx! { review::Review {} },
                            ActiveView::WorkQueue => rsx! { work_queue::WorkQueue {} },
                            ActiveView::History => rsx! { history::History {} },
                        }
                    },
                }
            }
        }
    }
}

#[component]
fn Nav(active: Signal<ActiveView>) -> Element {
    let queue = use_queue();
    let needs_review = queue
        .read()
        .as_ref()
        .map(|intents| {
            intents
                .iter()
                .filter(|i| review_status(i) == ReviewStatus::NeedsReview)
                .count()
        })
        .unwrap_or(0);

    let item = |view: ActiveView, label: &'static str, badge: usize| {
        let is_active = active() == view;
        rsx! {
            button {
                class: if is_active { "nav-item active" } else { "nav-item" },
                onclick: move |_| active.set(view),
                "{label}"
                if badge > 0 {
                    span { class: "nav-badge", "{badge}" }
                }
            }
        }
    };
    rsx! {
        nav { class: "sidebar",
            div { class: "brand", "Scrobble Scrubber" }
            {item(ActiveView::Dashboard, "Dashboard", 0)}
            {item(ActiveView::Review, "Review", needs_review)}
            {item(ActiveView::WorkQueue, "Work Queue", 0)}
            {item(ActiveView::History, "History", 0)}
        }
    }
}
