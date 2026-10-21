use super::*;
use scrobble_store::{Provenanced, RecordSource};

fn record(
    uts: u64,
    artist: &str,
    track: &str,
    album_artist: Provenanced<String>,
) -> ScrobbleRecord {
    ScrobbleRecord {
        id: ScrobbleId::new(uts, artist, track),
        uts,
        artist: artist.to_string(),
        track: track.to_string(),
        album: Some("Album".to_string()),
        album_artist,
        source: RecordSource::Api,
        fetched_at: 1,
        deleted: false,
        v: 1,
    }
}

#[test]
fn verified_album_artist_only() {
    let verified = record(1, "A", "x", Provenanced::Verified("VA".into()));
    assert_eq!(
        Subject::of_record(&verified).album_artist.as_deref(),
        Some("VA")
    );
    let unverified = record(2, "A", "x", Provenanced::Unverified("guess".into()));
    assert_eq!(Subject::of_record(&unverified).album_artist, None);
    let unknown = record(3, "A", "x", Provenanced::Unknown);
    assert_eq!(Subject::of_record(&unknown).album_artist, None);
}

#[test]
fn grouping_dedupes_and_skips_tombstones() {
    let mut dead = record(4, "A", "x", Provenanced::Unknown);
    dead.deleted = true;
    let records = vec![
        record(1, "A", "x", Provenanced::Unknown),
        record(2, "A", "x", Provenanced::Unknown),
        record(3, "B", "y", Provenanced::Unknown),
        dead,
    ];
    let groups = group_by_subject(&records);
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].1.len(), 2); // A — x
    assert_eq!(groups[1].1.len(), 1); // B — y
}

#[test]
fn representative_track_is_honest() {
    let subject = Subject::of_record(&record(1, "A", "x", Provenanced::Unknown));
    let track = subject.representative_track(3, Some(100));
    assert_eq!(track.playcount, 3);
    assert_eq!(track.album_artist, None); // never fabricated
}

#[test]
fn matches_record_semantics() {
    let base = record(1, "A", "x", Provenanced::Unknown);
    let subject = Subject::of_record(&base);
    assert!(subject.matches_record(&record(9, "A", "x", Provenanced::Unknown)));
    // Subject without pinned album_artist matches any provenance.
    assert!(subject.matches_record(&record(9, "A", "x", Provenanced::Verified("VA".into()))));
    // Pinned album_artist requires the verified value.
    let pinned = Subject {
        album_artist: Some("VA".into()),
        ..subject.clone()
    };
    assert!(pinned.matches_record(&record(9, "A", "x", Provenanced::Verified("VA".into()))));
    assert!(!pinned.matches_record(&record(9, "A", "x", Provenanced::Unknown)));
}
