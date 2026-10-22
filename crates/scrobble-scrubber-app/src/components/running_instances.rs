use crate::model::{edit_step_label, fmt_ts, runner_activity_label, runner_kind_label};
use crate::{CoreSignal, RunnerSignal, UiSignal};
use dioxus::prelude::*;

#[component]
pub fn RunningInstances() -> Element {
    let core = use_context::<CoreSignal>();
    let ui = use_context::<UiSignal>();
    let runners = use_context::<RunnerSignal>();
    let mut now = use_signal(|| chrono::Utc::now().timestamp());
    use_future(move || async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            now.set(chrono::Utc::now().timestamp());
        }
    });
    let own_id = core
        .read()
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .map(|core| core.runner_id);
    let snapshot = runners.read();
    rsx! {
        section { class: "card running-instances", aria_label: "Running instances",
            strong { "Running instances" }
            p { class: "muted", "Live processes using this store. Only one instance can edit this queue at a time." }
            match snapshot.as_ref() {
                None => rsx! { div { class: "muted", "Checking running instances…" } },
                Some(Err(error)) => rsx! { div { class: "banner danger", role: "alert", "Could not read running instances: {error}" } },
                Some(Ok(snapshot)) => rsx! {
                    if ui.read().last_exec.as_ref().is_some_and(|report| report.ended == scrobble_scrubber::ExecEnded::Busy) && !snapshot.instances.iter().any(|runner| runner.activity.owns_edits()) {
                        div { class: "banner warn",
                            "The last attempt found another editor without live status. It may be an older build; retry after it finishes or update that instance."
                        }
                    }
                    for runner in &snapshot.instances {
                        {
                            let kind = runner_kind_label(runner.kind);
                            let activity = runner_activity_label(&runner.activity, now());
                            let started = fmt_ts(runner.started_at);
                            let since_update = (now() - runner.updated_at as i64).max(0);
                            let owner = runner.activity.owns_edits();
                            rsx! {
                                div { class: "runner", key: "{runner.id}",
                                    div { class: "row",
                                        strong { "{kind}" }
                                        span { class: "muted", "PID {runner.pid}" }
                                        if Some(runner.id) == own_id { span { class: "pill", "This window" } }
                                        if owner { span { class: "pill accent", "Owns edit queue" } }
                                    }
                                    div { class: "runner-activity", "{activity}" }
                                    if let Some(subject) = &runner.subject {
                                        div { "{subject}" }
                                    }
                                    if runner.activity == scrobble_scrubber::RunnerActivity::Editing {
                                        if let Some(step) = &runner.step {
                                            div { class: "muted", {edit_step_label(step, now())} }
                                        }
                                    }
                                    div { class: "muted",
                                        "{runner.applied} applied · {runner.failed} failed · last activity {since_update}s ago"
                                    }
                                    details {
                                        summary { class: "muted", "Started {started}" }
                                        div { class: "mono", "Instance {runner.id}" }
                                    }
                                    if let Some(error) = &runner.last_error {
                                        details { class: "runner-error",
                                            summary { "Latest error" }
                                            pre { "{error}" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                },
            }
        }
    }
}
