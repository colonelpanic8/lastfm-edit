//! In-memory storage backend for tests and ephemeral use.

use super::{AppendStats, ArtistCount, Storage, SyncState, TrackCount};
use crate::coverage::CoverageMap;
use crate::error::Result;
use crate::id::ScrobbleId;
use crate::record::ScrobbleRecord;
use std::collections::{BTreeMap, HashMap};
use std::ops::Range;
use std::sync::Mutex;

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
    entries.sort_by(|a, b| b.1.cmp(&a.1));
    entries.truncate(limit);
    entries
}

#[async_trait::async_trait]
impl Storage for MemoryStorage {
    async fn append_scrobbles(&self, records: &[ScrobbleRecord]) -> Result<AppendStats> {
        let mut inner = self.inner.lock().unwrap();
        let mut stats = AppendStats::default();
        for rec in records {
            match inner.records.get(&rec.id) {
                None => {
                    inner.records.insert(rec.id.clone(), rec.clone());
                    stats.new += 1;
                }
                Some(existing) if existing == rec || !rec.supersedes(existing) => {
                    stats.unchanged += 1;
                }
                Some(_) => {
                    inner.records.insert(rec.id.clone(), rec.clone());
                    stats.updated += 1;
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

    async fn reindex(&self) -> Result<()> {
        Ok(()) // no derived state
    }
}

#[cfg(test)]
mod tests {
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

    #[tokio::test]
    async fn append_is_idempotent_and_lww() {
        let store = MemoryStorage::new();
        let a = rec(100, "A", "x", 10);

        let stats = store.append_scrobbles(&[a.clone()]).await.unwrap();
        assert_eq!(
            stats,
            AppendStats {
                new: 1,
                updated: 0,
                unchanged: 0
            }
        );

        // Re-append: unchanged.
        let stats = store.append_scrobbles(&[a.clone()]).await.unwrap();
        assert_eq!(stats.unchanged, 1);

        // Newer observation with different content: updated.
        let mut newer = a.clone();
        newer.fetched_at = 20;
        newer.album = Some("Album".to_string());
        let stats = store.append_scrobbles(&[newer.clone()]).await.unwrap();
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
        store.append_scrobbles(&[a.clone()]).await.unwrap();
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
}
