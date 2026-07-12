//! Library: local-mirror listening stats — top artists, albums, and per-artist drill-down.

use crate::model::fmt_ts;
use crate::{CoreSignal, UiSignal};
use dioxus::prelude::*;
use scrobble_store::{AlbumCount, ArtistCount, Storage, TrackCount};
use std::ops::Range;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, PartialEq, Eq)]
enum RangeChoice {
    All,
    D30,
    D90,
    D365,
}

const RANGES: [(RangeChoice, &str); 4] = [
    (RangeChoice::All, "All time"),
    (RangeChoice::D30, "30d"),
    (RangeChoice::D90, "90d"),
    (RangeChoice::D365, "365d"),
];

impl RangeChoice {
    fn range(self) -> Option<Range<u64>> {
        let days: u64 = match self {
            RangeChoice::All => return None,
            RangeChoice::D30 => 30,
            RangeChoice::D90 => 90,
            RangeChoice::D365 => 365,
        };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Some(now.saturating_sub(days * 86_400)..now)
    }
}

/// Range-scoped library aggregates plus the always-global status figures.
#[derive(Clone, PartialEq, Default)]
struct LibraryData {
    total: u64,
    latest_uts: Option<u64>,
    sync_span: Option<(u64, u64)>,
    sync_total: u64,
    last_sync_at: Option<u64>,
    artists: Vec<ArtistCount>,
    albums: Vec<AlbumCount>,
}

/// Per-artist drill-down aggregates.
#[derive(Clone, PartialEq, Default)]
struct DrillData {
    tracks: Vec<TrackCount>,
    albums: Vec<AlbumCount>,
}

