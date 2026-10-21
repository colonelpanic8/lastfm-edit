use super::*;
use crate::record::{Provenanced, RecordSource};

fn rec(uts: u64, artist: &str, track: &str, fetched_at: u64) -> ScrobbleRecord {
    ScrobbleRecord {
        id: ScrobbleId::new(uts, artist, track),
        uts,
        artist: artist.to_string(),
        track: track.to_string(),
        album: Some("Album".to_string()),
        album_artist: Provenanced::Unknown,
        source: RecordSource::Api,
        fetched_at,
        deleted: false,
        v: 1,
    }
}

fn rec_album(uts: u64, artist: &str, track: &str, album: Option<&str>) -> ScrobbleRecord {
    ScrobbleRecord {
        album: album.map(str::to_string),
        ..rec(uts, artist, track, 1)
    }
}

// 2024-01-15 and 2024-02-15 (UTC), so records land in different partitions.
const JAN: u64 = 1_705_312_800;
const FEB: u64 = 1_707_991_200;

#[tokio::test]
async fn round_trip_and_partitioning() {
    let dir = tempfile::tempdir().unwrap();
    let store = FsStorage::open(dir.path()).unwrap();

    let stats = store
        .append_scrobbles(&[rec(JAN, "A", "x", 1), rec(FEB, "B", "y", 1)])
        .await
        .unwrap();
    assert_eq!(stats.new, 2);

    assert!(dir.path().join("scrobbles/2024/2024-01.jsonl").exists());
    assert!(dir.path().join("scrobbles/2024/2024-02.jsonl").exists());
    assert!(dir.path().join(".gitattributes").exists());

    let all = store.scrobbles_in_range(0..u64::MAX).await.unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].artist, "A");
    assert_eq!(store.latest_uts().await.unwrap(), Some(FEB));

    // Fresh handle re-reads from disk identically.
    let store2 = FsStorage::open(dir.path()).unwrap();
    let all2 = store2.scrobbles_in_range(0..u64::MAX).await.unwrap();
    assert_eq!(all, all2);
}

#[test]
fn compaction_preserves_concurrent_appends_from_another_handle() {
    let dir = tempfile::tempdir().unwrap();
    let barrier = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let store = FsStorage::open(dir.path()).unwrap();
            let runtime = tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap();
            for index in 0..100 {
                let original = rec(JAN + index, "A", "x", 1);
                let mut updated = original.clone();
                updated.album = Some("Updated".into());
                updated.fetched_at = 2;
                updated.deleted = index % 2 == 1;
                barrier.wait();
                runtime
                    .block_on(store.append_scrobbles(&[original, updated]))
                    .unwrap();
                barrier.wait();
            }
        });
        let store = FsStorage::open(dir.path()).unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        for _ in 0..100 {
            barrier.wait();
            runtime.block_on(store.compact()).unwrap();
            barrier.wait();
        }
    });
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    runtime.block_on(async {
        let store = FsStorage::open(dir.path()).unwrap();
        for index in 0..100 {
            let record = store
                .get_scrobble(&ScrobbleId::new(JAN + index, "A", "x"))
                .await
                .unwrap()
                .unwrap_or_else(|| panic!("compaction lost record {index}"));
            assert_eq!(record.album.as_deref(), Some("Updated"));
            assert_eq!(record.deleted, index % 2 == 1);
        }
        assert_eq!(store.scrobble_count(None).await.unwrap(), 50);
    });
}

#[tokio::test]
async fn lww_append_and_compaction() {
    let dir = tempfile::tempdir().unwrap();
    let store = FsStorage::open(dir.path()).unwrap();

    let a = rec(JAN, "A", "x", 1);
    store
        .append_scrobbles(std::slice::from_ref(&a))
        .await
        .unwrap();
    // Idempotent re-append writes nothing.
    let stats = store
        .append_scrobbles(std::slice::from_ref(&a))
        .await
        .unwrap();
    assert_eq!(stats.unchanged, 1);

    let mut refetched = a.clone();
    refetched.fetched_at = 5;
    let stats = store
        .append_scrobbles(std::slice::from_ref(&refetched))
        .await
        .unwrap();
    assert_eq!(stats.unchanged, 1);

    let mut verified = a.clone();
    verified.album_artist = Provenanced::Verified("AA".to_string());
    verified.fetched_at = 6;
    store
        .append_scrobbles(std::slice::from_ref(&verified))
        .await
        .unwrap();
    let mut blind_refetch = a.clone();
    blind_refetch.fetched_at = 7;
    let stats = store
        .append_scrobbles(std::slice::from_ref(&blind_refetch))
        .await
        .unwrap();
    assert_eq!(stats.unchanged, 1);
    let got = store.get_scrobble(&a.id).await.unwrap().unwrap();
    assert_eq!(got.album_artist, Provenanced::Verified("AA".to_string()));

    let mut newer = a.clone();
    newer.fetched_at = 8;
    newer.album = Some("Corrected".to_string());
    let stats = store
        .append_scrobbles(std::slice::from_ref(&newer))
        .await
        .unwrap();
    assert_eq!(stats.updated, 1);

    // Three lines on disk, one logical record.
    let path = dir.path().join("scrobbles/2024/2024-01.jsonl");
    assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 3);
    let got = store.get_scrobble(&a.id).await.unwrap().unwrap();
    assert_eq!(got.album.as_deref(), Some("Corrected"));

    let dropped = store.compact().await.unwrap();
    assert_eq!(dropped, 2);
    assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 1);
    let got = store.get_scrobble(&a.id).await.unwrap().unwrap();
    assert_eq!(got.album.as_deref(), Some("Corrected"));
}

