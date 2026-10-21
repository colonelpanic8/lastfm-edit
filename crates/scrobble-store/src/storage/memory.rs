//! In-memory storage backend for tests and ephemeral use.

use super::{
    AlbumCount, AppendStats, ArtistCount, EditLease, ScrobbleGroup, Storage, SyncState, TrackCount,
};
use crate::coverage::CoverageMap;
use crate::error::Result;
use crate::id::ScrobbleId;
use crate::record::ScrobbleRecord;
use std::collections::{BTreeMap, HashMap};
use std::ops::Range;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Inner {
    records: HashMap<ScrobbleId, ScrobbleRecord>,
    coverage: CoverageMap,
    sync_state: SyncState,
    edit_events: Vec<crate::edits::EditLogEvent>,
}

/// A [`Storage`] backend holding everything in memory. Query methods are computed naively;
/// this backend exists for tests and short-lived tooling, not scale.
#[derive(Default)]
pub struct MemoryStorage {
    inner: Mutex<Inner>,
    edit_lease: Arc<tokio::sync::Mutex<()>>,
}

impl MemoryStorage {
    pub fn new() -> Self {
        Self::default()
    }
}

fn live_in_range<'a>(
    records: impl Iterator<Item = &'a ScrobbleRecord>,
    range: &Option<Range<u64>>,
) -> Vec<ScrobbleRecord> {
    let mut result: Vec<ScrobbleRecord> = records
        .filter(|rec| !rec.deleted)
        .filter(|rec| match range {
            Some(r) => r.contains(&rec.uts),
            None => true,
        })
        .cloned()
        .collect();
    result.sort_by(|a, b| a.uts.cmp(&b.uts).then_with(|| a.id.cmp(&b.id)));
    result
}

fn top_by_key<K: Ord + Clone, F: Fn(&ScrobbleRecord) -> K>(
    records: &[ScrobbleRecord],
    key: F,
    limit: usize,
) -> Vec<(K, u64)> {
    let mut counts: BTreeMap<K, u64> = BTreeMap::new();
    for rec in records {
        *counts.entry(key(rec)).or_default() += 1;
    }
    let mut entries: Vec<(K, u64)> = counts.into_iter().collect();
    // Descending by count; key order (already deterministic from BTreeMap) breaks ties.
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    entries.truncate(limit);
    entries
}

#[async_trait::async_trait]
impl Storage for MemoryStorage {
    fn try_lease_edits(&self) -> Result<Option<EditLease>> {
        Ok(self
            .edit_lease
            .clone()
            .try_lock_owned()
            .ok()
            .map(EditLease::new))
    }

    async fn append_scrobbles(&self, records: &[ScrobbleRecord]) -> Result<AppendStats> {
        let mut inner = self.inner.lock().unwrap();
        let mut stats = AppendStats::default();
        for rec in records {
            match inner.records.get(&rec.id) {
                None => {
                    inner.records.insert(rec.id.clone(), rec.clone());
                    stats.new += 1;
                }
                Some(existing) => {
                    let merged = rec.merged_over(existing);
                    if !rec.supersedes(existing) || existing.same_data(&merged) {
                        stats.unchanged += 1;
                    } else {
                        inner.records.insert(rec.id.clone(), merged);
                        stats.updated += 1;
                    }
                }
            }
        }
        Ok(stats)
    }

    async fn scrobbles_in_range(&self, range: Range<u64>) -> Result<Vec<ScrobbleRecord>> {
        let inner = self.inner.lock().unwrap();
        Ok(live_in_range(inner.records.values(), &Some(range)))
    }

    async fn get_scrobble(&self, id: &ScrobbleId) -> Result<Option<ScrobbleRecord>> {
        let inner = self.inner.lock().unwrap();
        Ok(inner.records.get(id).cloned())
    }