#[component]
pub fn Library() -> Element {
    let core = use_context::<CoreSignal>();
    let ui = use_context::<UiSignal>();

    let range_choice = use_signal(|| RangeChoice::All);
    let mut refresh = use_signal(|| 0_u64);
    let mut selected_artist = use_signal(|| None::<String>);
    let epoch = use_memo(move || ui.read().queue_epoch);

    let data = use_resource(move || async move {
        let _reload_on = (epoch(), refresh());
        let range = range_choice().range();
        let Some(Ok(core)) = core.read().clone() else {
            return LibraryData::default();
        };
        let total = core.store.scrobble_count(None).await.unwrap_or(0);
        let latest_uts = core.store.latest_uts().await.ok().flatten();
        let (sync_span, sync_total, last_sync_at) = match core.store.load_coverage().await {
            Ok(map) => {
                let last_sync_at = core
                    .store
                    .load_sync_state()
                    .await
                    .ok()
                    .and_then(|s| s.last_sync_at);
                (
                    map.first()
                        .zip(map.last())
                        .map(|(first, last)| (first.start, last.end)),
                    map.total_covered(),
                    last_sync_at,
                )
            }
            Err(_) => (None, 0, None),
        };
        let artists = core
            .store
            .top_artists(25, range.clone())
            .await
            .unwrap_or_default();
        let albums = core
            .store
            .top_albums(None, 25, range)
            .await
            .unwrap_or_default();
        LibraryData {
            total,
            latest_uts,
            sync_span,
            sync_total,
            last_sync_at,
            artists,
            albums,
        }
    });

    let drill = use_resource(move || async move {
        let _reload_on = (epoch(), refresh());
        let range = range_choice().range();
        let Some(artist) = selected_artist.read().clone() else {
            return DrillData::default();
        };
        let Some(Ok(core)) = core.read().clone() else {
            return DrillData::default();
        };
        let tracks = core
            .store
            .top_tracks(Some(&artist), 25, range.clone())
            .await
            .unwrap_or_default();
        let albums = core
            .store
            .top_albums(Some(&artist), 25, range)
            .await
            .unwrap_or_default();
        DrillData { tracks, albums }
    });

    let data_read = data.read();
    let Some(data) = &*data_read else {
        return rsx! {
            div { class: "page",
                h1 { "Library" }
                div { class: "card muted", "loading…" }
            }
        };
    };

    let latest = data
        .latest_uts
        .map(fmt_ts)
        .unwrap_or_else(|| "—".to_string());
    let sync_coverage = match data.sync_span {
        Some((start, end)) => {
            let from = fmt_ts(start);
            let to = fmt_ts(end);
            let days = data.sync_total / 86_400;
            format!("{from} → {to} ({days}d covered)")
        }
        None => "—".to_string(),
    };
    let last_sync = data
        .last_sync_at
        .map(fmt_ts)
        .unwrap_or_else(|| "—".to_string());
    let total = data.total;

    let artist_max = data.artists.first().map(|a| a.count).unwrap_or(1).max(1);
    let album_max = data.albums.first().map(|a| a.count).unwrap_or(1).max(1);

    rsx! {
        div { class: "page",
            h1 { "Library" }
            div { class: "grid",
                div { class: "stat",
                    div { class: "label", "total scrobbles" }
                    div { class: "value", "{total}" }
                }
                div { class: "stat",
                    div { class: "label", "latest scrobble" }
                    div { class: "value", "{latest}" }
                }
                div { class: "stat",
                    div { class: "label", "sync coverage (last.fm mirror)" }
                    div { class: "value", "{sync_coverage}" }
                }
                div { class: "stat",
                    div { class: "label", "last sync" }
                    div { class: "value", "{last_sync}" }
                }
            }
            div { class: "row", style: "margin-bottom: 14px; justify-content: space-between;",
                div { class: "tabs", style: "margin-bottom: 0;",
                    for (choice , label) in RANGES {
                        {
                            let mut range_choice = range_choice;
                            let class = if range_choice() == choice { "tab active" } else { "tab" };
                            rsx! {
                                button {
                                    class,
                                    onclick: move |_| range_choice.set(choice),
                                    "{label}"
                                }
                            }
                        }
                    }
                }
                button {
                    class: "btn",
                    onclick: move |_| refresh.set(refresh() + 1),
                    "Refresh"
                }
            }
            if total == 0 {
                div { class: "card muted", "no scrobbles yet — sync from the dashboard to populate your library" }
            } else {
                if let Some(artist) = selected_artist() {
                    {
                        let drill_read = drill.read();
                        let drill = drill_read.clone().unwrap_or_default();
                        let track_max = drill.tracks.first().map(|t| t.count).unwrap_or(1).max(1);
                        let album_max = drill.albums.first().map(|a| a.count).unwrap_or(1).max(1);
                        rsx! {
                            div { class: "card",
                                div { class: "row", style: "justify-content: space-between; margin-bottom: 10px;",
                                    div { class: "headline-count", "{artist}" }
                                    button {
                                        class: "btn",
                                        onclick: move |_| selected_artist.set(None),
                                        "← back"
                                    }
                                }
                                div { class: "lib-panels",
                                    div {
                                        div { class: "lib-panel-title", "Top Tracks" }
                                        if drill.tracks.is_empty() {
                                            div { class: "muted", "no tracks in range" }
                                        }
                                        div { class: "lib-list",
                                            for (i , track) in drill.tracks.iter().enumerate() {
                                                {
                                                    let pct = track.count * 100 / track_max;
                                                    let rank = i + 1;
                                                    rsx! {
                                                        div { class: "lib-row",
                                                            span { class: "lib-rank", "{rank}" }
                                                            span { class: "lib-name", "{track.track}" }
                                                            div { class: "lib-bar",
                                                                div { class: "lib-bar-fill", style: "width: {pct}%;" }
                                                            }
                                                            span { class: "lib-count", "{track.count}" }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    div {
                                        div { class: "lib-panel-title", "Top Albums" }
                                        if drill.albums.is_empty() {
                                            div { class: "muted", "no albums in range" }
                                        }
                                        div { class: "lib-list",
                                            for (i , album) in drill.albums.iter().enumerate() {
                                                {
                                                    let pct = album.count * 100 / album_max;
                                                    let rank = i + 1;
                                                    rsx! {
                                                        div { class: "lib-row",
                                                            span { class: "lib-rank", "{rank}" }
                                                            span { class: "lib-name", "{album.album}" }
                                                            div { class: "lib-bar",
                                                                div { class: "lib-bar-fill", style: "width: {pct}%;" }
                                                            }
                                                            span { class: "lib-count", "{album.count}" }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                div { class: "lib-panels",
                    div { class: "card",
                        div { class: "lib-panel-title", "Top Artists" }
                        if data.artists.is_empty() {
                            div { class: "muted", "no artists in range" }
                        }
                        div { class: "lib-list",
                            for (i , artist) in data.artists.iter().enumerate() {
                                {
                                    let pct = artist.count * 100 / artist_max;
                                    let rank = i + 1;
                                    let name = artist.artist.clone();
                                    rsx! {
                                        div {
                                            class: "lib-row clickable",
                                            onclick: move |_| selected_artist.set(Some(name.clone())),
                                            span { class: "lib-rank", "{rank}" }
                                            span { class: "lib-name", "{artist.artist}" }
                                            div { class: "lib-bar",
                                                div { class: "lib-bar-fill", style: "width: {pct}%;" }
                                            }
                                            span { class: "lib-count", "{artist.count}" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    div { class: "card",
                        div { class: "lib-panel-title", "Top Albums" }
                        if data.albums.is_empty() {
                            div { class: "muted", "no albums in range" }
                        }
                        div { class: "lib-list",
                            for (i , album) in data.albums.iter().enumerate() {
                                {
                                    let pct = album.count * 100 / album_max;
                                    let rank = i + 1;
                                    let name = album.artist.clone();
                                    rsx! {
                                        div {
                                            class: "lib-row clickable",
                                            onclick: move |_| selected_artist.set(Some(name.clone())),
                                            span { class: "lib-rank", "{rank}" }
                                            span { class: "lib-name", "{album.album} — {album.artist}" }
                                            div { class: "lib-bar",
                                                div { class: "lib-bar-fill", style: "width: {pct}%;" }
                                            }
                                            span { class: "lib-count", "{album.count}" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
