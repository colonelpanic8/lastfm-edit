//! View shell: two-page navigation without a router.

pub mod dashboard;
pub mod queue;

use crate::CoreSignal;
use dioxus::prelude::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ActiveView {
    Dashboard,
    Queue,
}

#[component]
pub fn Shell() -> Element {
    // Dev hook: SCRUBBER_APP_VIEW=queue opens on the queue page.
    let active = use_signal(|| match std::env::var("SCRUBBER_APP_VIEW").as_deref() {
        Ok("queue") => ActiveView::Queue,
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
                            ActiveView::Queue => rsx! { queue::Queue {} },
                        }
                    },
                }
            }
        }
    }
}

#[component]
fn Nav(active: Signal<ActiveView>) -> Element {
    let item = |view: ActiveView, label: &'static str| {
        let is_active = active() == view;
        rsx! {
            button {
                class: if is_active { "nav-item active" } else { "nav-item" },
                onclick: move |_| active.set(view),
                "{label}"
            }
        }
    };
    rsx! {
        nav { class: "sidebar",
            div { class: "brand", "Scrobble Scrubber" }
            {item(ActiveView::Dashboard, "Dashboard")}
            {item(ActiveView::Queue, "Queue")}
        }
    }
}
