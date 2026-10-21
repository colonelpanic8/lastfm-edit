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
use super::{
    AlbumCount, AppendStats, ArtistCount, EditLease, ScrobbleGroup, Storage, SyncState, TrackCount,
};
use crate::coverage::{CoverageMap, Segment};
use crate::durable::{
    append_jsonl, atomic_write, create_dir_all, parse_jsonl, read_jsonl, rewrite_locked,
};
use crate::error::Result;
use crate::id::ScrobbleId;
use crate::record::ScrobbleRecord;
use chrono::{Datelike, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
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
        create_dir_all(&root.join("scrobbles"))?;
        create_dir_all(&root.join("state"))?;
        create_dir_all(&root.join("edits"))?;
        create_dir_all(&root.join("index"))?;
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

        let folded = fold_partition(read_jsonl::<ScrobbleRecord>(path)?);
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
    fn try_lease_edits(&self) -> Result<Option<EditLease>> {
        Ok(crate::durable::try_lease(&self.root.join("edits/execution.lock"))?.map(EditLease::new))
    }

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
                let rec = match folded.get(&rec.id) {
                    None => {
                        stats.new += 1;
                        rec.clone()
                    }
                    Some(existing) => {
                        let merged = rec.merged_over(existing);
                        if !rec.supersedes(existing) || existing.same_data(&merged) {
                            stats.unchanged += 1;
                            continue;
                        }
                        stats.updated += 1;
                        merged
                    }
                };
                lines.push_str(&serde_json::to_string(&rec)?);
                lines.push('\n');
                folded.insert(rec.id.clone(), rec);
            }
            if lines.is_empty() {
                continue;
            }
            append_jsonl(&path, lines.as_bytes())?;
            cache.remove(&path);
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
        Ok(atomic_write(
            &self.state_path("coverage.json"),
            contents.as_bytes(),
        )?)
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
        Ok(atomic_write(
            &self.state_path("sync.json"),
            contents.as_bytes(),
        )?)
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
        append_jsonl(&path, lines.as_bytes())?;
        Ok(())
    }

    async fn load_edit_log(&self) -> Result<Vec<crate::edits::EditLogEntry>> {
        let path = self.root.join("edits").join("log.jsonl");
        Ok(crate::edits::fold_edit_log(read_jsonl(&path)?))
    }

    async fn compact(&self) -> Result<u64> {
        let mut cache = self.cache.lock().unwrap();
        let mut dropped = 0;
        for path in self.partition_files()? {
            rewrite_locked(&path, |bytes| {
                let records = parse_jsonl::<ScrobbleRecord>(bytes)?;
                let total = records.len() as u64;
                let folded = fold_partition(records);
                let kept = folded.len() as u64;
                if kept == total {
                    return Ok(None);
                }
                // Tombstones are needed when merging another machine's live records.
                let mut records: Vec<_> = folded.into_values().collect();
                records.sort_by(|a, b| a.uts.cmp(&b.uts).then_with(|| a.id.cmp(&b.id)));
                let mut lines = Vec::new();
                for record in records {
                    serde_json::to_writer(&mut lines, &record).map_err(std::io::Error::other)?;
                    lines.push(b'\n');
                }
                dropped += total - kept;
                Ok(Some(lines))
            })?;
            // A different writer may append as soon as rewrite_locked releases its lock.
            cache.remove(&path);
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

    async fn search_recent_scrobbles(
        &self,
        query: &str,
        before: Option<(u64, ScrobbleId)>,
        limit: usize,
    ) -> Result<Vec<ScrobbleRecord>> {
        self.with_index(|index| index.search_recent_scrobbles(query, before.as_ref(), limit))
    }

    async fn search_scrobbles(
        &self,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<ScrobbleGroup>> {
        self.with_index(|index| index.search_scrobbles(query, offset, limit))
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

fn fold_partition(records: Vec<ScrobbleRecord>) -> BTreeMap<ScrobbleId, ScrobbleRecord> {
    let mut folded: BTreeMap<ScrobbleId, ScrobbleRecord> = BTreeMap::new();
    for record in records {
        match folded.get(&record.id) {
            Some(existing) if !record.supersedes(existing) => {}
            _ => {
                folded.insert(record.id.clone(), record);
            }
        }
    }
    folded
}

// StoreError: map poisoned-lock panics through unwrap() intentionally — a poisoned lock
// means a bug already tore invariants, and continuing would risk corrupting the store.

#[cfg(test)]
#[path = "fs_tests.rs"]
mod tests;
