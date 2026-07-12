//! Dashboard: status, stats, controls, activity log.

use crate::components::ActivityLog;
use crate::model::{fmt_ts, CycleInfo, CyclePhase, PassState, PlanStatus, SyncStatus};
use crate::{CoreSignal, UiSignal};
use dioxus::prelude::*;
use scrobble_scrubber::{
    review_status, work_status, ReviewStatus, ScrubFeed, ScrubberCommand, ScrubberState, WorkStatus,
};
use scrobble_store::Storage;

/// Store-derived numbers the stat cards show; reloaded when the queue epoch bumps or a
/// sync finishes.
#[derive(Clone, Debug, Default, PartialEq)]
struct Stats {
    latest_uts: Option<u64>,
    /// What span of history the local store has mirrored from last.fm.
    sync_span: Option<(u64, u64)>,
    sync_total: u64,
    /// What span of history the rules provider has already analyzed.
    rule_span: Option<(u64, u64)>,
    rule_total: u64,
    /// The rule set changed since rule coverage was computed; the planner will rescan.
    rule_coverage_stale: bool,
    needs_review: usize,
    queued: usize,
    partial: usize,
    done: usize,
}

#[component]
pub fn Dashboard() -> Element {
    let core = use_context::<CoreSignal>();
    let mut ui = use_context::<UiSignal>();

    // 1s ticker so the rate-limit countdown stays live.
    let mut now = use_signal(|| chrono::Utc::now().timestamp());
    use_future(move || async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            now.set(chrono::Utc::now().timestamp());
        }
    });

    let epoch = use_memo(move || ui.read().queue_epoch);
    let stats = use_resource(move || async move {
        let _reload_on = epoch();
        let Some(Ok(core)) = core.read().clone() else {
            return Stats::default();
        };
        let store = core.store.clone();
        let state = core.state.clone();
        let rules_hash = core.rules_hash.clone();
        crate::background::run_off_ui_thread(async move {
            let latest_uts = store.latest_uts().await.ok().flatten();
            let (sync_span, sync_total) = match store.load_coverage().await {
                Ok(map) => (
                    map.first()
                        .zip(map.last())
                        .map(|(first, last)| (first.start, last.end)),
                    map.total_covered(),
                ),
                Err(_) => (None, 0),
            };
            let (rule_span, rule_total, rule_coverage_stale) =
                match state.load_provider_coverage("rewrite_rules").await {
                    Ok(provider) => {
                        // Never-planned stores have no hash; only a *different* hash is stale.
                        let stale =
                            provider.rules_hash.is_some() && provider.rules_hash != rules_hash;
                        (
                            provider
                                .coverage
                                .first()
                                .zip(provider.coverage.last())
                                .map(|(first, last)| (first.start, last.end)),
                            provider.coverage.total_covered(),
                            stale,
                        )
                    }
                    Err(_) => (None, 0, false),
                };
            let mut stats = Stats {
                latest_uts,
                sync_span,
                sync_total,
                rule_span,
                rule_total,
                rule_coverage_stale,
                ..Stats::default()
            };
            if let Ok(queue) = state.load_queue().await {
                for intent in &queue {
                    let review = review_status(intent);
                    if review == ReviewStatus::NeedsReview {
                        stats.needs_review += 1;
                    }
                    match work_status(intent) {
                        // "queued" means accepted and waiting; unreviewed intents are
                        // counted on the review axis instead.
                        Some(WorkStatus::Queued) if review == ReviewStatus::Accepted => {
                            stats.queued += 1;
                        }
                        Some(WorkStatus::Partial { .. }) => stats.partial += 1,
                        Some(WorkStatus::Done) => stats.done += 1,
                        _ => {}
                    }
                }
            }
            stats
        })
        .await
    });

    // Continuous-mode controls (mirrored to the backend loop on toggle).
    let mut continuous = use_signal(|| false);
    let mut interval_input = use_signal(|| "300".to_string());
    let mut enabling_rules = use_signal(|| false);

    let Some(Ok(core)) = core.read().clone() else {
        return rsx! {};
    };
    let handle = core.handle.clone();
    let backend_sync = core.backend.clone();
    let backend_continuous = core.backend.clone();
    let backend_rules = core.backend.clone();
    let sync_available = core.sync_available;

    let ui_read = ui.read();
    let plan_label = match &ui_read.plan {
        PlanStatus::Idle => "Plan incremental".to_string(),
        PlanStatus::Planning { subjects } => format!("Planning… ({subjects})"),
    };
    let (exec_pill_class, exec_pill) = match &ui_read.pass {
        PassState::Idle => ("", "executor idle".to_string()),
        PassState::Running(progress) => (
            "accent",
            format!(
                "executing — {} applied{}",
                progress.applied,
                if progress.failed > 0 {
                    format!(", {} failed", progress.failed)
                } else {
                    String::new()
                }
            ),
        ),
        PassState::Paused { progress, until } => (
            "danger",
            match until {
                Some(until) => {
                    let left = (*until as i64 - now()).max(0);
                    format!("rate limited — {left}s left ({} applied)", progress.applied)
                }
                None => format!("rate limited ({} applied)", progress.applied),
            },
        ),
    };
    let (plan_pill_class, plan_pill) = match &ui_read.plan {
        PlanStatus::Idle => ("", "planner idle"),
        PlanStatus::Planning { .. } => ("accent", "planning"),
    };
    let (sync_pill_class, sync_pill) = match &ui_read.sync {
        SyncStatus::Unavailable => ("", "sync unavailable".to_string()),
        SyncStatus::Idle => ("", "sync idle".to_string()),
        SyncStatus::Syncing => ("accent", "syncing".to_string()),
        SyncStatus::RateLimited { until } => (
            "warn",
            match until {
                Some(until) => {
                    let left = (*until as i64 - now()).max(0);
                    format!("sync rate limited — {left}s left")
                }
                None => "sync rate limited".to_string(),
            },
        ),
    };

    let stats_read = stats.read();
    let stats_now = stats_read.clone().unwrap_or_default();
    let stats_loading = *stats.state().read() == UseResourceState::Pending;
    let latest = stats_now
        .latest_uts
        .map(fmt_ts)
        .unwrap_or_else(|| "—".to_string());
    let span_text = |span: Option<(u64, u64)>, total: u64| match span {
        Some((start, end)) => {
            let from = fmt_ts(start);
            let to = fmt_ts(end);
            let days = total / 86_400;
            format!("{from} → {to} ({days}d covered)")
        }
        None => "—".to_string(),
    };
    let sync_coverage = span_text(stats_now.sync_span, stats_now.sync_total);
    let rule_coverage = if stats_now.rule_coverage_stale {
        "stale — rules changed; next plan rescans".to_string()
    } else {
        span_text(stats_now.rule_span, stats_now.rule_total)
    };
    let queue_summary = format!(
        "{} need review · {} queued · {} in flight · {} done",
        stats_now.needs_review, stats_now.queued, stats_now.partial, stats_now.done
    );

    rsx! {
        div { class: "page",
            h1 { "Dashboard" }
            div { class: "row", style: "margin-bottom: 14px;",
                span { class: "muted", "{core.username}" }
                span { class: "muted mono", "{core.store_root.display()}" }
            }
            div { class: "row", style: "margin-bottom: 14px;",
                span { class: "pill {exec_pill_class}", "{exec_pill}" }
                span { class: "pill {plan_pill_class}", "{plan_pill}" }
                span { class: "pill {sync_pill_class}", "{sync_pill}" }
                if stats_loading {
                    span { class: "pill accent", "Refreshing dashboard…" }
                }
            }
            if core.rules_empty {
                div { class: "card setup-card",
                    div { class: "setup-copy",
                        h2 { "Set up cleanup rules" }
                        p { class: "muted",
                            "Start with the built-in rule set for cleaning remasters, special editions, and common metadata noise."
                        }
                    }
                    button {
                        class: "btn primary",
                        disabled: enabling_rules(),
                        onclick: move |_| {
                            if enabling_rules() {
                                return;
                            }
                            enabling_rules.set(true);
                            let backend = backend_rules.clone();
                            spawn(async move {
                                if backend
                                    .send(crate::core::BackendCommand::EnableDefaultRules)
                                    .await
                                    .is_err()
                                {
                                    enabling_rules.set(false);
                                    ui.with_mut(|state| {
                                        state.error = Some("the backend is not available".into());
                                    });
                                }
                            });
                        },
                        if enabling_rules() { "Enabling…" } else { "Enable default rules" }
                    }
                }
            }
            if let PassState::Paused { until, .. } = &ui_read.pass {
                {
                    let detail = match until {
                        Some(until) => {
                            let left = (*until as i64 - now()).max(0);
                            format!("last.fm is rate limiting edits; resuming in ~{left}s")
                        }
                        None => "last.fm is rate limiting edits; waiting it out".to_string(),
                    };
                    rsx! {
                        div { class: "banner warn", "{detail}" }
                    }
                }
            }
            if let Some(error) = &ui_read.error {
                div { class: "banner danger", "{error}" }
            }
            div { class: "grid",
                div { class: "stat",
                    div { class: "label", "latest scrobble" }
                    div { class: "value", "{latest}" }
                }
                div { class: "stat",
                    div { class: "label", "sync coverage (last.fm mirror)" }
                    div { class: "value", "{sync_coverage}" }
                }
                div { class: "stat",
                    div { class: "label", "rule coverage (analyzed)" }
                    div { class: "value", "{rule_coverage}" }
                }
                div { class: "stat",
                    div { class: "label", "queue" }
                    div { class: "value", "{queue_summary}" }
                }
            }
            div { class: "card",
                div { class: "row",
                    button {
                        class: "btn",
                        disabled: !sync_available || ui_read.sync == SyncStatus::Syncing,
                        title: if sync_available { "" } else { "set LASTFM_EDIT_API_KEY to enable sync" },
                        onclick: move |_| {
                            ui.with_mut(|state| state.sync = SyncStatus::Syncing);
                            let backend = backend_sync.clone();
                            spawn(async move {
                                if backend
                                    .send(crate::core::BackendCommand::SyncNow)
                                    .await
                                    .is_err()
                                {
                                    ui.with_mut(|state| {
                                        state.sync = SyncStatus::Idle;
                                        state.error = Some("the backend is not available".into());
                                    });
                                }
                            });
                        },
                        if ui_read.sync == SyncStatus::Syncing { "Syncing…" } else { "Sync now" }
                    }
                    button {
                        class: "btn primary",
                        disabled: ui_read.plan != PlanStatus::Idle,
                        onclick: move |_| {
                            ui.with_mut(|state| {
                                state.plan = PlanStatus::Planning { subjects: 0 };
                            });
                            let handle = handle.clone();
                            spawn(async move {
                                if handle
                                    .send(ScrubberCommand::PlanFeed(ScrubFeed::Incremental {
                                        window: None,
                                    }))
                                    .await
                                    .is_err()
                                {
                                    ui.with_mut(|state| {
                                        state.plan = PlanStatus::Idle;
                                        state.error = Some("the scrubber actor is not available".into());
                                    });
                                }
                            });
                        },
                        "{plan_label}"
                    }
                }
                div { class: "row", style: "margin-top: 10px;",
                    button {
                        class: if continuous() { "btn danger" } else { "btn" },
                        onclick: move |_| {
                            let enabled = !continuous();
                            let interval_secs = interval_input
                                .peek()
                                .trim()
                                .parse::<u64>()
                                .unwrap_or(300);
                            let backend = backend_continuous.clone();
                            spawn(async move {
                                if backend
                                    .send(crate::core::BackendCommand::SetContinuous {
                                        enabled,
                                        interval_secs,
                                    })
                                    .await
                                    .is_err()
                                {
                                    continuous.set(!enabled);
                                    ui.with_mut(|state| {
                                        state.error = Some("the backend is not available".into());
                                    });
                                }
                            });
                            continuous.set(enabled);
                        },
                        if continuous() { "Stop continuous" } else { "Start continuous" }
                    }
                    input {
                        r#type: "number",
                        title: "continuous interval (seconds)",
                        value: "{interval_input}",
                        disabled: continuous(),
                        oninput: move |event| interval_input.set(event.value()),
                    }
                    if continuous() {
                        {
                            let label = match &ui_read.continuous {
                                Some(CycleInfo { n, phase: CyclePhase::Running }) => {
                                    format!("continuous — cycle {n} running")
                                }
                                Some(CycleInfo { n, phase: CyclePhase::Sleeping { seconds } }) => {
                                    format!("continuous — cycle {n} enqueued, idle {seconds}s")
                                }
                                None => "continuous — sync → plan → execute".to_string(),
                            };
                            rsx! {
                                span { class: "pill accent", "{label}" }
                            }
                        }
                    }
                }
            }
            ActivityLog {}
        }
    }
}
