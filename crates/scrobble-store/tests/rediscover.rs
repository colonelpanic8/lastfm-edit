use scrobble_store::insights::rediscover::{History, Options, YEAR_SECONDS};
use scrobble_store::{
    CoverageMap, FsStorage, MemoryStorage, Provenanced, RecordSource, ScrobbleId, ScrobbleRecord,
    Segment, Storage, SyncState,
};

fn play(track: &str, uts: u64) -> ScrobbleRecord {
    ScrobbleRecord {
        id: ScrobbleId::new(uts, "Artist", track),
        uts,
        artist: "Artist".into(),
        track: track.into(),
        album: Some(format!("Album {}", uts % 2)),
        album_artist: Provenanced::Unknown,
        source: RecordSource::Api,
        fetched_at: 1,
        deleted: false,
        v: 1,
    }
}

async fn check_backend(store: &dyn Storage) {
    let now = 25 * YEAR_SECONDS;
    let mut records = Vec::new();
    for (track, plays, years) in [("Frequent", 30, 3), ("Balanced", 25, 6), ("Old", 10, 12)] {
        for i in 0..plays {
            records.push(play(track, now - years * YEAR_SECONDS - i));
        }
    }
    store.append_scrobbles(&records).await.unwrap();
    let coverage = CoverageMap::from_segments([Segment::new(0, now + 1, now)]);
    store.save_coverage(&coverage).await.unwrap();
    store
        .save_sync_state(&SyncState {
            history_start_uts: Some(0),
            last_sync_at: Some(now),
        })
        .await
        .unwrap();
    let history = History::load(store, now + YEAR_SECONDS).await.unwrap();
    assert_eq!(history.as_of, now);
    assert!(history.counts_complete);
    let best = |gap_weight| {
        history.rank(Options {
            gap_weight,
            ..Options::default()
        })[0]
            .stats
            .track
            .as_str()
    };
    assert_eq!(best(0), "Frequent");
    assert_eq!(best(10), "Frequent");
    assert_eq!(best(50), "Balanced");
    assert_eq!(best(90), "Old");
    assert_eq!(best(100), "Old");
    let rows = history.rank(Options::default());
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].score, 150.0);
    assert!(rows.iter().all(|row| row.gap_verified));

    let recent = play("Balanced", now + 1);
    store
        .append_scrobbles(std::slice::from_ref(&recent))
        .await
        .unwrap();
    let edited = History::load(store, now + YEAR_SECONDS).await.unwrap();
    assert_eq!(edited.rank(Options::default()).len(), 2);
    store
        .append_scrobbles(&[recent.into_tombstone(2)])
        .await
        .unwrap();
    let range = now - 6 * YEAR_SECONDS..now;
    let bounded = store.track_stats(Some(range)).await.unwrap();
    let balanced = bounded.iter().find(|row| row.track == "Balanced").unwrap();
    assert_eq!(balanced.count, 1);
    assert_eq!(balanced.first_uts, balanced.last_uts);

    let mut partial = coverage;
    partial.subtract(now - 10 * YEAR_SECONDS..now - 9 * YEAR_SECONDS);
    store.save_coverage(&partial).await.unwrap();
    let history = History::load(store, now).await.unwrap();
    let rows = history.rank(Options::default());
    assert_eq!(rows.len(), 3);
    assert!(!history.counts_complete);
    assert!(rows
        .iter()
        .all(|r| r.gap_verified == (r.stats.track != "Old")));
}

#[tokio::test]
async fn rediscover_ranking_tracks_edits_and_coverage_on_both_backends() {
    check_backend(&MemoryStorage::new()).await;
    let dir = tempfile::tempdir().unwrap();
    check_backend(&FsStorage::open(dir.path()).unwrap()).await;
}
