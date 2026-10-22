use crate::model::{execution_failure_copy, fmt_ts, PassState};
use crate::UiSignal;
use dioxus::prelude::*;

#[component]
pub fn ExecutionNotice() -> Element {
    let ui = use_context::<UiSignal>();
    let state = ui.read();
    match &state.pass {
        PassState::Failed {
            progress,
            error,
            kind,
        } => {
            let (title, action) = execution_failure_copy(*kind);
            rsx! {
                div { class: "banner danger execution-notice", role: "alert",
                    strong { "{title}" }
                    p { "{action}" }
                    if let Some(current) = &progress.current {
                        p { "While processing: {current.subject}" }
                    }
                    p { "Completed edits are saved. Unfinished edits stay queued; retrying resumes the queue." }
                    details {
                        summary { "Technical details" }
                        pre { "{error}" }
                    }
                }
            }
        }
        _ => match &state.last_edit_failure {
            Some(failure) => {
                let title = if state.pass.is_active() {
                    "An edit failed — continuing with the queue"
                } else {
                    "Some edits could not be applied"
                };
                let played = fmt_ts(failure.instance.uts());
                rsx! {
                    div { class: "banner warn execution-notice", role: "alert",
                        strong { "{title}" }
                        p { "Most recent failure: {failure.subject} · played {played}" }
                        p { "This attempt did not complete. Check the details below. The Activity errors filter shows other failures from this run." }
                        details {
                            summary { "Technical details" }
                            pre { "{failure.error}" }
                        }
                    }
                }
            }
            None => rsx! {},
        },
    }
}

#[component]
pub fn ErrorNotice() -> Element {
    let mut ui = use_context::<UiSignal>();
    let error = ui.read().error.clone();
    rsx! {
        if let Some(error) = error {
            div { class: "banner danger execution-notice", role: "alert",
                div { class: "row",
                    strong { "An action could not finish" }
                    button {
                        class: "btn",
                        onclick: move |_| ui.with_mut(|state| state.error = None),
                        "Dismiss"
                    }
                }
                p { "{error}" }
            }
        }
    }
}
