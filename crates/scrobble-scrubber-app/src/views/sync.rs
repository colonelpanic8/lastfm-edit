//! Sync: synchronization state, a coverage timeline, and a recent-scrobbles feed drawn
//! entirely from the local mirror. This view never talks to Last.fm — the "Sync now"
//! button hands off to the backend sync engine, which is the only thing that does.

use crate::model::{fmt_ts, SyncStatus};
use crate::{CoreSignal, UiSignal};
use chrono::{DateTime, Local};
use dioxus::prelude::*;
use scrobble_store::{CoverageMap, ScrobbleGroup, Storage};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// One page of grouped scrobble-history search results.
const PAGE_SIZE: usize = 50;

/// Status/coverage figures, reloaded when a sync completes (or on manual refresh).
#[derive(Clone, PartialEq, Default)]
struct SyncData {
    total: u64,
    coverage: CoverageMap,
    history_start_uts: Option<u64>,
    last_sync_at: Option<u64>,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Date-only label for the coverage timeline ends.
fn fmt_date(ts: u64) -> String {
    DateTime::from_timestamp(ts as i64, 0)
        .map(|dt| dt.with_timezone(&Local).format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| ts.to_string())
}

#[component]
pub fn Sync() -> Element {
    let core = use_context::<CoreSignal>();
    let mut ui = use_context::<UiSignal>();

    // 1s ticker so the rate-limit countdown stays live (mirrors the dashboard).
    let mut now = use_signal(|| chrono::Utc::now().timestamp());
    use_future(move || async move {
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            now.set(chrono::Utc::now().timestamp());
        }
    });

    // The feed resets whenever a sync makes progress; `sync_epoch` is the memo dependency.
    let sync_epoch = use_memo(move || ui.read().sync_epoch);
    let mut refresh = use_signal(|| 0_u64);

    let data = use_resource(move || async move {
        let _reload_on = (sync_epoch(), refresh());
        let Some(Ok(core)) = core.read().clone() else {
            return SyncData::default();
        };
        let store = core.store.clone();
        crate::background::run_off_ui_thread(async move {
            let total = store.scrobble_count(None).await.unwrap_or(0);
            let coverage = store.load_coverage().await.unwrap_or_default();
            let sync_state = store.load_sync_state().await.unwrap_or_default();
            SyncData {
                total,
                coverage,
                history_start_uts: sync_state.history_start_uts,
                last_sync_at: sync_state.last_sync_at,
            }
        })
        .await
    });

    // Debounced SQLite-backed search, accumulated across "Load more" clicks.
    let mut query = use_signal(String::new);
    let mut debounced_query = use_signal(String::new);
    let _debounce = use_resource(move || async move {
        let value = query();
        tokio::time::sleep(Duration::from_millis(200)).await;
        debounced_query.set(value);
    });

    let mut groups = use_signal(Vec::<ScrobbleGroup>::new);
    let mut exhausted = use_signal(|| false);
    let mut loading = use_signal(|| false);
    let mut history_error = use_signal(|| None::<String>);
    let mut reindexing = use_signal(|| false);
    let mut reindex_error = use_signal(|| None::<String>);

    // (Re)load the first page when the query changes, sync makes progress, or the user refreshes.
    let _first_page = use_resource(move || async move {
        let _reload_on = (sync_epoch(), refresh());
        let search = debounced_query();
        let Some(Ok(core)) = core.read().clone() else {
            return;
        };
        let store = core.store.clone();
        loading.set(true);
        history_error.set(None);
        let result = crate::background::run_off_ui_thread(async move {
            store.search_scrobbles(&search, 0, PAGE_SIZE).await
        })
        .await;
        match result {
            Ok(page) => {
                let full = page.len() == PAGE_SIZE;
                groups.set(page);
                exhausted.set(!full);
            }
            Err(error) => {
                groups.set(Vec::new());
                exhausted.set(true);
                history_error.set(Some(error.to_string()));
            }
        }
        loading.set(false);
    });

