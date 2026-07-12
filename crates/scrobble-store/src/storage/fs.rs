//! Flat-file storage: append-only JSONL in a git-friendly directory tree.
//!
//! Layout (relative to the store root):
//! ```text
//! scrobbles/2024/2024-01.jsonl   # one file per UTC year-month, append-only, LWW by id
//! state/coverage.json            # coverage map, atomically rewritten
//! state/sync.json                # sync engine state, atomically rewritten
//! index/cache.db                 # derived SQLite index — gitignored, disposable
//! .gitattributes                 # *.jsonl merge=union so concurrent machines merge cleanly
//! .gitignore                     # ignores index/
//! ```
//!
//! The JSONL files are the source of truth. Records are idempotent, deduplicated by id with
//! last-write-wins on `fetched_at`, so a git `merge=union` of two machines' appends —
//! including duplicated or interleaved lines — folds back into the same state.

use super::index::Index;
use super::{AlbumCount, AppendStats, ArtistCount, Storage, SyncState, TrackCount};
use crate::coverage::{CoverageMap, Segment};
use crate::error::Result;
use crate::id::ScrobbleId;
use crate::record::ScrobbleRecord;
use chrono::{Datelike, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

const GITATTRIBUTES: &str = "*.jsonl merge=union\n";
const GITIGNORE: &str = "/index/\n";

/// Folded (LWW) view of one partition file, cache-validated by size+mtime.
struct PartitionCache {
    size: u64,
    mtime: SystemTime,
    folded: BTreeMap<ScrobbleId, ScrobbleRecord>,
}

/// Versioned wrapper for state files, so future readers can detect old shapes.
#[derive(Serialize, Deserialize)]
struct StateFile<T> {
    v: u32,
    #[serde(flatten)]
    inner: T,
}

#[derive(Serialize, Deserialize)]
struct CoverageBody {
    segments: Vec<Segment>,
}

/// The flat-file [`Storage`] backend. See module docs for the layout.
pub struct FsStorage {
    root: PathBuf,
    cache: Mutex<HashMap<PathBuf, PartitionCache>>,
    index: Mutex<Option<Index>>,
}

impl FsStorage {
    /// Open a store rooted at `root`, creating the directory skeleton (and the git glue
    /// files, if absent) as needed. Safe to call on an existing store.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(root.join("scrobbles"))?;
        std::fs::create_dir_all(root.join("state"))?;
        std::fs::create_dir_all(root.join("edits"))?;
        std::fs::create_dir_all(root.join("index"))?;
        ensure_file_contains(&root.join(".gitattributes"), GITATTRIBUTES)?;
        ensure_file_contains(&root.join(".gitignore"), GITIGNORE)?;
        Ok(Self {
            root,
            cache: Mutex::new(HashMap::new()),
            index: Mutex::new(None),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn partition_path(&self, uts: u64) -> PathBuf {
        let dt = Utc
            .timestamp_opt(uts as i64, 0)
            .single()
            .unwrap_or_else(|| Utc.timestamp_opt(0, 0).single().unwrap());
        self.root
            .join("scrobbles")
            .join(format!("{:04}", dt.year()))
            .join(format!("{:04}-{:02}.jsonl", dt.year(), dt.month()))
    }

    /// All partition files, sorted ascending (which is chronological, given the naming).
    fn partition_files(&self) -> Result<Vec<PathBuf>> {
        let mut files = Vec::new();
        let scrobbles = self.root.join("scrobbles");
        for year_entry in std::fs::read_dir(&scrobbles)? {
            let year_dir = year_entry?.path();
            if !year_dir.is_dir() {
                continue;
            }
            for entry in std::fs::read_dir(&year_dir)? {
                let path = entry?.path();
                if path.extension().is_some_and(|ext| ext == "jsonl") {
                    files.push(path);
                }
            }
        }
        files.sort();
        Ok(files)
    }

    /// UTC month bounds `[start, end)` for a partition file, parsed from its name.
    fn partition_bounds(path: &Path) -> Option<Range<u64>> {
        let stem = path.file_stem()?.to_str()?;
        let (year, month) = stem.split_once('-')?;
        let (year, month): (i32, u32) = (year.parse().ok()?, month.parse().ok()?);
        let start = Utc.with_ymd_and_hms(year, month, 1, 0, 0, 0).single()?;
        let (next_year, next_month) = if month == 12 {
            (year + 1, 1)
        } else {
            (year, month + 1)
        };
        let end = Utc
            .with_ymd_and_hms(next_year, next_month, 1, 0, 0, 0)
            .single()?;
        Some(start.timestamp() as u64..end.timestamp() as u64)
    }

    /// Load the folded LWW view of a partition, via the cache when the file is unchanged.
    fn load_partition(
        &self,
        cache: &mut HashMap<PathBuf, PartitionCache>,
        path: &Path,
    ) -> Result<BTreeMap<ScrobbleId, ScrobbleRecord>> {
        let meta = match std::fs::metadata(path) {
            Ok(meta) => meta,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
            Err(err) => return Err(err.into()),
        };
        let (size, mtime) = (meta.len(), meta.modified()?);
        if let Some(cached) = cache.get(path) {
            if cached.size == size && cached.mtime == mtime {
                return Ok(cached.folded.clone());
            }
        }

        let content = std::fs::read_to_string(path)?;
        let mut folded: BTreeMap<ScrobbleId, ScrobbleRecord> = BTreeMap::new();
        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<ScrobbleRecord>(line) {
                Ok(rec) => {
                    match folded.get(&rec.id) {
                        Some(existing) if !rec.supersedes(existing) => {}
                        _ => {
                            folded.insert(rec.id.clone(), rec);
                        }
                    };
                }
                Err(err) => {
                    // Torn tail from a crash mid-append, or a hand-mangled line: skip it.
                    // The record it would have carried is re-fetched by the next sync
                    // (coverage is only persisted after a successful append).
                    log::warn!("skipping unparseable line in {}: {err}", path.display());
                }
            }
        }
        cache.insert(
            path.to_path_buf(),
            PartitionCache {
                size,
                mtime,
                folded: folded.clone(),
            },
        );
        Ok(folded)
    }

    fn state_path(&self, name: &str) -> PathBuf {
        self.root.join("state").join(name)
    }

    fn with_index<T>(&self, f: impl FnOnce(&mut Index) -> Result<T>) -> Result<T> {
        let mut guard = self.index.lock().unwrap();
        if guard.is_none() {
            *guard = Some(Index::open(&self.root.join("index").join("cache.db"))?);
        }
        let index = guard.as_mut().expect("just initialized");
        index.catch_up(&self.partition_files()?)?;
        f(index)
    }
}

/// Write a file atomically: temp sibling + rename.
fn atomic_write(path: &Path, contents: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(contents)?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Create a file with `contents` if it doesn't exist; leave existing files alone (the user
/// may have customized them).
fn ensure_file_contains(path: &Path, contents: &str) -> Result<()> {
    if !path.exists() {
        atomic_write(path, contents.as_bytes())?;
    }
    Ok(())
}

#[async_trait::async_trait]
impl Storage for FsStorage {
    async fn append_scrobbles(&self, records: &[ScrobbleRecord]) -> Result<AppendStats> {
        let mut by_partition: BTreeMap<PathBuf, Vec<&ScrobbleRecord>> = BTreeMap::new();
        for rec in records {
            by_partition
                .entry(self.partition_path(rec.uts))
                .or_default()
                .push(rec);
        }

        let mut cache = self.cache.lock().unwrap();
        let mut stats = AppendStats::default();
        for (path, batch) in by_partition {
            let mut folded = self.load_partition(&mut cache, &path)?;
            let mut lines = String::new();
            for rec in batch {
                match folded.get(&rec.id) {
                    None => stats.new += 1,
                    Some(existing) if existing == rec || !rec.supersedes(existing) => {
                        stats.unchanged += 1;
                        continue;
                    }
                    Some(_) => stats.updated += 1,
                }
                lines.push_str(&serde_json::to_string(rec)?);
                lines.push('\n');
                folded.insert(rec.id.clone(), rec.clone());
            }
            if lines.is_empty() {
                continue;
            }
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)?;
            file.write_all(lines.as_bytes())?;
            file.sync_all()?;
            let meta = std::fs::metadata(&path)?;
            cache.insert(
                path,
                PartitionCache {
                    size: meta.len(),
                    mtime: meta.modified()?,
                    folded,
                },
            );
        }
        Ok(stats)
    }

    async fn scrobbles_in_range(&self, range: Range<u64>) -> Result<Vec<ScrobbleRecord>> {
        let mut cache = self.cache.lock().unwrap();
        let mut result = Vec::new();
        for path in self.partition_files()? {
            if let Some(bounds) = Self::partition_bounds(&path) {
                if bounds.end <= range.start || bounds.start >= range.end {
                    continue;
                }
            }
            let folded = self.load_partition(&mut cache, &path)?;
            result.extend(
                folded
                    .into_values()
                    .filter(|rec| !rec.deleted && range.contains(&rec.uts)),
            );
        }
        result.sort_by(|a, b| a.uts.cmp(&b.uts).then_with(|| a.id.cmp(&b.id)));
        Ok(result)
    }

    async fn get_scrobble(&self, id: &ScrobbleId) -> Result<Option<ScrobbleRecord>> {
        let mut cache = self.cache.lock().unwrap();
        let path = self.partition_path(id.uts());
        let folded = self.load_partition(&mut cache, &path)?;
        Ok(folded.get(id).cloned())
    }

    async fn latest_uts(&self) -> Result<Option<u64>> {
        let mut cache = self.cache.lock().unwrap();
        for path in self.partition_files()?.into_iter().rev() {
            let folded = self.load_partition(&mut cache, &path)?;
            let latest = folded
                .values()
                .filter(|rec| !rec.deleted)
                .map(|rec| rec.uts)
                .max();
            if latest.is_some() {
                return Ok(latest);
            }
        }
        Ok(None)
    }

    async fn load_coverage(&self) -> Result<CoverageMap> {
        let path = self.state_path("coverage.json");
        if !path.exists() {
            return Ok(CoverageMap::new());
        }
        let content = std::fs::read_to_string(&path)?;
        let file: StateFile<CoverageBody> = serde_json::from_str(&content)?;
        // Re-normalizing on load makes union-merged (possibly overlapping) segment lists
        // from different machines collapse back into a valid map.
        Ok(CoverageMap::from_segments(file.inner.segments))
    }

    async fn save_coverage(&self, coverage: &CoverageMap) -> Result<()> {
        let file = StateFile {
            v: 1,
            inner: CoverageBody {
                segments: coverage.segments().to_vec(),
            },
        };
        let mut contents = serde_json::to_string_pretty(&file)?;
        contents.push('\n');
        atomic_write(&self.state_path("coverage.json"), contents.as_bytes())
    }

    async fn load_sync_state(&self) -> Result<SyncState> {
        let path = self.state_path("sync.json");
        if !path.exists() {
            return Ok(SyncState::default());
        }
        let content = std::fs::read_to_string(&path)?;
        let file: StateFile<SyncState> = serde_json::from_str(&content)?;
        Ok(file.inner)
    }

    async fn save_sync_state(&self, state: &SyncState) -> Result<()> {
        let file = StateFile {
            v: 1,
            inner: state.clone(),
        };
        let mut contents = serde_json::to_string_pretty(&file)?;
        contents.push('\n');
        atomic_write(&self.state_path("sync.json"), contents.as_bytes())
    }

    async fn append_edit_events(&self, events: &[crate::edits::EditLogEvent]) -> Result<()> {
        if events.is_empty() {
            return Ok(());
        }
        let mut lines = String::new();
        for event in events {
            lines.push_str(&serde_json::to_string(event)?);
            lines.push('\n');
        }
        let path = self.root.join("edits").join("log.jsonl");
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        file.write_all(lines.as_bytes())?;
        file.sync_all()?;
        Ok(())
    }

    async fn load_edit_log(&self) -> Result<Vec<crate::edits::EditLogEntry>> {
        let path = self.root.join("edits").join("log.jsonl");
        if !path.exists() {
            return Ok(Vec::new());
        }
        let content = std::fs::read_to_string(&path)?;
        let mut events = Vec::new();
        for line in content.lines().filter(|l| !l.trim().is_empty()) {
            match serde_json::from_str(line) {
                Ok(event) => events.push(event),
                Err(err) => {
                    // Same torn-tail tolerance as scrobble partitions.
                    log::warn!("skipping unparseable edit-log line: {err}");
                }
            }
        }
        Ok(crate::edits::fold_edit_log(events))
    }

    async fn compact(&self) -> Result<u64> {
        let mut cache = self.cache.lock().unwrap();
        let mut dropped = 0;
        for path in self.partition_files()? {
            let content = std::fs::read_to_string(&path)?;
            let total_lines = content.lines().filter(|l| !l.trim().is_empty()).count() as u64;
            let folded = self.load_partition(&mut cache, &path)?;
            let kept = folded.len() as u64;
            if kept == total_lines {
                continue;
            }
            // Keep tombstones: they are load-bearing when another machine's un-merged
            // files still carry the live record.
            let mut records: Vec<&ScrobbleRecord> = folded.values().collect();
            records.sort_by(|a, b| a.uts.cmp(&b.uts).then_with(|| a.id.cmp(&b.id)));
            let mut lines = String::new();
            for rec in records {
                lines.push_str(&serde_json::to_string(rec)?);
                lines.push('\n');
            }
            atomic_write(&path, lines.as_bytes())?;
            let meta = std::fs::metadata(&path)?;
            cache.insert(
                path,
                PartitionCache {
                    size: meta.len(),
                    mtime: meta.modified()?,
                    folded,
                },
            );
            dropped += total_lines - kept;
        }
        Ok(dropped)
    }

    async fn top_artists(
        &self,
        limit: usize,
        range: Option<Range<u64>>,
    ) -> Result<Vec<ArtistCount>> {
        self.with_index(|index| index.top_artists(limit, &range))
    }

    async fn top_tracks(
        &self,
        artist: Option<&str>,
        limit: usize,
        range: Option<Range<u64>>,
    ) -> Result<Vec<TrackCount>> {
        self.with_index(|index| index.top_tracks(artist, limit, &range))
    }

    async fn top_albums(
        &self,
        artist: Option<&str>,
        limit: usize,
        range: Option<Range<u64>>,
    ) -> Result<Vec<AlbumCount>> {
        self.with_index(|index| index.top_albums(artist, limit, &range))
    }

    async fn scrobble_count(&self, range: Option<Range<u64>>) -> Result<u64> {
        self.with_index(|index| index.scrobble_count(&range))
    }

    async fn artist_scrobbles(
        &self,
        artist: &str,
        range: Option<Range<u64>>,
    ) -> Result<Vec<ScrobbleRecord>> {
        self.with_index(|index| index.artist_scrobbles(artist, &range))
    }

    async fn recent_scrobbles(
        &self,
        before: Option<(u64, ScrobbleId)>,
        limit: usize,
    ) -> Result<Vec<ScrobbleRecord>> {
        self.with_index(|index| index.recent_scrobbles(before.as_ref(), limit))
    }

    async fn reindex(&self) -> Result<()> {
        let mut guard = self.index.lock().unwrap();
        if let Some(index) = guard.take() {
            index.destroy()?;
        }
        let db_path = self.root.join("index").join("cache.db");
        if db_path.exists() {
            std::fs::remove_file(&db_path)?;
        }
        let mut index = Index::open(&db_path)?;
        index.rebuild(&self.partition_files()?)?;
        *guard = Some(index);
        Ok(())
    }
}

// StoreError: map poisoned-lock panics through unwrap() intentionally — a poisoned lock
// means a bug already tore invariants, and continuing would risk corrupting the store.

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
            album: Some("Album".to_string()),
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

    // 2024-01-15 and 2024-02-15 (UTC), so records land in different partitions.
    const JAN: u64 = 1_705_312_800;
    const FEB: u64 = 1_707_991_200;

    #[tokio::test]
    async fn round_trip_and_partitioning() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsStorage::open(dir.path()).unwrap();

        let stats = store
            .append_scrobbles(&[rec(JAN, "A", "x", 1), rec(FEB, "B", "y", 1)])
            .await
            .unwrap();
        assert_eq!(stats.new, 2);

        assert!(dir.path().join("scrobbles/2024/2024-01.jsonl").exists());
        assert!(dir.path().join("scrobbles/2024/2024-02.jsonl").exists());
        assert!(dir.path().join(".gitattributes").exists());

        let all = store.scrobbles_in_range(0..u64::MAX).await.unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].artist, "A");
        assert_eq!(store.latest_uts().await.unwrap(), Some(FEB));

        // Fresh handle re-reads from disk identically.
        let store2 = FsStorage::open(dir.path()).unwrap();
        let all2 = store2.scrobbles_in_range(0..u64::MAX).await.unwrap();
        assert_eq!(all, all2);
    }

    #[tokio::test]
    async fn lww_append_and_compaction() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsStorage::open(dir.path()).unwrap();

        let a = rec(JAN, "A", "x", 1);
        store.append_scrobbles(&[a.clone()]).await.unwrap();
        // Idempotent re-append writes nothing.
        let stats = store.append_scrobbles(&[a.clone()]).await.unwrap();
        assert_eq!(stats.unchanged, 1);

        let mut newer = a.clone();
        newer.fetched_at = 2;
        newer.album = Some("Corrected".to_string());
        let stats = store.append_scrobbles(&[newer.clone()]).await.unwrap();
        assert_eq!(stats.updated, 1);

        // Two lines on disk, one logical record.
        let path = dir.path().join("scrobbles/2024/2024-01.jsonl");
        assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 2);
        let got = store.get_scrobble(&a.id).await.unwrap().unwrap();
        assert_eq!(got.album.as_deref(), Some("Corrected"));

        let dropped = store.compact().await.unwrap();
        assert_eq!(dropped, 1);
        assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 1);
        let got = store.get_scrobble(&a.id).await.unwrap().unwrap();
        assert_eq!(got.album.as_deref(), Some("Corrected"));
    }

    #[tokio::test]
    async fn tombstones_survive_compaction_and_hide_from_reads() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsStorage::open(dir.path()).unwrap();
        let a = rec(JAN, "A", "x", 1);
        store.append_scrobbles(&[a.clone()]).await.unwrap();
        store
            .append_scrobbles(&[a.clone().into_tombstone(2)])
            .await
            .unwrap();
        store.compact().await.unwrap();

        assert!(store
            .scrobbles_in_range(0..u64::MAX)
            .await
            .unwrap()
            .is_empty());
        let got = store.get_scrobble(&a.id).await.unwrap().unwrap();
        assert!(got.deleted);
    }

    #[tokio::test]
    async fn torn_final_line_is_tolerated() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsStorage::open(dir.path()).unwrap();
        store
            .append_scrobbles(&[rec(JAN, "A", "x", 1)])
            .await
            .unwrap();

        // Simulate a crash mid-append: valid line + torn line.
        let path = dir.path().join("scrobbles/2024/2024-01.jsonl");
        let mut content = std::fs::read_to_string(&path).unwrap();
        content.push_str("{\"id\":\"170531");
        std::fs::write(&path, content).unwrap();

        let store2 = FsStorage::open(dir.path()).unwrap();
        let all = store2.scrobbles_in_range(0..u64::MAX).await.unwrap();
        assert_eq!(all.len(), 1);
    }

    #[tokio::test]
    async fn coverage_and_sync_state_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsStorage::open(dir.path()).unwrap();

        assert!(store.load_coverage().await.unwrap().is_empty());
        let mut cov = CoverageMap::new();
        cov.insert(Segment::new(10, 20, 5));
        cov.insert(Segment::new(30, 40, 6));
        store.save_coverage(&cov).await.unwrap();
        assert_eq!(store.load_coverage().await.unwrap(), cov);

        let state = SyncState {
            history_start_uts: Some(123),
            last_sync_at: Some(456),
        };
        store.save_sync_state(&state).await.unwrap();
        assert_eq!(store.load_sync_state().await.unwrap(), state);
    }

    #[tokio::test]
    async fn union_merged_coverage_normalizes_on_load() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsStorage::open(dir.path()).unwrap();
        // Simulate the aftermath of a git conflict resolved by unioning segment lists:
        // overlapping, out-of-order segments.
        let raw = r#"{"v":1,"segments":[
            {"start":30,"end":50,"verified_at":2},
            {"start":10,"end":35,"verified_at":1}
        ]}"#;
        std::fs::write(dir.path().join("state/coverage.json"), raw).unwrap();
        let cov = store.load_coverage().await.unwrap();
        assert_eq!(cov.segments().len(), 1);
        assert_eq!(cov.segments()[0].range(), 10..50);
    }

    #[tokio::test]
    async fn indexed_queries_and_incremental_ingest() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsStorage::open(dir.path()).unwrap();
        store
            .append_scrobbles(&[
                rec(JAN, "A", "x", 1),
                rec(JAN + 60, "A", "x", 1),
                rec(JAN + 120, "A", "y", 1),
                rec(FEB, "B", "z", 1),
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

        // Appends after the first query are picked up incrementally.
        store
            .append_scrobbles(&[rec(FEB + 60, "B", "z", 1), rec(FEB + 120, "B", "w", 1)])
            .await
            .unwrap();
        let artists = store.top_artists(10, None).await.unwrap();
        assert_eq!(artists[0].count, 3);
        assert_eq!(
            artists[1],
            ArtistCount {
                artist: "B".into(),
                count: 3
            }
        );

        let tracks = store.top_tracks(Some("B"), 10, None).await.unwrap();
        assert_eq!(tracks[0].track, "z");
        assert_eq!(tracks[0].count, 2);

        let scrobbles = store.artist_scrobbles("A", None).await.unwrap();
        assert_eq!(scrobbles.len(), 3);
        assert!(scrobbles.windows(2).all(|w| w[0].uts <= w[1].uts));

        // A tombstone updates the aggregates (via LWW upsert in the index).
        let dead = rec(JAN, "A", "x", 1).into_tombstone(9);
        store.append_scrobbles(&[dead]).await.unwrap();
        let artists = store.top_artists(10, None).await.unwrap();
        assert_eq!(artists.iter().find(|a| a.artist == "A").unwrap().count, 2);
    }

    #[tokio::test]
    async fn top_albums_and_scrobble_count() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsStorage::open(dir.path()).unwrap();
        store
            .append_scrobbles(&[
                rec_album(JAN, "A", "x", Some("One")),
                rec_album(JAN + 60, "A", "y", Some("One")),
                rec_album(JAN + 120, "A", "z", Some("Two")),
                rec_album(FEB, "B", "w", Some("Three")),
                rec_album(FEB + 60, "A", "n", None),
                rec_album(FEB + 120, "A", "e", Some("")),
            ])
            .await
            .unwrap();

        let albums = store.top_albums(None, 10, None).await.unwrap();
        assert_eq!(
            albums[0],
            AlbumCount {
                artist: "A".into(),
                album: "One".into(),
                count: 2
            }
        );
        // None and empty albums are excluded.
        assert_eq!(albums.len(), 3);

        let a_albums = store.top_albums(Some("A"), 10, None).await.unwrap();
        assert_eq!(a_albums.len(), 2);
        assert!(a_albums.iter().all(|a| a.artist == "A"));

        assert_eq!(store.scrobble_count(None).await.unwrap(), 6);
        assert_eq!(store.scrobble_count(Some(JAN..JAN + 121)).await.unwrap(), 3);

        // Tombstones drop out of both aggregates.
        store
            .append_scrobbles(&[rec_album(JAN, "A", "x", Some("One")).into_tombstone(9)])
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
        let dir = tempfile::tempdir().unwrap();
        let store = FsStorage::open(dir.path()).unwrap();
        // Two records share JAN (different track → different id); the rest are spread out.
        store
            .append_scrobbles(&[
                rec(JAN, "A", "x", 1),
                rec(JAN, "A", "y", 1),
                rec(JAN + 60, "A", "z", 1),
                rec(FEB, "B", "w", 1),
                rec(FEB + 60, "B", "v", 1),
            ])
            .await
            .unwrap();

        // Newest-first, no cursor.
        let p1 = store.recent_scrobbles(None, 2).await.unwrap();
        assert_eq!(p1.len(), 2);
        assert_eq!(p1[0].uts, FEB + 60);
        assert_eq!(p1[1].uts, FEB);

        // Walk every page with the keyset cursor; expect no gaps or duplicates.
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
        assert_eq!(all.len(), 5);
        let ids: std::collections::HashSet<_> = all.iter().map(|r| r.id.clone()).collect();
        assert_eq!(ids.len(), 5, "no duplicate records across pages");
        // Fully descending, including the same-second id tiebreak.
        assert!(all
            .windows(2)
            .all(|w| (w[0].uts, w[0].id.as_str()) >= (w[1].uts, w[1].id.as_str())));
        // The two JAN records are adjacent at the tail (oldest), both present.
        assert_eq!(all[3].uts, JAN);
        assert_eq!(all[4].uts, JAN);

        // Tombstones are excluded.
        store
            .append_scrobbles(&[rec(FEB + 60, "B", "v", 1).into_tombstone(9)])
            .await
            .unwrap();
        let after = store.recent_scrobbles(None, 10).await.unwrap();
        assert_eq!(after.len(), 4);
        assert!(after.iter().all(|r| r.uts != FEB + 60));

        // Records appended after an earlier query are picked up (index catch_up).
        store
            .append_scrobbles(&[rec(FEB + 120, "B", "n", 1)])
            .await
            .unwrap();
        let fresh = store.recent_scrobbles(None, 1).await.unwrap();
        assert_eq!(fresh[0].uts, FEB + 120);
    }

    #[tokio::test]
    async fn compaction_triggers_index_rebuild_and_reindex_works() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsStorage::open(dir.path()).unwrap();
        let a = rec(JAN, "A", "x", 1);
        let mut newer = a.clone();
        newer.fetched_at = 2;
        store.append_scrobbles(&[a]).await.unwrap();
        store.append_scrobbles(&[newer]).await.unwrap();

        // Prime the index, then compact (file shrinks), then query again.
        assert_eq!(store.top_artists(10, None).await.unwrap()[0].count, 1);
        store.compact().await.unwrap();
        assert_eq!(store.top_artists(10, None).await.unwrap()[0].count, 1);

        // Explicit reindex from scratch.
        store.reindex().await.unwrap();
        assert_eq!(store.top_artists(10, None).await.unwrap()[0].count, 1);

        // A fresh handle (fresh index connection) sees the same state.
        let store2 = FsStorage::open(dir.path()).unwrap();
        assert_eq!(store2.top_artists(10, None).await.unwrap()[0].count, 1);
    }
}
