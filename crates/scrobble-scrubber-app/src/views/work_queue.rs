//! Work queue: what is truly queued for execution (in executor order) plus the live
//! pass panel with execute/stop controls.

use crate::components::{ActivityLog, CardContext, ExecutionNotice, IntentCard};
use crate::core::BackendCommand;
use crate::model::{edit_step_label, ended_label, fmt_ts, parse_edit_limit, PassState};
use crate::views::use_queue;
use crate::{CoreSignal, UiSignal};
use dioxus::prelude::*;
use scrobble_scrubber::{work_queue_view, EditStep, ExecEnded, ExecFailureKind, ScrubberCommand};

#[component]
pub fn WorkQueue() -> Element {
    let core = use_context::<CoreSignal>();
    let mut ui = use_context::<UiSignal>();
    let queue = use_queue();
    let mut limit_input = use_signal(String::new);
    let mut limit_error = use_signal(|| None::<String>);

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
    let is_active = ui_read.pass.is_active();
    let stopping = ui_read.execution_stop_requested;
    let retrying = matches!(&ui_read.pass, PassState::Failed { .. });
    let has_work = !view.items.is_empty();
    let elapsed = ui_read
        .execution_started_at
        .map(|started| (now() - started).max(0));
    let since_update = ui_read
        .execution_updated_at
        .map(|updated| (now() - updated).max(0));

    // Precompute pass-panel text (RSX format segments can't contain method calls).
    let (pass_pill_class, status_line, detail_line) = match &ui_read.pass {
        PassState::Idle => {
            if let Some(report) = &ui_read.last_exec {
                let ended = ended_label(&report.ended);
                let status = format!(
                    "Last run: {ended} — {} applied, {} failed",
                    report.instances_applied, report.instances_failed
                );
                let detail = match report.ended {
                    ExecEnded::Busy => "The background service or another window has the queue. This window cannot show that process’s live progress.",
                    ExecEnded::BudgetExhausted => "Your edit limit was reached. Remaining edits stay queued; apply again to continue.",
                    ExecEnded::Deferred => "Last.fm is still rate limiting requests. Remaining edits stay queued for a later run.",
                    ExecEnded::Cancelled => "Stopped. Completed edits are saved and unfinished edits stay queued.",
                    ExecEnded::Completed => "This run has finished. Check any reported failures below.",
                };
                (
                    if report.instances_failed > 0 || report.ended == ExecEnded::Busy {
                        "warn"
                    } else {
                        "ok"
                    },
                    status,
                    Some(detail.into()),
                )
            } else {
                ("", "Ready to apply queued edits".to_string(), None)
            }
        }
        PassState::Running(progress) => {
            let detail = match (&progress.current, &progress.current_edit) {
                (Some(current), Some(edit)) => {
                    let step = edit_step_label(&edit.step, now());
                    let played = fmt_ts(edit.instance.uts());
                    Some(format!("{step}: {} · played {played}", current.subject))
                }
                (Some(current), None) => Some(format!("Finding scrobbles: {}", current.subject)),
                (None, _) => Some("Loading queued edits…".into()),
            };
            (
                "accent",
                format!(
                    "Applying edits — {} applied, {} failed, {} track groups finished",
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
                    "Paused — {} applied, {} failed, {} track groups finished",
                    progress.applied, progress.failed, progress.intents_done
                ),
                Some(countdown),
            )
        }
        PassState::Failed { progress, .. } => (
            "danger",
            format!(
                "Execution stopped — {} applied, {} failed",
                progress.applied, progress.failed
            ),
            None,
        ),
    };

    let mut headline = format!("{} track groups queued", view.items.len());
    if view.known_pending_total > 0 {
        headline.push_str(&format!(
            " · {} scrobble edits counted",
            view.known_pending_total
        ));
    }
    if view.unexpanded_intents > 0 {
        headline.push_str(&format!(
            " · {} groups will be counted when they start",
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
                if is_active {
                    if let (Some(elapsed), Some(since_update)) = (elapsed, since_update) {
                        div { class: "muted", style: "margin-top: 6px;",
                            "Running for {elapsed}s · last activity {since_update}s ago"
                        }
                    }
                    if stopping {
                        div { class: "banner warn execution-notice", role: "status",
                            "Stopping after the current request. Completed edits are saved; unfinished edits stay queued."
                        }
                    } else if since_update.is_some_and(|seconds| seconds >= 30)
                        && matches!(&ui_read.pass, PassState::Running(progress)
                            if !matches!(&progress.current_edit, Some(edit) if matches!(edit.step, EditStep::Pacing { .. }))) {
                        div { class: "banner warn execution-notice", role: "status",
                            "Last.fm has not returned an update recently. The request may still be waiting. You can stop this run after the current request finishes."
                        }
                    }
                }
                ExecutionNotice {}
                div { class: "row", style: "margin-top: 10px;",
                    button {
                        class: "btn primary",
                        disabled: is_active || !has_work,
                        onclick: move |_| {
                            let max_edits = match parse_edit_limit(&limit_input.peek()) {
                                Ok(limit) => limit,
                                Err(error) => {
                                    limit_error.set(Some(error.into()));
                                    return;
                                }
                            };
                            limit_error.set(None);
                            ui.with_mut(|state| {
                                state.pass = PassState::Running(Default::default());
                                state.execution_stop_requested = false;
                                state.last_edit_failure = None;
                                let now = chrono::Utc::now().timestamp();
                                state.execution_started_at = Some(now);
                                state.execution_updated_at = Some(now);
                            });
                            let handle = handle_exec.clone();
                            spawn(async move {
                                if handle
                                    .send(ScrubberCommand::ExecuteOnce { max_edits })
                                    .await
                                    .is_err()
                                {
                                    ui.with_mut(|state| {
                                        state.fail_execution(
                                            "The scrubber backend is not available. Reopen the app to reconnect.".into(),
                                            ExecFailureKind::Other,
                                        );
                                    });
                                }
                            });
                        },
                        if retrying { "Retry queued edits" } else { "Apply queued edits" }
                    }
                    label {
                        r#for: "edit-limit",
                        "Edit limit (optional)"
                    }
                    input {
                        id: "edit-limit",
                        r#type: "number",
                        min: "1",
                        step: "1",
                        placeholder: "No limit",
                        disabled: is_active,
                        aria_describedby: "edit-limit-help",
                        value: "{limit_input}",
                        oninput: move |event| {
                            limit_input.set(event.value());
                            limit_error.set(None);
                        },
                    }
                    button {
                        class: "btn danger",
                        disabled: !is_active || stopping,
                        title: "Stop this run; unfinished edits stay queued",
                        onclick: move |_| {
                            ui.with_mut(|state| state.execution_stop_requested = true);
                            let backend = backend_stop.clone();
                            spawn(async move {
                                if backend.send(BackendCommand::StopExecution).await.is_err() {
                                    ui.with_mut(|state| {
                                        state.execution_stop_requested = false;
                                        state.error = Some("The stop request could not reach the backend. Reopen the app to reconnect.".into());
                                    });
                                }
                            });
                        },
                        if stopping { "Stopping…" } else { "Stop execution" }
                    }
                }
                div { id: "edit-limit-help", class: "muted", style: "margin-top: 8px;",
                    "Maximum individual scrobble edit attempts for this run. Leave blank to work through the queue. Requests are paced to respect Last.fm’s limits."
                }
                if let Some(error) = limit_error.read().as_ref() {
                    div { role: "alert", "{error}" }
                }
            }
            ActivityLog {}
            if !has_work {
                div { class: "card muted", "nothing queued for execution" }
            }
            for item in view.items {
                IntentCard {
                    key: "{item.intent.id}",
                    intent: item.intent,
                    context: CardContext::WorkQueue,
                }
            }
        }
    }
}
