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
    /// The identity-resolution key: everything but the album artist, which doesn't decide
    /// which recording a scrobble is.
    pub fn key(&self) -> ScrobbleKey {
        ScrobbleKey {
            artist: self.artist.clone(),
            track: self.track.clone(),
            album: self.album.clone(),
        }
    }

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

/// The unit of MusicBrainz identity resolution: one distinct `(artist, track, album)`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ScrobbleKey {
    pub artist: String,
    pub track: String,
    pub album: Option<String>,
}

impl ScrobbleKey {
    /// Live records with exactly this key, via the indexed per-artist query.
    pub async fn live_records(
        &self,
        store: &dyn scrobble_store::Storage,
    ) -> scrobble_store::Result<Vec<ScrobbleRecord>> {
        let mut records = store.artist_scrobbles(&self.artist, None).await?;
        records.retain(|record| record.track == self.track && record.album == self.album);
        Ok(records)
    }
}

impl std::fmt::Display for ScrobbleKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} — {}", self.artist, self.track)?;
        if let Some(album) = &self.album {
            write!(f, " [{album}]")?;
        }
        Ok(())
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
#[path = "subject_tests.rs"]
mod tests;