#[tokio::test]
async fn tombstones_survive_compaction_and_hide_from_reads() {
    let dir = tempfile::tempdir().unwrap();
    let store = FsStorage::open(dir.path()).unwrap();
    let a = rec(JAN, "A", "x", 1);
    store
        .append_scrobbles(std::slice::from_ref(&a))
        .await
        .unwrap();
    store
        .append_scrobbles(&[a.clone().into_tombstone(2)])
        .await
        .unwrap();
    store.compact().await.unwrap();

    assert!(store
        .scrobbles_in_range(0..u64::MAX)
        .await
        .unwrap()
        .is_empty());
    let got = store.get_scrobble(&a.id).await.unwrap().unwrap();
    assert!(got.deleted);
}

#[tokio::test]
async fn torn_final_line_is_tolerated() {
    let dir = tempfile::tempdir().unwrap();
    let store = FsStorage::open(dir.path()).unwrap();
    store
        .append_scrobbles(&[rec(JAN, "A", "x", 1)])
        .await
        .unwrap();

    // Simulate a crash mid-append: valid line + torn line.
    let path = dir.path().join("scrobbles/2024/2024-01.jsonl");
    let mut content = std::fs::read_to_string(&path).unwrap();
    content.push_str("{\"id\":\"170531");
    std::fs::write(&path, content).unwrap();

    let store2 = FsStorage::open(dir.path()).unwrap();
    let all = store2.scrobbles_in_range(0..u64::MAX).await.unwrap();
    assert_eq!(all.len(), 1);
    store2
        .append_scrobbles(&[rec(JAN + 60, "A", "new", 2)])
        .await
        .unwrap();
    let reopened = FsStorage::open(dir.path()).unwrap();
    assert_eq!(
        reopened
            .scrobbles_in_range(0..u64::MAX)
            .await
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn coverage_and_sync_state_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let store = FsStorage::open(dir.path()).unwrap();

    assert!(store.load_coverage().await.unwrap().is_empty());
    let mut cov = CoverageMap::new();
    cov.insert(Segment::new(10, 20, 5));
    cov.insert(Segment::new(30, 40, 6));
    store.save_coverage(&cov).await.unwrap();
    assert_eq!(store.load_coverage().await.unwrap(), cov);

    let state = SyncState {
        history_start_uts: Some(123),
        last_sync_at: Some(456),
    };
    store.save_sync_state(&state).await.unwrap();
    assert_eq!(store.load_sync_state().await.unwrap(), state);
}

#[tokio::test]
async fn union_merged_coverage_normalizes_on_load() {
    let dir = tempfile::tempdir().unwrap();
    let store = FsStorage::open(dir.path()).unwrap();
    // Simulate the aftermath of a git conflict resolved by unioning segment lists:
    // overlapping, out-of-order segments.
    let raw = r#"{"v":1,"segments":[
            {"start":30,"end":50,"verified_at":2},
            {"start":10,"end":35,"verified_at":1}
        ]}"#;
    std::fs::write(dir.path().join("state/coverage.json"), raw).unwrap();
    let cov = store.load_coverage().await.unwrap();
    assert_eq!(cov.segments().len(), 1);
    assert_eq!(cov.segments()[0].range(), 10..50);
}

