use crate::{background::run_off_ui_thread, CoreSignal, UiSignal};
use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use scrobble_store::insights::rediscover::{History, Options};

const PAGE_SIZE: usize = 50;

fn date(uts: u64) -> String {
    DateTime::from_timestamp(uts as i64, 0)
        .map(|dt| dt.date_naive().to_string())
        .unwrap_or_else(|| uts.to_string())
}

#[component]
pub fn Rediscover() -> Element {
    let core = use_context::<CoreSignal>();
    let ui = use_context::<UiSignal>();
    let epoch = use_memo(move || (ui.read().sync_epoch, ui.read().queue_epoch));
    let mut refresh = use_signal(|| 0_u64);
    let history = use_resource(move || async move {
        let _reload = (epoch(), refresh());
        let store = core
            .read()
            .as_ref()
            .and_then(|result| result.as_ref().ok())
            .map(|core| core.store.clone())
            .ok_or("Library unavailable")?;
        run_off_ui_thread(async move {
            History::load(store.as_ref(), Utc::now().timestamp() as u64)
                .await
                .map_err(|e| e.to_string())
        })
        .await
    });
    let mut options = use_signal(Options::default);
    let mut search = use_signal(String::new);
    let mut limit = use_signal(|| PAGE_SIZE);
    let data = history.read();
    let Some(result) = data.as_ref() else {
        return rsx! { div { class: "card muted", "Loading listening history…" } };
    };
    let history = match result {
        Ok(history) => history,
        Err(error) => {
            return rsx! { div { class: "card", role: "alert",
                p { "Could not load listening history. {error}" }
                button { class: "btn", onclick: move |_| refresh += 1, "Retry" }
            } }
        }
    };
    let opts = options();
    let gap = opts.gap_weight;
    let plays = 100 - gap;
    let mix = format!("{plays}% plays · {gap}% time away");
    let query = search().to_lowercase();
    let terms: Vec<_> = query.split_whitespace().collect();
    let mut rows = history.rank(opts);
    if !terms.is_empty() {
        rows.retain(|row| {
            let artist = row.stats.artist.to_lowercase();
            let track = row.stats.track.to_lowercase();
            terms
                .iter()
                .all(|term| artist.contains(term) || track.contains(term))
        });
    }
    let count = rows.len();
    let as_of = date(history.as_of);
    rsx! {
        section { class: "card rediscover",
            div { class: "row rediscover-heading",
                h2 { "Forgotten favorites" }
                button { class: "btn", onclick: move |_| refresh += 1, "Refresh" }
            }
            p { class: "muted", "Ranked by exact artist and track title across albums. Listening history through {as_of} (UTC)." }
            if !history.counts_complete { p { class: "muted", "Sync coverage is incomplete. Play counts may be low." } }
            div { class: "rediscover-controls",
                label { "Minimum plays"
                    select { value: "{opts.min_plays}",
                        onchange: move |e| if let Ok(n) = e.value().parse() { options.write().min_plays = n; limit.set(PAGE_SIZE); },
                        for n in [5, 10, 25, 50] { option { value: "{n}", "{n}" } }
                    }
                }
                label { "Time since last play"
                    select { value: "{opts.min_gap_days}",
                        onchange: move |e| if let Ok(n) = e.value().parse() { options.write().min_gap_days = n; limit.set(PAGE_SIZE); },
                        for y in [1, 3, 5, 10] { option { value: "{y * 365}", "{y}+ years" } }
                    }
                }
                label { "Find a favorite"
                    input { r#type: "search", placeholder: "Track or artist", value: "{search}",
                        oninput: move |e| { search.set(e.value()); limit.set(PAGE_SIZE); } }
                }
            }
            label { class: "rediscover-balance",
                span { "Ranking balance" }
                span { class: "muted", "{mix}" }
                input { r#type: "range", min: "0", max: "100", step: "1", value: "{gap}",
                    aria_valuetext: "{mix}",
                    oninput: move |e| if let Ok(n) = e.value().parse() { options.write().gap_weight = n; limit.set(PAGE_SIZE); } }
            }
            if history.is_empty() {
                p { "Sync your listening history to find forgotten favorites." }
            } else if count == 0 {
                p { "No matching tracks. Try a shorter gap or a lower minimum play count." }
            } else {
                p { class: "muted", "{count} matching tracks" }
                div { class: "rediscover-results",
                    table {
                        thead { tr { th { "Track / artist" } th { "Plays" } th { "Last played" } th { "Score" } } }
                        tbody {
                            for row in rows.iter().take(limit()) {
                                tr {
                                    td { strong { "{row.stats.track}" } small { "{row.stats.artist}" } }
                                    td { "{row.stats.count}" }
                                    td { "{date(row.stats.last_uts)}" small { "{row.years_away:.1} years ago" }
                                        if !row.gap_verified { small { "Gap unverified" } }
                                    }
                                    td { "{row.score:.1}" }
                                }
                            }
                        }
                    }
                }
                if count > limit() { button { class: "btn", onclick: move |_| limit += PAGE_SIZE, "Show more" } }
            }
        }
    }
}
