//! View shell: four-page navigation without a router.

pub mod dashboard;
pub mod history;
pub mod library;
pub mod review;
pub mod work_queue;

use crate::{CoreSignal, UiSignal};
use dioxus::prelude::*;
use scrobble_scrubber::{review_status, EditIntent, ReviewStatus, ScrubberState};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ActiveView {
    Dashboard,
    Library,
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
    let core_snapshot = core.read().clone();

    match &core_snapshot {
        None => rsx! {
            main { class: "login-page", div { class: "muted", "Starting..." } }
        },
        Some(Err(error)) if error.needs_login() => rsx! {
            login::Login { startup_error: error.to_string() }
        },
        Some(Err(error)) => rsx! {
            main { class: "content",
                div { class: "page",
                    h1 { "Cannot start" }
                    div { class: "banner danger", "{error}" }
                }
            }
        },
        Some(Ok(_)) => rsx! {
            div { class: "shell",
                Nav { active }
                main { class: "content",
                        match active() {
                            ActiveView::Dashboard => rsx! { dashboard::Dashboard {} },
                            ActiveView::Library => rsx! { library::Library {} },
                            ActiveView::Review => rsx! { review::Review {} },
                            ActiveView::WorkQueue => rsx! { work_queue::WorkQueue {} },
                            ActiveView::History => rsx! { history::History {} },
                        }
                }
            }
        },
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
            {item(ActiveView::Library, "Library", 0)}
            {item(ActiveView::Review, "Review", needs_review)}
            {item(ActiveView::WorkQueue, "Work Queue", 0)}
            {item(ActiveView::History, "History", 0)}
        }
    }
}

mod login {
    use crate::{core, install_core, CoreSignal, UiSignal};
    use dioxus::prelude::*;

    #[component]
    pub fn Login(startup_error: String) -> Element {
        let core_state = use_context::<CoreSignal>();
        let ui = use_context::<UiSignal>();
        let mut username = use_signal(core::configured_username);
        let mut password = use_signal(String::new);
        let mut api_key = use_signal(String::new);
        let mut submitting = use_signal(|| false);
        let mut login_error = use_signal(|| None::<String>);

        rsx! {
            main { class: "login-page",
                section { class: "login-panel",
                    div { class: "login-brand", "Scrobble Scrubber" }
                    h1 { "Sign in to Last.fm" }
                    form {
                        class: "login-form",
                        onsubmit: move |event| {
                            event.prevent_default();
                            if submitting() {
                                return;
                            }
                            let username_value = username().trim().to_string();
                            let password_value = password();
                            if username_value.is_empty() || password_value.is_empty() {
                                login_error.set(Some("Enter your username and password.".into()));
                                return;
                            }
                            let api_key_value = match api_key().trim() {
                                "" => None,
                                value => Some(value.to_string()),
                            };
                            submitting.set(true);
                            login_error.set(None);
                            let receiver = core::login_and_start(
                                username_value,
                                password_value,
                                api_key_value,
                            );
                            password.set(String::new());
                            spawn(async move {
                                match receiver.await {
                                    Ok(Ok(core)) => install_core(core, core_state, ui),
                                    Ok(Err(error)) => {
                                        login_error.set(Some(error.to_string()));
                                        submitting.set(false);
                                    }
                                    Err(_) => {
                                        login_error.set(Some(
                                            "The login worker stopped unexpectedly.".into(),
                                        ));
                                        submitting.set(false);
                                    }
                                }
                            });
                        },
                        label { r#for: "lastfm-username", "Username or email" }
                        input {
                            id: "lastfm-username",
                            r#type: "text",
                            autocomplete: "username",
                            value: "{username}",
                            disabled: submitting(),
                            oninput: move |event| username.set(event.value()),
                        }
                        label { r#for: "lastfm-password", "Password" }
                        input {
                            id: "lastfm-password",
                            r#type: "password",
                            autocomplete: "current-password",
                            value: "{password}",
                            disabled: submitting(),
                            oninput: move |event| password.set(event.value()),
                        }
                        label { r#for: "lastfm-api-key", "API key (optional)" }
                        input {
                            id: "lastfm-api-key",
                            r#type: "password",
                            autocomplete: "off",
                            value: "{api_key}",
                            disabled: submitting(),
                            oninput: move |event| api_key.set(event.value()),
                        }
                        if let Some(error) = login_error() {
                            div { class: "banner danger", role: "alert", "{error}" }
                        } else {
                            div { class: "login-status muted", "{startup_error}" }
                        }
                        button {
                            class: "btn primary login-submit",
                            r#type: "submit",
                            disabled: submitting(),
                            if submitting() { "Signing in..." } else { "Sign in" }
                        }
                    }
                }
            }
        }
    }
}