    let load_more = move |_| {
        if loading() || exhausted() {
            return;
        }
        loading.set(true);
        let search = debounced_query.peek().clone();
        let offset = groups.peek().len();
        spawn(async move {
            let Some(Ok(core)) = core.read().clone() else {
                loading.set(false);
                return;
            };
            let store = core.store.clone();
            let task_search = search.clone();
            let result = crate::background::run_off_ui_thread(async move {
                store
                    .search_scrobbles(&task_search, offset, PAGE_SIZE)
                    .await
            })
            .await;
            match result {
                Ok(page) => {
                    if query.peek().as_str() != search || debounced_query.peek().as_str() != search
                    {
                        loading.set(false);
                        return;
                    }
                    if page.len() < PAGE_SIZE {
                        exhausted.set(true);
                    }
                    groups.with_mut(|current| current.extend(page));
                }
                Err(error) => {
                    history_error.set(Some(error.to_string()));
                }
            }
            loading.set(false);
        });
    };

    let Some(Ok(core)) = core.read().clone() else {
        return rsx! {
            div { class: "page",
                h1 { "Sync" }
                div { class: "card muted", "loading…" }
            }
        };
    };
    let backend_sync = core.backend.clone();
    let backend_reindex = core.backend.clone();
    let sync_available = core.sync_available;

    let ui_read = ui.read();
    let (sync_pill_class, sync_pill) = match &ui_read.sync {
        SyncStatus::Unavailable => ("", "sync unavailable".to_string()),
        SyncStatus::Idle => ("ok", "sync idle".to_string()),
        SyncStatus::Syncing => ("accent", "syncing".to_string()),
        SyncStatus::RateLimited { until } => (
            "warn",
            match until {
                Some(until) => {
                    let left = (*until as i64 - now()).max(0);
                    format!("rate limited — {left}s left")
                }
                None => "rate limited".to_string(),
            },
        ),
    };
    let is_syncing = ui_read.sync == SyncStatus::Syncing;

    let data_read = data.read();
    let data_loading = *data.state().read() == UseResourceState::Pending;
    let data = data_read.clone().unwrap_or_default();
    let last_sync = data
        .last_sync_at
        .map(fmt_ts)
        .unwrap_or_else(|| "—".to_string());
    let history_start = data
        .history_start_uts
        .map(fmt_ts)
        .unwrap_or_else(|| "—".to_string());
    let total = data.total;

    // Coverage timeline bounds: [start, end] where start is the discovered history origin
    // (or the earliest covered instant) and end is now (or the frontier, whichever is later).
    let now_ts = now_secs();
    let cov_start = data
        .history_start_uts
        .or_else(|| data.coverage.first().map(|seg| seg.start));
    let cov_end = data
        .coverage
        .last()
        .map(|seg| seg.end.max(now_ts))
        .unwrap_or(now_ts);

    let history_groups = groups.read();
    let search_pending = query() != debounced_query();

