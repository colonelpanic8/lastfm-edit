//! Offline synthetic planning and edit-log benchmark.
//! Run: `cargo run --release -p scrobble-scrubber --example pipeline_bench -- [records]`.

use scrobble_scrubber::{FsScrubberState, Planner, RewriteRulesScrubActionProvider, ScrubFeed};
use scrobble_store::{
    CoverageMap, FsStorage, Provenanced, RecordSource, ScrobbleId, ScrobbleRecord, Segment, Storage,
};
use std::sync::Arc;
use std::time::Instant;

fn main() {
    let count: u64 = std::env::args()
        .nth(1)
        .map_or(100_000, |arg| arg.parse().expect("record count"));
    assert!(count > 0, "record count must be positive");
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(run(count));
}

async fn run(count: u64) {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(FsStorage::open(dir.path()).unwrap());
    let state = Arc::new(FsScrubberState::open(dir.path().join("scrubber")).unwrap());
    let start = 1_700_000_000;
    let records: Vec<_> = (0..count)
        .map(|index| {
            let uts = start + index * 180;
            let track = format!("Track {}", index % 1_000);
            ScrobbleRecord {
                id: ScrobbleId::new(uts, "Artist", &track),
                uts,
                artist: "Artist".into(),
                track,
                album: Some("Album".into()),
                album_artist: Provenanced::Verified("Artist".into()),
                source: RecordSource::Scrape,
                fetched_at: 1,
                deleted: false,
                v: 1,
            }
        })
        .collect();
    store.append_scrobbles(&records).await.unwrap();
    store
        .save_coverage(&CoverageMap::from_segments(vec![Segment::new(
            start,
            start + count * 180,
            1,
        )]))
        .await
        .unwrap();
    let planner = Planner::new(store.clone(), state)
        .with_provider(RewriteRulesScrubActionProvider::from_rules(vec![]));
    let feed = ScrubFeed::Incremental { window: None };
    let before = Instant::now();
    planner.plan(&feed).await.unwrap();
    println!(
        "{count} records, first plan: {:.3}s",
        before.elapsed().as_secs_f64()
    );
    for run in 1..=3 {
        let before = Instant::now();
        planner.plan(&feed).await.unwrap();
        println!(
            "unchanged plan {run}: {:.3}s",
            before.elapsed().as_secs_f64()
        );
    }
    for log_size in [10_000, 100_000] {
        let events: Vec<_> = (0..log_size)
            .map(|index| scrobble_store::EditLogEvent {
                edit_id: format!("operation-{log_size}-{index}"),
                at: 1,
                kind: scrobble_store::EditEventKind::Queued {
                    op: scrobble_store::EditOp::Delete {
                        artist: "Artist".into(),
                        track: "Track".into(),
                        uts: start + index,
                    },
                    target_ids: vec![ScrobbleId::new(start + index, "Artist", "Track")],
                },
            })
            .collect();
        store.append_edit_events(&events).await.unwrap();
        for run in 1..=3 {
            let before = Instant::now();
            let entries = store.load_edit_log().await.unwrap();
            println!(
                "load {} edits {run}: {:.3}s",
                entries.len(),
                before.elapsed().as_secs_f64()
            );
        }
        let before = Instant::now();
        store
            .append_edit_events(&[scrobble_store::EditLogEvent {
                edit_id: events[0].edit_id.clone(),
                at: 2,
                kind: scrobble_store::EditEventKind::Applied { result_ids: vec![] },
            }])
            .await
            .unwrap();
        println!(
            "append one edit event: {:.3}s",
            before.elapsed().as_secs_f64()
        );
    }
    let path = dir.path().join("scrubber/coverage/rewrite_rules.json");
    println!(
        "coverage snapshot: {} bytes",
        std::fs::metadata(path).unwrap().len()
    );
}