    async fn latest_uts(&self) -> Result<Option<u64>> {
        let inner = self.inner.lock().unwrap();
        Ok(inner
            .records
            .values()
            .filter(|rec| !rec.deleted)
            .map(|rec| rec.uts)
            .max())
    }

    async fn load_coverage(&self) -> Result<CoverageMap> {
        Ok(self.inner.lock().unwrap().coverage.clone())
    }

    async fn save_coverage(&self, coverage: &CoverageMap) -> Result<()> {
        self.inner.lock().unwrap().coverage = coverage.clone();
        Ok(())
    }

    async fn load_sync_state(&self) -> Result<SyncState> {
        Ok(self.inner.lock().unwrap().sync_state.clone())
    }

    async fn save_sync_state(&self, state: &SyncState) -> Result<()> {
        self.inner.lock().unwrap().sync_state = state.clone();
        Ok(())
    }

    async fn append_edit_events(&self, events: &[crate::edits::EditLogEvent]) -> Result<()> {
        self.inner
            .lock()
            .unwrap()
            .edit_events
            .extend_from_slice(events);
        Ok(())
    }

    async fn load_edit_log(&self) -> Result<Vec<crate::edits::EditLogEntry>> {
        let events = self.inner.lock().unwrap().edit_events.clone();
        Ok(crate::edits::fold_edit_log(events))
    }

    async fn compact(&self) -> Result<u64> {
        Ok(0) // no redundant representation to compact
    }

    async fn top_artists(
        &self,
        limit: usize,
        range: Option<Range<u64>>,
    ) -> Result<Vec<ArtistCount>> {
        let inner = self.inner.lock().unwrap();
        let live = live_in_range(inner.records.values(), &range);
        Ok(top_by_key(&live, |rec| rec.artist.clone(), limit)
            .into_iter()
            .map(|(artist, count)| ArtistCount { artist, count })
            .collect())
    }

    async fn top_tracks(
        &self,
        artist: Option<&str>,
        limit: usize,
        range: Option<Range<u64>>,
    ) -> Result<Vec<TrackCount>> {
        let inner = self.inner.lock().unwrap();
        let mut live = live_in_range(inner.records.values(), &range);
        if let Some(artist) = artist {
            live.retain(|rec| rec.artist == artist);
        }
        Ok(
            top_by_key(&live, |rec| (rec.artist.clone(), rec.track.clone()), limit)
                .into_iter()
                .map(|((artist, track), count)| TrackCount {
                    artist,
                    track,
                    count,
                })
                .collect(),
        )
    }

    async fn top_albums(
        &self,
        artist: Option<&str>,
        limit: usize,
        range: Option<Range<u64>>,
    ) -> Result<Vec<AlbumCount>> {
        let inner = self.inner.lock().unwrap();
        let mut live = live_in_range(inner.records.values(), &range);
        if let Some(artist) = artist {
            live.retain(|rec| rec.artist == artist);
        }
        live.retain(|rec| rec.album.as_deref().is_some_and(|album| !album.is_empty()));
        Ok(top_by_key(
            &live,
            |rec| (rec.artist.clone(), rec.album.clone().unwrap_or_default()),
            limit,
        )
        .into_iter()
        .map(|((artist, album), count)| AlbumCount {
            artist,
            album,
            count,
        })
        .collect())
    }

    async fn scrobble_count(&self, range: Option<Range<u64>>) -> Result<u64> {
        let inner = self.inner.lock().unwrap();
        Ok(live_in_range(inner.records.values(), &range).len() as u64)
    }

    async fn artist_scrobbles(
        &self,
        artist: &str,
        range: Option<Range<u64>>,
    ) -> Result<Vec<ScrobbleRecord>> {
        let inner = self.inner.lock().unwrap();
        let mut live = live_in_range(inner.records.values(), &range);
        live.retain(|rec| rec.artist == artist);
        Ok(live)
    }

