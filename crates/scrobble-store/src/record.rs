//! The scrobble record: the unit of storage, with metadata provenance.

use crate::id::ScrobbleId;
use lastfm_edit::Track;
use serde::{Deserialize, Serialize};

/// A value together with how much we trust where it came from.
///
/// Used for metadata (currently album artist) that some sources provide authoritatively
/// (Last.fm edit forms), some sources guess at, and some sources omit entirely (the official
/// recent-tracks API).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "value", rename_all = "lowercase")]
pub enum Provenanced<T> {
    /// Obtained from an authoritative source (e.g. scraped Last.fm edit-form values).
    Verified(T),
    /// Present but not confirmed against an authoritative source.
    Unverified(T),
    /// The source did not provide this value.
    Unknown,
}

impl<T> Provenanced<T> {
    /// The value regardless of trust level, if there is one.
    pub fn value(&self) -> Option<&T> {
        match self {
            Provenanced::Verified(v) | Provenanced::Unverified(v) => Some(v),
            Provenanced::Unknown => None,
        }
    }

    /// Whether the value is verified against an authoritative source.
    pub fn is_verified(&self) -> bool {
        matches!(self, Provenanced::Verified(_))
    }
}

/// Where a record (or the fetch that produced it) came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordSource {
    /// The official Last.fm API (`user.getrecenttracks`).
    Api,
    /// Scraped from Last.fm web pages.
    Scrape,
    /// Written locally as the mirror of an edit we applied upstream.
    EditMirror,
    /// Entered manually.
    Manual,
}

fn default_schema_version() -> u32 {
    1
}

fn is_false(b: &bool) -> bool {
    !b
}

/// One scrobble as stored locally. Append-only on disk; the record with the newest
/// `fetched_at` for a given [`ScrobbleId`] wins (last-write-wins), and `deleted: true`
/// records are tombstones.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScrobbleRecord {
    pub id: ScrobbleId,
    /// Unix timestamp of the scrobble itself.
    pub uts: u64,
    pub artist: String,
    pub track: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album: Option<String>,
    /// Album artist with provenance — see [`Provenanced`]. Editing a scrobble requires the
    /// verified value; unverified/unknown values must be resolved via scraping first.
    pub album_artist: Provenanced<String>,
    /// Which kind of source produced this observation of the scrobble.
    pub source: RecordSource,
    /// When we observed this state (seconds since epoch). LWW tiebreaker.
    pub fetched_at: u64,
    /// Tombstone marker: the scrobble no longer exists upstream.
    #[serde(default, skip_serializing_if = "is_false")]
    pub deleted: bool,
    /// Record schema version.
    #[serde(default = "default_schema_version")]
    pub v: u32,
}

impl ScrobbleRecord {
    /// Build a record from a [`Track`] returned by lastfm-edit.
    ///
    /// Returns `None` for tracks without a timestamp (e.g. "now playing" rows), which are not
    /// scrobbles and cannot be stored.
    ///
    /// Album-artist trust is derived from the source: values from scraped pages are
    /// `Verified`, values from anywhere else are `Unverified`, and a missing value is
    /// `Unknown`. This is deliberately defensive: even if a future lastfm-edit regression
    /// fabricates an album artist on the API path again, it enters the store as `Unverified`
    /// and edits will still resolve the real value first.
    pub fn from_track(track: &Track, source: RecordSource, fetched_at: u64) -> Option<Self> {
        let uts = track.timestamp?;
        let album_artist = match (&track.album_artist, source) {
            (Some(aa), RecordSource::Scrape) => Provenanced::Verified(aa.clone()),
            (Some(aa), _) => Provenanced::Unverified(aa.clone()),
            (None, _) => Provenanced::Unknown,
        };
        Some(Self {
            id: ScrobbleId::new(uts, &track.artist, &track.name),
            uts,
            artist: track.artist.clone(),
            track: track.name.clone(),
            album: track.album.clone(),
            album_artist,
            source,
            fetched_at,
            deleted: false,
            v: default_schema_version(),
        })
    }

    /// A tombstone for this record, observed gone at `at`.
    pub fn into_tombstone(mut self, at: u64) -> Self {
        self.deleted = true;
        self.fetched_at = at;
        self
    }

    /// Last-write-wins: whether this observation supersedes `other` (same id assumed).
    pub fn supersedes(&self, other: &Self) -> bool {
        debug_assert_eq!(self.id, other.id);
        self.fetched_at >= other.fetched_at
    }

    /// This observation layered over `existing`, keeping a known album artist when this
    /// observation could not see one (the JSON API never reports it) for the same album.
    pub fn merged_over(&self, existing: &Self) -> Self {
        let mut merged = self.clone();
        if merged.album_artist == Provenanced::Unknown && merged.album == existing.album {
            merged.album_artist = existing.album_artist.clone();
        }
        merged
    }

    /// Whether `other` carries the same data, ignoring when and through which source it was
    /// observed. Re-fetching an unchanged scrobble is not an update.
    pub fn same_data(&self, other: &Self) -> bool {
        let normalized = Self {
            fetched_at: other.fetched_at,
            source: other.source,
            ..self.clone()
        };
        normalized == *other
    }
}

#[cfg(test)]
#[path = "record_tests.rs"]
mod tests;