    rsx! {
        div { class: "page",
            h1 { "Sync" }
            div { class: "row", style: "margin-bottom: 14px;",
                span { class: "pill {sync_pill_class}", "{sync_pill}" }
                span { class: "muted mono", "{core.store_root.display()}" }
            }
            div { class: "grid",
                div { class: "stat",
                    div { class: "label", "total scrobbles" }
                    div { class: "value", "{total}" }
                }
                div { class: "stat",
                    div { class: "label", "last sync" }
                    div { class: "value", "{last_sync}" }
                }
                div { class: "stat",
                    div { class: "label", "history start" }
                    div { class: "value", "{history_start}" }
                }
            }
            div { class: "card",
                div { class: "row",
                    button {
                        class: "btn primary",
                        disabled: !sync_available || is_syncing,
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
                        if is_syncing { "Syncing…" } else { "Sync now" }
                    }
                    button {
                        class: "btn",
                        title: "drop and rebuild the local SQLite index from the flat files",
                        disabled: reindexing(),
                        onclick: move |_| {
                            if reindexing() {
                                return;
                            }
                            reindexing.set(true);
                            reindex_error.set(None);
                            let backend = backend_reindex.clone();
                            let (completed, receiver) = tokio::sync::oneshot::channel();
                            spawn(async move {
                                let result = match backend
                                    .send(crate::core::BackendCommand::Reindex { completed })
                                    .await
                                {
                                    Ok(()) => receiver.await.unwrap_or_else(|_| {
                                        Err("the backend stopped unexpectedly".into())
                                    }),
                                    Err(_) => Err("the backend is not available".into()),
                                };
                                reindexing.set(false);
                                match result {
                                    Ok(()) => refresh.set(refresh() + 1),
                                    Err(error) => reindex_error.set(Some(error)),
                                }
                            });
                        },
                        if reindexing() { "Rebuilding…" } else { "Rebuild index" }
                    }
                    button {
                        class: "btn",
                        disabled: data_loading,
                        onclick: move |_| refresh.set(refresh() + 1),
                        if data_loading { "Refreshing…" } else { "Refresh" }
                    }
                }
                if let Some(error) = reindex_error() {
                    div { class: "banner danger", role: "alert", "{error}" }
                }
            }

            div { class: "card",
                div { class: "lib-panel-title", "Coverage" }
                if data.coverage.is_empty() {
                    div { class: "muted", "no coverage yet — run a sync to mirror your history" }
                } else {
                    {
                        let start = cov_start.unwrap_or(0);
                        let span = cov_end.saturating_sub(start).max(1);
                        rsx! {
                            div { class: "cov-track",
                                for seg in data.coverage.segments() {
                                    {
                                        let left = seg.start.saturating_sub(start) * 100 / span;
                                        let width = (seg.len() * 100 / span).max(1);
                                        rsx! {
                                            div {
                                                class: "cov-seg",
                                                style: "left: {left}%; width: {width}%;",
                                                title: "{fmt_ts(seg.start)} → {fmt_ts(seg.end)}",
                                            }
                                        }
                                    }
                                }
                            }
                            div { class: "cov-labels",
                                span { "{fmt_date(start)}" }
                                span { "{fmt_date(cov_end)}" }
                            }
                        }
                    }
                }
            }

            div { class: "card history-card",
                div { class: "history-heading",
                    div {
                        div { class: "lib-panel-title", "Scrobble history" }
                        div { class: "muted history-help", "Identical artist, track, and album metadata is grouped across timestamps." }
                    }
                    input {
                        class: "history-search",
                        r#type: "text",
                        value: "{query}",
                        placeholder: "Search artist, track, or album…",
                        oninput: move |event| {
                            query.set(event.value());
                            groups.set(Vec::new());
                            exhausted.set(false);
                            history_error.set(None);
                        },
                    }
                }
                if let Some(error) = history_error() {
                    div { class: "banner danger", "history search failed: {error}" }
                } else if history_groups.is_empty() && (loading() || search_pending) {
                    div { class: "muted", "Searching…" }
                } else if history_groups.is_empty() {
                    div { class: "muted",
                        if debounced_query().is_empty() {
                            "no scrobbles yet — sync to populate the mirror"
                        } else {
                            "no matching scrobbles"
                        }
                    }
                } else {
                    div { class: "feed-list",
                        for group in history_groups.iter() {
                            div { class: "feed-row",
                                span { class: "feed-time mono",
                                    if group.count == 1 {
                                        "{fmt_ts(group.latest_uts)}"
                                    } else {
                                        "{fmt_ts(group.first_uts)} → {fmt_ts(group.latest_uts)}"
                                    }
                                }
                                span { class: "feed-main", "{group.artist} — {group.track}" }
                                if let Some(album) = &group.album {
                                    if !album.is_empty() {
                                        span { class: "feed-album muted", "{album}" }
                                    }
                                }
                                span { class: "pill feed-count", "{group.count}×" }
                            }
                        }
                    }
                    if !exhausted() {
                        div { class: "row", style: "margin-top: 12px;",
                            button {
                                class: "btn",
                                disabled: loading(),
                                onclick: load_more,
                                if loading() { "Loading…" } else { "Load more" }
                            }
                        }
                    }
                }
            }
        }
    }
}
