//! In-memory storage backend for tests and ephemeral use.

use super::{AlbumCount, AppendStats, ArtistCount, ScrobbleGroup, Storage, SyncState, TrackCount};
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
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.1));
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