    async fn recent_scrobbles(
        &self,
        before: Option<(u64, ScrobbleId)>,
        limit: usize,
    ) -> Result<Vec<ScrobbleRecord>> {
        let inner = self.inner.lock().unwrap();
        let mut live: Vec<ScrobbleRecord> = inner
            .records
            .values()
            .filter(|rec| !rec.deleted)
            .filter(|rec| match &before {
                Some((uts, id)) => rec.uts < *uts || (rec.uts == *uts && rec.id < *id),
                None => true,
            })
            .cloned()
            .collect();
        // Descending by (uts, id) — the reverse of the ascending order used elsewhere.
        live.sort_by(|a, b| b.uts.cmp(&a.uts).then_with(|| b.id.cmp(&a.id)));
        live.truncate(limit);
        Ok(live)
    }

    async fn search_recent_scrobbles(
        &self,
        query: &str,
        before: Option<(u64, ScrobbleId)>,
        limit: usize,
    ) -> Result<Vec<ScrobbleRecord>> {
        let terms: Vec<String> = query
            .split_whitespace()
            .map(|term| term.to_lowercase())
            .collect();
        let inner = self.inner.lock().unwrap();
        let mut live: Vec<ScrobbleRecord> = inner
            .records
            .values()
            .filter(|rec| !rec.deleted)
            .filter(|rec| match &before {
                Some((uts, id)) => rec.uts < *uts || (rec.uts == *uts && rec.id < *id),
                None => true,
            })
            .filter(|rec| {
                let fields = [
                    rec.artist.to_lowercase(),
                    rec.track.to_lowercase(),
                    rec.album.as_deref().unwrap_or_default().to_lowercase(),
                ];
                terms
                    .iter()
                    .all(|term| fields.iter().any(|field| field.contains(term)))
            })
            .cloned()
            .collect();
        live.sort_by(|a, b| b.uts.cmp(&a.uts).then_with(|| b.id.cmp(&a.id)));
        live.truncate(limit);
        Ok(live)
    }

    async fn search_scrobbles(
        &self,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<ScrobbleGroup>> {
        let terms: Vec<String> = query
            .split_whitespace()
            .map(|term| term.to_lowercase())
            .collect();
        let inner = self.inner.lock().unwrap();
        let mut groups: std::collections::BTreeMap<
            (String, String, Option<String>),
            ScrobbleGroup,
        > = std::collections::BTreeMap::new();
        for rec in inner.records.values().filter(|rec| !rec.deleted) {
            let fields = [
                rec.artist.to_lowercase(),
                rec.track.to_lowercase(),
                rec.album.as_deref().unwrap_or_default().to_lowercase(),
            ];
            if !terms
                .iter()
                .all(|term| fields.iter().any(|field| field.contains(term)))
            {
                continue;
            }
            let key = (rec.artist.clone(), rec.track.clone(), rec.album.clone());
            groups
                .entry(key)
                .and_modify(|group| {
                    group.count += 1;
                    group.first_uts = group.first_uts.min(rec.uts);
                    group.latest_uts = group.latest_uts.max(rec.uts);
                })
                .or_insert_with(|| ScrobbleGroup {
                    artist: rec.artist.clone(),
                    track: rec.track.clone(),
                    album: rec.album.clone(),
                    count: 1,
                    first_uts: rec.uts,
                    latest_uts: rec.uts,
                });
        }
        let mut groups: Vec<_> = groups.into_values().collect();
        groups.sort_by(|a, b| {
            b.latest_uts
                .cmp(&a.latest_uts)
                .then_with(|| a.artist.to_lowercase().cmp(&b.artist.to_lowercase()))
                .then_with(|| a.track.to_lowercase().cmp(&b.track.to_lowercase()))
                .then_with(|| a.album.cmp(&b.album))
        });
        Ok(groups.into_iter().skip(offset).take(limit).collect())
    }

    async fn reindex(&self) -> Result<()> {
        Ok(()) // no derived state
    }
}

#[cfg(test)]
#[path = "memory_tests.rs"]
mod tests;