#[tokio::test]
async fn indexed_queries_and_incremental_ingest() {
    let dir = tempfile::tempdir().unwrap();
    let store = FsStorage::open(dir.path()).unwrap();
    store
        .append_scrobbles(&[
            rec(JAN, "A", "x", 1),
            rec(JAN + 60, "A", "x", 1),
            rec(JAN + 120, "A", "y", 1),
            rec(FEB, "B", "z", 1),
        ])
        .await
        .unwrap();

    let artists = store.top_artists(10, None).await.unwrap();
    assert_eq!(
        artists[0],
        ArtistCount {
            artist: "A".into(),
            count: 3
        }
    );

    // Appends after the first query are picked up incrementally.
    store
        .append_scrobbles(&[rec(FEB + 60, "B", "z", 1), rec(FEB + 120, "B", "w", 1)])
        .await
        .unwrap();
    let artists = store.top_artists(10, None).await.unwrap();
    assert_eq!(artists[0].count, 3);
    assert_eq!(
        artists[1],
        ArtistCount {
            artist: "B".into(),
            count: 3
        }
    );

    let tracks = store.top_tracks(Some("B"), 10, None).await.unwrap();
    assert_eq!(tracks[0].track, "z");
    assert_eq!(tracks[0].count, 2);

    let scrobbles = store.artist_scrobbles("A", None).await.unwrap();
    assert_eq!(scrobbles.len(), 3);
    assert!(scrobbles.windows(2).all(|w| w[0].uts <= w[1].uts));

    // A tombstone updates the aggregates (via LWW upsert in the index).
    let dead = rec(JAN, "A", "x", 1).into_tombstone(9);
    store.append_scrobbles(&[dead]).await.unwrap();
    let artists = store.top_artists(10, None).await.unwrap();
    assert_eq!(artists.iter().find(|a| a.artist == "A").unwrap().count, 2);
}

#[tokio::test]
async fn top_albums_and_scrobble_count() {
    let dir = tempfile::tempdir().unwrap();
    let store = FsStorage::open(dir.path()).unwrap();
    store
        .append_scrobbles(&[
            rec_album(JAN, "A", "x", Some("One")),
            rec_album(JAN + 60, "A", "y", Some("One")),
            rec_album(JAN + 120, "A", "z", Some("Two")),
            rec_album(FEB, "B", "w", Some("Three")),
            rec_album(FEB + 60, "A", "n", None),
            rec_album(FEB + 120, "A", "e", Some("")),
        ])
        .await
        .unwrap();

    let albums = store.top_albums(None, 10, None).await.unwrap();
    assert_eq!(
        albums[0],
        AlbumCount {
            artist: "A".into(),
            album: "One".into(),
            count: 2
        }
    );
    // None and empty albums are excluded.
    assert_eq!(albums.len(), 3);

    let a_albums = store.top_albums(Some("A"), 10, None).await.unwrap();
    assert_eq!(a_albums.len(), 2);
    assert!(a_albums.iter().all(|a| a.artist == "A"));

    assert_eq!(store.scrobble_count(None).await.unwrap(), 6);
    assert_eq!(store.scrobble_count(Some(JAN..JAN + 121)).await.unwrap(), 3);

    // Tombstones drop out of both aggregates.
    store
        .append_scrobbles(&[rec_album(JAN, "A", "x", Some("One")).into_tombstone(9)])
        .await
        .unwrap();
    assert_eq!(store.scrobble_count(None).await.unwrap(), 5);
    let albums = store.top_albums(None, 10, None).await.unwrap();
    assert_eq!(
        albums.iter().find(|a| a.album == "One").map(|a| a.count),
        Some(1)
    );
}

#[tokio::test]
async fn recent_scrobbles_descending_keyset_pagination() {
    let dir = tempfile::tempdir().unwrap();
    let store = FsStorage::open(dir.path()).unwrap();
    // Two records share JAN (different track → different id); the rest are spread out.
    store
        .append_scrobbles(&[
            rec(JAN, "A", "x", 1),
            rec(JAN, "A", "y", 1),
            rec(JAN + 60, "A", "z", 1),
            rec(FEB, "B", "w", 1),
            rec(FEB + 60, "B", "v", 1),
        ])
        .await
        .unwrap();

    // Newest-first, no cursor.
    let p1 = store.recent_scrobbles(None, 2).await.unwrap();
    assert_eq!(p1.len(), 2);
    assert_eq!(p1[0].uts, FEB + 60);
    assert_eq!(p1[1].uts, FEB);

    // Walk every page with the keyset cursor; expect no gaps or duplicates.
    let mut all = Vec::new();
    let mut cursor: Option<(u64, ScrobbleId)> = None;
    loop {
        let page = store.recent_scrobbles(cursor.clone(), 2).await.unwrap();
        if page.is_empty() {
            break;
        }
        cursor = page.last().map(|r| (r.uts, r.id.clone()));
        all.extend(page);
    }
    assert_eq!(all.len(), 5);
    let ids: std::collections::HashSet<_> = all.iter().map(|r| r.id.clone()).collect();
    assert_eq!(ids.len(), 5, "no duplicate records across pages");
    // Fully descending, including the same-second id tiebreak.
    assert!(all
        .windows(2)
        .all(|w| (w[0].uts, w[0].id.as_str()) >= (w[1].uts, w[1].id.as_str())));
    // The two JAN records are adjacent at the tail (oldest), both present.
    assert_eq!(all[3].uts, JAN);
    assert_eq!(all[4].uts, JAN);

    // Tombstones are excluded.
    store
        .append_scrobbles(&[rec(FEB + 60, "B", "v", 1).into_tombstone(9)])
        .await
        .unwrap();
    let after = store.recent_scrobbles(None, 10).await.unwrap();
    assert_eq!(after.len(), 4);
    assert!(after.iter().all(|r| r.uts != FEB + 60));

    // Records appended after an earlier query are picked up (index catch_up).
    store
        .append_scrobbles(&[rec(FEB + 120, "B", "n", 1)])
        .await
        .unwrap();
    let fresh = store.recent_scrobbles(None, 1).await.unwrap();
    assert_eq!(fresh[0].uts, FEB + 120);
}

