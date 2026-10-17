//! Times the store and scrubber-state operations the app's views trigger, against a real
//! store directory. Usage: `cargo run --release --example perfbench -- <store-dir> [runs]`.

use scrobble_scrubber::{FsScrubberState, ScrubberState};
use scrobble_store::{FsStorage, Storage};
use std::future::Future;
use std::time::Instant;

struct Bench {
    runs: usize,
    rows: Vec<(String, f64, f64, f64, f64)>,
}

impl Bench {
    async fn time<T, F, Fut>(&mut self, name: &str, mut op: F)
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = T>,
    {
        let start = Instant::now();
        std::hint::black_box(op().await);
        let cold = start.elapsed().as_secs_f64() * 1e3;
        let mut samples: Vec<f64> = (0..self.runs).map(|_| 0.0).collect::<Vec<_>>();
        for sample in samples.iter_mut() {
            let start = Instant::now();
            std::hint::black_box(op().await);
            *sample = start.elapsed().as_secs_f64() * 1e3;
        }
        samples.sort_by(|a, b| a.total_cmp(b));
        let pick = |q: f64| samples[((samples.len() - 1) as f64 * q) as usize];
        self.rows
            .push((name.to_string(), cold, pick(0.0), pick(0.5), pick(0.9)));
    }

    fn print(&self) {
        println!(
            "{:<44} {:>9} {:>9} {:>9} {:>9}",
            "operation (ms)", "cold", "min", "median", "p90"
        );
        for (name, cold, min, median, p90) in &self.rows {
            println!("{name:<44} {cold:>9.2} {min:>9.2} {median:>9.2} {p90:>9.2}");
        }
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = std::path::PathBuf::from(args.next().expect("usage: perfbench <store-dir> [runs]"));
    let runs: usize = args.next().map_or(20, |r| r.parse().expect("runs"));
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(run(dir, runs));
}

async fn run(dir: std::path::PathBuf, runs: usize) {
    let store = FsStorage::open(&dir).unwrap();
    let state = FsScrubberState::open(dir.join("scrubber")).unwrap();
    let mut bench = Bench {
        runs,
        rows: Vec::new(),
    };

    let newest = store.latest_uts().await.unwrap().unwrap();
    let recent = store
        .recent_scrobbles(None, 1)
        .await
        .unwrap()
        .pop()
        .unwrap();
    let year = newest.saturating_sub(365 * 86_400)..newest + 1;

    bench.time("load_queue", || state.load_queue()).await;
    bench.time("load_edit_log", || store.load_edit_log()).await;
    bench.time("load_coverage", || store.load_coverage()).await;
    bench.time("latest_uts", || store.latest_uts()).await;
    bench
        .time("get_scrobble", || store.get_scrobble(&recent.id))
        .await;
    bench
        .time("scrobbles_in_range(last 24h)", || {
            store.scrobbles_in_range(newest - 86_400..newest + 1)
        })
        .await;
    bench
        .time("scrobbles_in_range(all)", || {
            store.scrobbles_in_range(0..u64::MAX)
        })
        .await;
    bench
        .time("scrobble_count(all)", || store.scrobble_count(None))
        .await;
    bench
        .time("top_artists(all)", || store.top_artists(50, None))
        .await;
    bench
        .time("top_albums(all)", || store.top_albums(None, 50, None))
        .await;
    bench
        .time("top_tracks(all)", || store.top_tracks(None, 50, None))
        .await;
    bench
        .time("top_artists(last year)", || {
            store.top_artists(50, Some(year.clone()))
        })
        .await;
    bench
        .time("artist_scrobbles(The Beatles)", || {
            store.artist_scrobbles("The Beatles", None)
        })
        .await;
    bench
        .time("search_recent(beatles)", || {
            store.search_recent_scrobbles("beatles", None, 50)
        })
        .await;
    bench
        .time("search_recent(no match)", || {
            store.search_recent_scrobbles("zzqqxx", None, 50)
        })
        .await;
    bench
        .time("recent_scrobbles(50)", || store.recent_scrobbles(None, 50))
        .await;
    bench
        .time("search_scrobbles(beatles)", || {
            store.search_scrobbles("beatles", 0, 50)
        })
        .await;
    bench
        .time("top_albums(last year)", || {
            store.top_albums(None, 50, Some(year.clone()))
        })
        .await;
    bench
        .time("top_tracks(The Beatles)", || {
            store.top_tracks(Some("The Beatles"), 25, None)
        })
        .await;
    bench
        .time("top_albums(The Beatles)", || {
            store.top_albums(Some("The Beatles"), 25, None)
        })
        .await;
    bench
        .time("scrobble_count(empty range)", || {
            store.scrobble_count(Some(0..1))
        })
        .await;
    let queue = state.load_queue().await.unwrap();
    bench
        .time("clone queue (WorkQueue render)", || {
            let queue = &queue;
            async move { queue.clone() }
        })
        .await;
    bench
        .time("dashboard reload", || async {
            (
                store.latest_uts().await.unwrap(),
                store.load_coverage().await.unwrap(),
                state.load_queue().await.unwrap(),
            )
        })
        .await;
    bench
        .time("library reload (all time)", || async {
            (
                store.scrobble_count(None).await.unwrap(),
                store.top_artists(50, None).await.unwrap(),
                store.top_albums(None, 50, None).await.unwrap(),
                store.latest_uts().await.unwrap(),
                store.load_coverage().await.unwrap(),
                store.load_sync_state().await.unwrap(),
            )
        })
        .await;
    println!("queue: {} intents; runs per op: {runs}", queue.len());
    bench.print();
}
