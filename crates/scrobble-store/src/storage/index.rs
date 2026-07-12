//! Derived SQLite index over the JSONL source of truth.
//!
//! This database is disposable local state: it lives in the gitignored `index/` directory,
//! is fed incrementally from per-file byte offsets, and is **never migrated** — any schema
//! mismatch, missing file, or inconsistency is handled by deleting it and rebuilding from
//! the flat files. Nothing in it is authoritative.

use crate::error::{Result, StoreError};
use crate::id::ScrobbleId;
use crate::record::ScrobbleRecord;
use crate::storage::{AlbumCount, ArtistCount, TrackCount};
use rusqlite::{params, Connection, OptionalExtension};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

const SCHEMA_VERSION: i64 = 2;

pub(crate) struct Index {
    conn: Connection,
    db_path: PathBuf,
}

/// Stat data used to detect whether a partition file changed since last ingest.
struct FileStamp {
    size: u64,
    mtime_ns: i128,
}

fn stamp(path: &Path) -> Result<FileStamp> {
    let meta = std::fs::metadata(path)?;
    let mtime_ns = meta
        .modified()?
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as i128)
        .unwrap_or(0);
    Ok(FileStamp {
        size: meta.len(),
        mtime_ns,
    })
}

impl Index {
    /// Open (or create) the index database. If the schema version doesn't match, the file is
    /// deleted and recreated — rebuild, never migrate.
    pub(crate) fn open(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        match Self::try_open(db_path) {
            Ok(index) => Ok(index),
            Err(err) => {
                log::warn!(
                    "index at {} unusable ({err}); deleting and rebuilding",
                    db_path.display()
                );
                let _ = std::fs::remove_file(db_path);
                Self::try_open(db_path)
            }
        }
    }

    fn try_open(db_path: &Path) -> Result<Self> {
        let conn = Connection::open(db_path).map_err(sqlite_err)?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(sqlite_err)?;
        let version: Option<i64> = conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .optional()
            .unwrap_or(None);
        match version {
            Some(v) if v == SCHEMA_VERSION => {}
            Some(v) => {
                return Err(StoreError::Corrupt(format!(
                    "index schema version {v}, expected {SCHEMA_VERSION}"
                )))
            }
            None => {
                conn.execute_batch(
                    "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value INTEGER);
                     CREATE TABLE IF NOT EXISTS ingested_files (
                         path TEXT PRIMARY KEY,
                         byte_offset INTEGER NOT NULL,
                         size INTEGER NOT NULL,
                         mtime_ns TEXT NOT NULL
                     );
                     CREATE TABLE IF NOT EXISTS scrobbles (
                         id TEXT PRIMARY KEY,
                         uts INTEGER NOT NULL,
                         artist TEXT NOT NULL,
                         track TEXT NOT NULL,
                         album TEXT,
                         fetched_at INTEGER NOT NULL,
                         deleted INTEGER NOT NULL,
                         json TEXT NOT NULL
                     );
                     CREATE INDEX IF NOT EXISTS idx_scrobbles_uts ON scrobbles(uts);
                     CREATE INDEX IF NOT EXISTS idx_scrobbles_artist ON scrobbles(artist, uts);",
                )
                .map_err(sqlite_err)?;
                conn.execute(
                    "INSERT INTO meta (key, value) VALUES ('schema_version', ?1)",
                    params![SCHEMA_VERSION],
                )
                .map_err(sqlite_err)?;
            }
        }
        Ok(Self {
            conn,
            db_path: db_path.to_path_buf(),
        })
    }