#[tokio::test]
async fn search_scrobbles_groups_matches_and_catches_up_incrementally() {
    let dir = tempfile::tempdir().unwrap();
    let store = FsStorage::open(dir.path()).unwrap();
    store
        .append_scrobbles(&[
            rec_album(
                JAN,
                "Boards of Canada",
                "Roygbiv",
                Some("Music Has the Right"),
            ),
            rec_album(
                JAN + 60,
                "Boards of Canada",
                "Roygbiv",
                Some("Music Has the Right"),
            ),
            rec_album(FEB, "Boards of Canada", "Dayvan Cowboy", Some("Campfire")),
            rec_album(FEB + 60, "Other", "Roygbiv", None),
        ])
        .await
        .unwrap();

    let results = store.search_scrobbles("boards royg", 0, 10).await.unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].count, 2);
    assert_eq!(results[0].first_uts, JAN);
    assert_eq!(results[0].latest_uts, JAN + 60);

    // Album is part of identity, and LIKE wildcards in user input stay literal.
    assert_eq!(
        store.search_scrobbles("camp", 0, 10).await.unwrap().len(),
        1
    );
    assert!(store.search_scrobbles("%", 0, 10).await.unwrap().is_empty());

    // A later append is visible without an explicit rebuild.
    store
        .append_scrobbles(&[rec_album(
            FEB + 120,
            "Boards of Canada",
            "Roygbiv",
            Some("Music Has the Right"),
        )])
        .await
        .unwrap();
    let refreshed = store.search_scrobbles("roygbiv", 0, 10).await.unwrap();
    assert_eq!(refreshed[0].count, 3);
    assert_eq!(refreshed[0].latest_uts, FEB + 120);

    let second_page = store.search_scrobbles("roygbiv", 1, 1).await.unwrap();
    assert_eq!(second_page.len(), 1);
    assert_eq!(second_page[0].artist, "Other");
}

#[tokio::test]
async fn search_recent_scrobbles_filters_and_pages_newest_first() {
    let dir = tempfile::tempdir().unwrap();
    let store = FsStorage::open(dir.path()).unwrap();
    store
        .append_scrobbles(&[
            rec(10, "Boards of Canada", "Roygbiv", 1),
            rec(20, "Boards of Canada", "Roygbiv", 1),
            rec(30, "Boards of Canada", "Dayvan Cowboy", 1),
            rec(40, "Other", "Roygbiv", 1),
        ])
        .await
        .unwrap();

    let first = store
        .search_recent_scrobbles("boards royg", None, 1)
        .await
        .unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].uts, 20);

    let cursor = Some((first[0].uts, first[0].id.clone()));
    let second = store
        .search_recent_scrobbles("boards royg", cursor, 1)
        .await
        .unwrap();
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].uts, 10);
}

#[tokio::test]
async fn compaction_triggers_index_rebuild_and_reindex_works() {
    let dir = tempfile::tempdir().unwrap();
    let store = FsStorage::open(dir.path()).unwrap();
    let a = rec(JAN, "A", "x", 1);
    let mut newer = a.clone();
    newer.fetched_at = 2;
    store.append_scrobbles(&[a]).await.unwrap();
    store.append_scrobbles(&[newer]).await.unwrap();

    // Prime the index, then compact (file shrinks), then query again.
    assert_eq!(store.top_artists(10, None).await.unwrap()[0].count, 1);
    store.compact().await.unwrap();
    assert_eq!(store.top_artists(10, None).await.unwrap()[0].count, 1);

    // Explicit reindex from scratch.
    store.reindex().await.unwrap();
    assert_eq!(store.top_artists(10, None).await.unwrap()[0].count, 1);

    // A fresh handle (fresh index connection) sees the same state.
    let store2 = FsStorage::open(dir.path()).unwrap();
    assert_eq!(store2.top_artists(10, None).await.unwrap()[0].count, 1);
}
