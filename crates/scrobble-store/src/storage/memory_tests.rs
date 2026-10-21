use super::*;
use crate::record::{Provenanced, RecordSource};

fn rec(uts: u64, artist: &str, track: &str, fetched_at: u64) -> ScrobbleRecord {
    ScrobbleRecord {
        id: ScrobbleId::new(uts, artist, track),
        uts,
        artist: artist.to_string(),
        track: track.to_string(),
        album: None,
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

#[tokio::test]
async fn append_is_idempotent_and_lww() {
    let store = MemoryStorage::new();
    let a = rec(100, "A", "x", 10);

    let stats = store
        .append_scrobbles(std::slice::from_ref(&a))
        .await
        .unwrap();
    assert_eq!(
        stats,
        AppendStats {
            new: 1,
            updated: 0,
            unchanged: 0
        }
    );

    // Re-append: unchanged.
    let stats = store
        .append_scrobbles(std::slice::from_ref(&a))
        .await
        .unwrap();
    assert_eq!(stats.unchanged, 1);

    // Newer observation with different content: updated.
    let mut newer = a.clone();
    newer.fetched_at = 20;
    newer.album = Some("Album".to_string());
    let stats = store
        .append_scrobbles(std::slice::from_ref(&newer))
        .await
        .unwrap();
    assert_eq!(stats.updated, 1);
    assert_eq!(store.get_scrobble(&a.id).await.unwrap().unwrap(), newer);

    // Older observation loses.
    let mut older = a.clone();
    older.fetched_at = 5;
    older.album = Some("Stale".to_string());
    let stats = store.append_scrobbles(&[older]).await.unwrap();
    assert_eq!(stats.unchanged, 1);
    assert_eq!(store.get_scrobble(&a.id).await.unwrap().unwrap(), newer);
}

#[tokio::test]
async fn tombstones_hide_from_reads_but_remain_gettable() {
    let store = MemoryStorage::new();
    let a = rec(100, "A", "x", 10);
    store
        .append_scrobbles(std::slice::from_ref(&a))
        .await
        .unwrap();
    store
        .append_scrobbles(&[a.clone().into_tombstone(20)])
        .await
        .unwrap();

    assert!(store.scrobbles_in_range(0..1000).await.unwrap().is_empty());
    assert_eq!(store.latest_uts().await.unwrap(), None);
    let got = store.get_scrobble(&a.id).await.unwrap().unwrap();
    assert!(got.deleted);
}

#[tokio::test]
async fn range_reads_are_sorted_and_bounded() {
    let store = MemoryStorage::new();
    store
        .append_scrobbles(&[
            rec(300, "A", "x", 1),
            rec(100, "B", "y", 1),
            rec(200, "C", "z", 1),
        ])
        .await
        .unwrap();
    let got = store.scrobbles_in_range(100..300).await.unwrap();
    assert_eq!(
        got.iter().map(|r| r.uts).collect::<Vec<_>>(),
        vec![100, 200]
    );
    assert_eq!(store.latest_uts().await.unwrap(), Some(300));
}

#[tokio::test]
async fn aggregates() {
    let store = MemoryStorage::new();
    store
        .append_scrobbles(&[
            rec(1, "A", "x", 1),
            rec(2, "A", "x", 1),
            rec(3, "A", "y", 1),
            rec(4, "B", "z", 1),
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
    assert_eq!(
        artists[1],
        ArtistCount {
            artist: "B".into(),
            count: 1
        }
    );

    let tracks = store.top_tracks(Some("A"), 10, None).await.unwrap();
    assert_eq!(tracks[0].track, "x");
    assert_eq!(tracks[0].count, 2);

    let ranged = store.top_artists(10, Some(3..5)).await.unwrap();
    assert_eq!(ranged.len(), 2);
    assert_eq!(ranged[0].count, 1);

    let scrobbles = store.artist_scrobbles("A", None).await.unwrap();
    assert_eq!(scrobbles.len(), 3);
}

#[tokio::test]
async fn top_albums_and_scrobble_count() {
    let store = MemoryStorage::new();
    store
        .append_scrobbles(&[
            rec_album(1, "A", "x", Some("One")),
            rec_album(2, "A", "y", Some("One")),
            rec_album(3, "A", "z", Some("Two")),
            rec_album(4, "B", "w", Some("Three")),
            rec_album(5, "A", "n", None),
            rec_album(6, "A", "e", Some("")),
        ])
        .await
        .unwrap();

    // Grouped by artist+album, descending by count; None/empty albums excluded.
    let albums = store.top_albums(None, 10, None).await.unwrap();
    assert_eq!(
        albums[0],
        AlbumCount {
            artist: "A".into(),
            album: "One".into(),
            count: 2
        }
    );
    assert_eq!(albums.len(), 3);
    assert!(albums
        .iter()
        .all(|a| !a.album.is_empty() && a.album != "Three" || a.artist == "B"));

    // Artist filter.
    let a_albums = store.top_albums(Some("A"), 10, None).await.unwrap();
    assert_eq!(a_albums.len(), 2);
    assert!(a_albums.iter().all(|a| a.artist == "A"));

    // scrobble_count counts every live record, album or not.
    assert_eq!(store.scrobble_count(None).await.unwrap(), 6);
    assert_eq!(store.scrobble_count(Some(1..4)).await.unwrap(), 3);

    // Deleted records drop out of both.
    store
        .append_scrobbles(&[rec_album(1, "A", "x", Some("One")).into_tombstone(9)])
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
    let store = MemoryStorage::new();
    store
        .append_scrobbles(&[
            rec(10, "A", "x", 1),
            rec(10, "A", "y", 1), // same uts, different id
            rec(20, "A", "z", 1),
            rec(30, "B", "w", 1),
        ])
        .await
        .unwrap();

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
    assert_eq!(all.len(), 4);
    assert_eq!(all[0].uts, 30);
    assert!(all
        .windows(2)
        .all(|w| (w[0].uts, w[0].id.as_str()) >= (w[1].uts, w[1].id.as_str())));
    let ids: std::collections::HashSet<_> = all.iter().map(|r| r.id.clone()).collect();
    assert_eq!(ids.len(), 4);

    // Tombstones excluded.
    store
        .append_scrobbles(&[rec(30, "B", "w", 1).into_tombstone(9)])
        .await
        .unwrap();
    let after = store.recent_scrobbles(None, 10).await.unwrap();
    assert_eq!(after.len(), 3);
    assert!(after.iter().all(|r| r.uts != 30));
}

#[tokio::test]
async fn search_scrobbles_groups_and_pages() {
    let store = MemoryStorage::new();
    store
        .append_scrobbles(&[
            rec(10, "Boards of Canada", "Roygbiv", 1),
            rec(20, "Boards of Canada", "Roygbiv", 1),
            rec(30, "Boards of Canada", "Dayvan Cowboy", 1),
        ])
        .await
        .unwrap();

    let grouped = store.search_scrobbles("boards royg", 0, 10).await.unwrap();
    assert_eq!(grouped.len(), 1);
    assert_eq!(grouped[0].count, 2);
    assert_eq!((grouped[0].first_uts, grouped[0].latest_uts), (10, 20));

    let page = store.search_scrobbles("boards", 1, 1).await.unwrap();
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].track, "Roygbiv");
}

#[tokio::test]
async fn search_recent_scrobbles_filters_and_pages_newest_first() {
    let store = MemoryStorage::new();
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
async fn coverage_and_sync_state_round_trip() {
    let store = MemoryStorage::new();
    let mut cov = CoverageMap::new();
    cov.insert(crate::coverage::Segment::new(10, 20, 5));
    store.save_coverage(&cov).await.unwrap();
    assert_eq!(store.load_coverage().await.unwrap(), cov);

    let state = SyncState {
        history_start_uts: Some(1),
        last_sync_at: Some(2),
    };
    store.save_sync_state(&state).await.unwrap();
    assert_eq!(store.load_sync_state().await.unwrap(), state);
}
