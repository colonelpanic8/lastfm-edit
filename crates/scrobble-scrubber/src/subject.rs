//! The unit of analysis: one distinct metadata combination.
//!
//! Rules and providers reason about *metadata*, not scrobble instances. The planner
//! dedupes store records into [`Subject`]s, analyzes one representative
//! [`Track`](lastfm_edit::Track) per subject, and the executor later re-expands an
//! accepted subject-level edit into per-instance exact edits.

use lastfm_edit::Track;
use scrobble_store::{ScrobbleId, ScrobbleRecord};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One distinct `(artist, track, album, album_artist)` combination.
///
/// `album_artist` carries only *verified* values (from the store's provenance tracking);
/// unverified or unknown album artists appear as `None` and are resolved by the executor's
/// enrichment step at apply time. It is never guessed from the artist.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Subject {
    pub artist: String,
    pub track: String,
    pub album: Option<String>,
    pub album_artist: Option<String>,
}

impl Subject {
    /// The subject of a stored scrobble record.
    pub fn of_record(record: &ScrobbleRecord) -> Self {
        let album_artist = match &record.album_artist {
            scrobble_store::Provenanced::Verified(value) => Some(value.clone()),
            _ => None,
        };
        Self {
            artist: record.artist.clone(),
            track: record.track.clone(),
            album: record.album.clone(),
            album_artist,
        }
    }

    /// Whether a live record has this subject's metadata (exact match; the album_artist
    /// component is ignored when the subject doesn't pin one).
    pub fn matches_record(&self, record: &ScrobbleRecord) -> bool {
        if record.deleted {
            return false;
        }
        if record.artist != self.artist || record.track != self.track {
            return false;
        }
        if record.album != self.album {
            return false;
        }
        match &self.album_artist {
            None => true,
            Some(expected) => record.album_artist.value() == Some(expected),
        }
    }

    /// Synthesize the representative [`Track`] handed to providers for this subject.
    ///
    /// `playcount` is the number of live instances behind the subject in the current
    /// batch; `newest_uts` the most recent of their timestamps.
    pub fn representative_track(&self, playcount: u32, newest_uts: Option<u64>) -> Track {
        Track {
            name: self.track.clone(),
            artist: self.artist.clone(),
            playcount,
            timestamp: newest_uts,
            album: self.album.clone(),
            album_artist: self.album_artist.clone(),
        }
    }
}

impl std::fmt::Display for Subject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} — {}", self.artist, self.track)?;
        if let Some(album) = &self.album {
            write!(f, " [{album}]")?;
        }
        Ok(())
    }
}

/// Group live records by subject, preserving deterministic (subject-sorted) order.
/// Tombstoned records are skipped.
pub fn group_by_subject(records: &[ScrobbleRecord]) -> Vec<(Subject, Vec<ScrobbleId>)> {
    let mut groups: BTreeMap<Subject, Vec<ScrobbleId>> = BTreeMap::new();
    for record in records.iter().filter(|r| !r.deleted) {
        groups
            .entry(Subject::of_record(record))
            .or_default()
            .push(record.id.clone());
    }
    groups.into_iter().collect()
}

#[cfg(test)]
mod tests {
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
}
