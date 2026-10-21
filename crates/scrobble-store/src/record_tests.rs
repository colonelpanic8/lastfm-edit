use super::*;

fn track(timestamp: Option<u64>, album_artist: Option<&str>) -> Track {
    Track {
        name: "Roygbiv".to_string(),
        artist: "Boards of Canada".to_string(),
        playcount: 1,
        timestamp,
        album: Some("Music Has the Right to Children".to_string()),
        album_artist: album_artist.map(str::to_string),
    }
}

#[test]
fn now_playing_rows_are_rejected() {
    assert!(ScrobbleRecord::from_track(&track(None, None), RecordSource::Api, 10).is_none());
}

#[test]
fn api_source_without_album_artist_is_unknown() {
    let rec = ScrobbleRecord::from_track(&track(Some(100), None), RecordSource::Api, 10).unwrap();
    assert_eq!(rec.album_artist, Provenanced::Unknown);
    assert_eq!(rec.uts, 100);
    assert_eq!(rec.id, ScrobbleId::new(100, "Boards of Canada", "Roygbiv"));
}

#[test]
fn scrape_source_album_artist_is_verified() {
    let rec = ScrobbleRecord::from_track(
        &track(Some(100), Some("Various Artists")),
        RecordSource::Scrape,
        10,
    )
    .unwrap();
    assert_eq!(
        rec.album_artist,
        Provenanced::Verified("Various Artists".to_string())
    );
}

#[test]
fn api_source_with_album_artist_is_only_unverified() {
    let rec = ScrobbleRecord::from_track(&track(Some(100), Some("guess")), RecordSource::Api, 10)
        .unwrap();
    assert_eq!(
        rec.album_artist,
        Provenanced::Unverified("guess".to_string())
    );
}

#[test]
fn lww_supersedes_by_fetched_at() {
    let old = ScrobbleRecord::from_track(&track(Some(100), None), RecordSource::Api, 10).unwrap();
    let new = ScrobbleRecord::from_track(&track(Some(100), None), RecordSource::Api, 20).unwrap();
    assert!(new.supersedes(&old));
    assert!(!old.supersedes(&new));
    // Ties resolve in favor of the record being applied (idempotent re-appends).
    assert!(old.supersedes(&old.clone()));
}

#[test]
fn tombstone_round_trip() {
    let rec = ScrobbleRecord::from_track(&track(Some(100), None), RecordSource::Api, 10).unwrap();
    let tomb = rec.clone().into_tombstone(30);
    assert!(tomb.deleted);
    assert!(tomb.supersedes(&rec));
    let json = serde_json::to_string(&tomb).unwrap();
    assert!(json.contains("\"deleted\":true"));
    let back: ScrobbleRecord = serde_json::from_str(&json).unwrap();
    assert_eq!(back, tomb);
}

#[test]
fn serde_shape_matches_design() {
    let rec = ScrobbleRecord::from_track(
        &track(Some(100), Some("Various Artists")),
        RecordSource::Scrape,
        10,
    )
    .unwrap();
    let json = serde_json::to_string(&rec).unwrap();
    assert!(
        json.contains("\"album_artist\":{\"state\":\"verified\",\"value\":\"Various Artists\"}")
    );
    assert!(json.contains("\"source\":\"scrape\""));
    // Live records serialize without a `deleted` field.
    assert!(!json.contains("deleted"));
    // Unknown album artist serializes without a value payload.
    let rec2 = ScrobbleRecord::from_track(&track(Some(100), None), RecordSource::Api, 10).unwrap();
    let json2 = serde_json::to_string(&rec2).unwrap();
    assert!(json2.contains("\"album_artist\":{\"state\":\"unknown\"}"));
}