    /// Bring the index up to date with the given partition files.
    ///
    /// Appended tails are ingested incrementally from the recorded byte offset. Anything
    /// that isn't a pure append (file shrank, rewritten with different mtime at or below
    /// the recorded offset, file deleted) triggers a full rebuild — cheap enough, and
    /// guaranteed correct.
    pub(crate) fn catch_up(&mut self, partition_files: &[PathBuf]) -> Result<()> {
        if self.needs_rebuild(partition_files)? {
            log::info!("index: partition files changed non-incrementally; rebuilding");
            self.rebuild(partition_files)?;
            return Ok(());
        }
        for path in partition_files {
            let stamp = stamp(path)?;
            let offset: Option<u64> = self
                .conn
                .query_row(
                    "SELECT byte_offset FROM ingested_files WHERE path = ?1",
                    params![path_key(path)],
                    |row| row.get::<_, i64>(0).map(|v| v as u64),
                )
                .optional()
                .map_err(sqlite_err)?;
            let offset = offset.unwrap_or(0);
            if stamp.size > offset {
                self.ingest_from(path, offset, &stamp)?;
            }
        }
        Ok(())
    }

    fn needs_rebuild(&self, partition_files: &[PathBuf]) -> Result<bool> {
        // Recorded files that vanished or shrank mean history was rewritten (compaction,
        // git operations); incremental ingest can't recover from that.
        let mut statement = self
            .conn
            .prepare("SELECT path, byte_offset FROM ingested_files")
            .map_err(sqlite_err)?;
        let recorded: Vec<(String, u64)> = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64))
            })
            .map_err(sqlite_err)?
            .collect::<std::result::Result<_, _>>()
            .map_err(sqlite_err)?;

        let current: std::collections::HashMap<String, u64> = partition_files
            .iter()
            .map(|path| stamp(path).map(|stamp| (path_key(path), stamp.size)))
            .collect::<Result<_>>()?;

        for (path, offset) in recorded {
            match current.get(&path) {
                None => return Ok(true),
                Some(size) if *size < offset => return Ok(true),
                Some(_) => {}
            }
        }
        Ok(false)
    }

    /// Drop all derived rows and re-ingest everything.
    pub(crate) fn rebuild(&mut self, partition_files: &[PathBuf]) -> Result<()> {
        let tx = self.conn.transaction().map_err(sqlite_err)?;
        tx.execute("DELETE FROM scrobbles", [])
            .map_err(sqlite_err)?;
        tx.execute("DELETE FROM ingested_files", [])
            .map_err(sqlite_err)?;
        tx.commit().map_err(sqlite_err)?;
        for path in partition_files {
            let stamp = stamp(path)?;
            self.ingest_from(path, 0, &stamp)?;
        }
        Ok(())
    }

    fn ingest_from(&mut self, path: &Path, offset: u64, stamp: &FileStamp) -> Result<()> {
        use std::io::{BufRead, BufReader, Seek, SeekFrom};

        let mut file = std::fs::File::open(path)?;
        file.seek(SeekFrom::Start(offset))?;
        let reader = BufReader::new(file);

        let tx = self.conn.transaction().map_err(sqlite_err)?;
        let mut ingested_bytes = offset;
        {
            let mut upsert = tx
                .prepare(
                    "INSERT INTO scrobbles (id, uts, artist, track, album, fetched_at, deleted, json)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                     ON CONFLICT(id) DO UPDATE SET
                         uts = excluded.uts,
                         artist = excluded.artist,
                         track = excluded.track,
                         album = excluded.album,
                         fetched_at = excluded.fetched_at,
                         deleted = excluded.deleted,
                         json = excluded.json
                     WHERE excluded.fetched_at >= scrobbles.fetched_at",
                )
                .map_err(sqlite_err)?;
            let lines: Vec<String> = reader.lines().collect::<std::io::Result<_>>()?;
            for (i, line) in lines.iter().enumerate() {
                let line_bytes = line.len() as u64 + 1; // newline
                match serde_json::from_str::<ScrobbleRecord>(line) {
                    Ok(rec) => {
                        upsert
                            .execute(params![
                                rec.id.as_str(),
                                rec.uts as i64,
                                rec.artist,
                                rec.track,
                                rec.album,
                                rec.fetched_at as i64,
                                rec.deleted as i64,
                                line,
                            ])
                            .map_err(sqlite_err)?;
                        ingested_bytes += line_bytes;
                    }
                    Err(err) if i + 1 == lines.len() => {
                        // A torn final line stays un-ingested (offset not advanced past it),
                        // so a later completed write picks it up.
                        log::warn!(
                            "index: leaving torn final line of {} for a later pass: {err}",
                            path.display()
                        );
                    }
                    Err(err) => {
                        log::warn!(
                            "index: skipping unparseable line in {}: {err}",
                            path.display()
                        );
                        ingested_bytes += line_bytes;
                    }
                }
            }
            drop(upsert);
        }
        tx.execute(
            "INSERT INTO ingested_files (path, byte_offset, size, mtime_ns)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(path) DO UPDATE SET
                 byte_offset = excluded.byte_offset,
                 size = excluded.size,
                 mtime_ns = excluded.mtime_ns",
            params![
                path_key(path),
                ingested_bytes as i64,
                stamp.size as i64,
                stamp.mtime_ns.to_string(),
            ],
        )
        .map_err(sqlite_err)?;
        tx.commit().map_err(sqlite_err)?;
        Ok(())
    }

    /// Delete the database file entirely; the next open recreates and rebuilds.
    pub(crate) fn destroy(self) -> Result<()> {
        let path = self.db_path.clone();
        drop(self);
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        // WAL sidecar files.
        for suffix in ["-wal", "-shm"] {
            let side = PathBuf::from(format!("{}{suffix}", path.display()));
            let _ = std::fs::remove_file(side);
        }
        Ok(())
    }

    pub(crate) fn top_artists(
        &self,
        limit: usize,
        range: &Option<Range<u64>>,
    ) -> Result<Vec<ArtistCount>> {
        let (lo, hi) = range_bounds(range);
        let mut statement = self
            .conn
            .prepare(
                "SELECT artist, COUNT(*) AS n FROM scrobbles
                 WHERE deleted = 0 AND uts >= ?1 AND uts < ?2
                 GROUP BY artist ORDER BY n DESC, artist ASC LIMIT ?3",
            )
            .map_err(sqlite_err)?;
        let rows = statement
            .query_map(params![lo, hi, limit as i64], |row| {
                Ok(ArtistCount {
                    artist: row.get(0)?,
                    count: row.get::<_, i64>(1)? as u64,
                })
            })
            .map_err(sqlite_err)?;
        rows.collect::<std::result::Result<_, _>>()
            .map_err(sqlite_err)
    }

    pub(crate) fn top_tracks(
        &self,
        artist: Option<&str>,
        limit: usize,
        range: &Option<Range<u64>>,
    ) -> Result<Vec<TrackCount>> {
        let (lo, hi) = range_bounds(range);
        let mut statement = self
            .conn
            .prepare(
                "SELECT artist, track, COUNT(*) AS n FROM scrobbles
                 WHERE deleted = 0 AND uts >= ?1 AND uts < ?2
                   AND (?3 IS NULL OR artist = ?3)
                 GROUP BY artist, track ORDER BY n DESC, artist ASC, track ASC LIMIT ?4",
            )
            .map_err(sqlite_err)?;
        let rows = statement
            .query_map(params![lo, hi, artist, limit as i64], |row| {
                Ok(TrackCount {
                    artist: row.get(0)?,
                    track: row.get(1)?,
                    count: row.get::<_, i64>(2)? as u64,
                })
            })
            .map_err(sqlite_err)?;
        rows.collect::<std::result::Result<_, _>>()
            .map_err(sqlite_err)
    }

    pub(crate) fn top_albums(
        &self,
        artist: Option<&str>,
        limit: usize,
        range: &Option<Range<u64>>,
    ) -> Result<Vec<AlbumCount>> {
        let (lo, hi) = range_bounds(range);
        let mut statement = self
            .conn
            .prepare(
                "SELECT artist, album, COUNT(*) AS n FROM scrobbles
                 WHERE deleted = 0 AND uts >= ?1 AND uts < ?2
                   AND album IS NOT NULL AND album != ''
                   AND (?3 IS NULL OR artist = ?3)
                 GROUP BY artist, album ORDER BY n DESC, artist ASC, album ASC LIMIT ?4",
            )
            .map_err(sqlite_err)?;
        let rows = statement
            .query_map(params![lo, hi, artist, limit as i64], |row| {
                Ok(AlbumCount {
                    artist: row.get(0)?,
                    album: row.get(1)?,
                    count: row.get::<_, i64>(2)? as u64,
                })
            })
            .map_err(sqlite_err)?;
        rows.collect::<std::result::Result<_, _>>()
            .map_err(sqlite_err)
    }

    pub(crate) fn scrobble_count(&self, range: &Option<Range<u64>>) -> Result<u64> {
        let (lo, hi) = range_bounds(range);
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM scrobbles
                 WHERE deleted = 0 AND uts >= ?1 AND uts < ?2",
                params![lo, hi],
                |row| row.get::<_, i64>(0).map(|n| n as u64),
            )
            .map_err(sqlite_err)
    }

    pub(crate) fn artist_scrobbles(
        &self,
        artist: &str,
        range: &Option<Range<u64>>,
    ) -> Result<Vec<ScrobbleRecord>> {
        let (lo, hi) = range_bounds(range);
        let mut statement = self
            .conn
            .prepare(
                "SELECT json FROM scrobbles
                 WHERE deleted = 0 AND artist = ?1 AND uts >= ?2 AND uts < ?3
                 ORDER BY uts ASC, id ASC",
            )
            .map_err(sqlite_err)?;
        let rows = statement
            .query_map(params![artist, lo, hi], |row| row.get::<_, String>(0))
            .map_err(sqlite_err)?;
        let mut records = Vec::new();
        for json in rows {
            let json = json.map_err(sqlite_err)?;
            records.push(serde_json::from_str(&json)?);
        }
        Ok(records)
    }

    pub(crate) fn recent_scrobbles(
        &self,
        before: Option<&(u64, ScrobbleId)>,
        limit: usize,
    ) -> Result<Vec<ScrobbleRecord>> {
        // NULL cursor (no `before`) short-circuits the keyset predicate via `?1 IS NULL`.
        let (cursor_uts, cursor_id): (Option<i64>, Option<&str>) = match before {
            Some((uts, id)) => (Some(*uts as i64), Some(id.as_str())),
            None => (None, None),
        };
        let mut statement = self
            .conn
            .prepare(
                "SELECT json FROM scrobbles
                 WHERE deleted = 0
                   AND (?1 IS NULL OR uts < ?1 OR (uts = ?1 AND id < ?2))
                 ORDER BY uts DESC, id DESC
                 LIMIT ?3",
            )
            .map_err(sqlite_err)?;
        let rows = statement
            .query_map(params![cursor_uts, cursor_id, limit as i64], |row| {
                row.get::<_, String>(0)
            })
            .map_err(sqlite_err)?;
        let mut records = Vec::new();
        for json in rows {
            let json = json.map_err(sqlite_err)?;
            records.push(serde_json::from_str(&json)?);
        }
        Ok(records)
    }
}

fn range_bounds(range: &Option<Range<u64>>) -> (i64, i64) {
    match range {
        Some(r) => (r.start as i64, r.end as i64),
        None => (0, i64::MAX),
    }
}

/// Stable key for a partition file in the `ingested_files` table (the file name is unique
/// within a store and survives the store directory being moved).
fn path_key(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

fn sqlite_err(err: rusqlite::Error) -> StoreError {
    StoreError::Corrupt(format!("sqlite: {err}"))
}
