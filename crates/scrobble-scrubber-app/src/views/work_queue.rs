//! Work queue: what is truly queued for execution (in executor order) plus the live
//! pass panel with execute/stop controls.

use crate::components::{CardContext, IntentCard};
use crate::core::BackendCommand;
use crate::model::{ended_label, PassState};
use crate::views::use_queue;
use crate::{CoreSignal, UiSignal};
use dioxus::prelude::*;
use scrobble_scrubber::{work_queue_view, ScrubberCommand};

#[component]
pub fn WorkQueue() -> Element {
    let core = use_context::<CoreSignal>();
    let ui = use_context::<UiSignal>();
    let queue = use_queue();
    let mut budget_input = use_signal(String::new);

    // 1s ticker so the rate-limit countdown stays live.
    let mut now = use_signal(|| chrono::Utc::now().timestamp());
    use_future(move || async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            now.set(chrono::Utc::now().timestamp());
        }
    });

    let queue_read = queue.read();
    let Some(intents) = &*queue_read else {
        return rsx! {
            div { class: "page",
                h1 { "Work Queue" }
                div { class: "card muted", "loading…" }
            }
        };
    };
    let view = work_queue_view(intents.clone());

    let Some(Ok(core)) = core.read().clone() else {
        return rsx! {};
    };
    let handle_exec = core.handle.clone();
    let backend_stop = core.backend.clone();

    let ui_read = ui.read();
    let is_idle = ui_read.pass == PassState::Idle;

    // Precompute pass-panel text (RSX format segments can't contain method calls).
    let (pass_pill_class, status_line, detail_line) = match &ui_read.pass {
        PassState::Idle => {
            let detail = ui_read.last_exec.as_ref().map(|report| {
                let ended = ended_label(&report.ended);
                format!(
                    "last pass: {ended} — {} applied, {} failed",
                    report.instances_applied, report.instances_failed
                )
            });
            ("", "executor idle".to_string(), detail)
        }
        PassState::Running(progress) => {
            let detail = progress.current.as_ref().map(|current| {
                format!(
                    "working on: {} ({} instances)",
                    current.subject, current.instances
                )
            });
            (
                "accent",
                format!(
                    "executing — {} applied, {} failed, {} intents done",
                    progress.applied, progress.failed, progress.intents_done
                ),
                detail,
            )
        }
        PassState::Paused { progress, until } => {
            let countdown = match until {
                Some(until) => {
                    let left = (*until as i64 - now()).max(0);
                    format!("rate limited — resuming in ~{left}s")
                }
                None => "rate limited — waiting it out".to_string(),
            };
            (
                "warn",
                format!(
                    "paused — {} applied, {} failed, {} intents done",
                    progress.applied, progress.failed, progress.intents_done
                ),
                Some(countdown),
            )
        }
    };

    let mut headline = format!(
        "{} known pending edits across {} intents",
        view.known_pending_total,
        view.items.len()
    );
    if view.unexpanded_intents > 0 {
        headline.push_str(&format!(
            " · {} not yet expanded (workload known at execution)",
            view.unexpanded_intents
        ));
    }

    rsx! {
        div { class: "page",
            h1 { "Work Queue" }
            div { class: "headline-count", "{headline}" }
            div { class: "card pass-panel",
                div { class: "row",
                    span { class: "pill {pass_pill_class}", "{status_line}" }
                }
                if let Some(detail) = &detail_line {
                    div { class: "muted", style: "margin-top: 6px;", "{detail}" }
                }
                div { class: "row", style: "margin-top: 10px;",
                    button {
                        class: "btn primary",
                        disabled: !is_idle,
                        onclick: move |_| {
                            let max_edits = budget_input.peek().trim().parse::<u32>().ok();
                            let _ = handle_exec
                                .try_send(ScrubberCommand::ExecuteOnce { max_edits });
                        },
                        "Execute"
                    }
                    input {
                        r#type: "number",
                        placeholder: "budget",
                        title: "max edits per execute (empty = unlimited)",
                        value: "{budget_input}",
                        oninput: move |event| budget_input.set(event.value()),
                    }
                    button {
                        class: "btn danger",
                        disabled: is_idle,
                        title: "interrupt the in-flight execute pass; unfinished intents stay in progress",
                        onclick: move |_| {
                            let _ = backend_stop.try_send(BackendCommand::StopExecution);
                        },
                        "Stop execution"
                    }
                }
            }
            if view.items.is_empty() {
                div { class: "card muted", "nothing queued for execution" }
            }
            for item in view.items {
                IntentCard { intent: item.intent, context: CardContext::WorkQueue }
            }
        }
    }
}
