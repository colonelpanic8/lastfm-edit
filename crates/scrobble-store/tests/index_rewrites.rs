use scrobble_store::{FsStorage, Provenanced, RecordSource, ScrobbleId, ScrobbleRecord, Storage};
fn record(uts: u64) -> ScrobbleRecord {
    ScrobbleRecord {
        id: ScrobbleId::new(uts, "Artist", "Track"),
        uts,
        artist: "Artist".into(),
        track: "Track".into(),
        album: Some("Before".into()),
        album_artist: Provenanced::Unknown,
        source: RecordSource::Api,
        fetched_at: 1,
        deleted: false,
        v: 1,
    }
}
#[tokio::test]
async fn index_detects_equal_and_growing_rewrites() {
    let tmp = tempfile::tempdir().unwrap();
    let store = FsStorage::open(tmp.path()).unwrap();
    let mut rec = record(100);
    store.append_scrobbles(&[rec.clone()]).await.unwrap();
    assert_eq!(
        store.top_albums(None, 10, None).await.unwrap()[0].album,
        "Before"
    );
    rec.album = Some("After!".into());
    rec.fetched_at = 2;
    let path = tmp.path().join("scrobbles/1970/1970-01.jsonl");
    let old_len = std::fs::metadata(&path).unwrap().len();
    std::fs::write(&path, format!("{}\n", serde_json::to_string(&rec).unwrap())).unwrap();
    assert_eq!(std::fs::metadata(&path).unwrap().len(), old_len);
    assert_eq!(
        store.get_scrobble(&rec.id).await.unwrap().unwrap().album,
        rec.album
    );
    assert_eq!(
        store.top_albums(None, 10, None).await.unwrap()[0].album,
        "After!"
    );
    rec.album = Some("A substantially longer rewritten album".into());
    rec.fetched_at = 3;
    std::fs::write(&path, format!("{}\n", serde_json::to_string(&rec).unwrap())).unwrap();
    assert!(std::fs::metadata(&path).unwrap().len() > old_len);
    assert_eq!(
        store.top_albums(None, 10, None).await.unwrap()[0].album,
        store
            .get_scrobble(&rec.id)
            .await
            .unwrap()
            .unwrap()
            .album
            .unwrap()
    );
    let mut next = record(200);
    next.album = rec.album.clone();
    store.append_scrobbles(&[next]).await.unwrap();
    assert_eq!(store.top_albums(None, 10, None).await.unwrap()[0].count, 2);
    let reopened = FsStorage::open(tmp.path()).unwrap();
    assert_eq!(reopened.scrobble_count(None).await.unwrap(), 2);
}

#[tokio::test]
async fn partial_tail_repair_and_unterminated_valid_record_match_direct_reads() {
    let tmp = tempfile::tempdir().unwrap();
    let store = FsStorage::open(tmp.path()).unwrap();
    store.append_scrobbles(&[record(100)]).await.unwrap();
    let path = tmp.path().join("scrobbles/1970/1970-01.jsonl");
    let contents = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, contents.trim_end()).unwrap();
    assert_eq!(store.scrobble_count(None).await.unwrap(), 1);
    std::fs::write(&path, format!("{contents}{{\"torn\":")).unwrap();
    assert_eq!(store.scrobble_count(None).await.unwrap(), 1);
    store.append_scrobbles(&[record(200)]).await.unwrap();
    assert_eq!(store.scrobble_count(None).await.unwrap(), 2);
    assert_eq!(store.scrobbles_in_range(0..300).await.unwrap().len(), 2);
    std::fs::write(&path, format!("bad\n{contents}")).unwrap();
    assert!(store.scrobble_count(None).await.is_err());
}
