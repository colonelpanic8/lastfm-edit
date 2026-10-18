//! Rebuildable sqlite cache of trimmed, domain-typed MusicBrainz answers keyed by request.
//! Deleting the file only costs time.

use super::MbError;
use rusqlite::{params, OptionalExtension};
use serde::{de::DeserializeOwned, Serialize};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub(crate) struct MbCache {
    conn: Mutex<rusqlite::Connection>,
}

impl MbCache {
    pub fn open(path: &Path) -> Result<Self, MbError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(cache_error)?;
        }
        let conn = rusqlite::Connection::open(path).map_err(cache_error)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS responses (
                key TEXT PRIMARY KEY,
                fetched_at INTEGER NOT NULL,
                body TEXT NOT NULL
            )",
        )
        .map_err(cache_error)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// `None` when absent or older than `ttl`.
    pub fn get<T: DeserializeOwned>(&self, key: &str, ttl: Duration) -> Result<Option<T>, MbError> {
        let row: Option<(i64, String)> = self
            .conn
            .lock()
            .expect("cache lock")
            .query_row(
                "SELECT fetched_at, body FROM responses WHERE key = ?1",
                params![key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(cache_error)?;
        let Some((fetched_at, body)) = row else {
            return Ok(None);
        };
        if now_secs().saturating_sub(fetched_at) > ttl.as_secs() as i64 {
            return Ok(None);
        }
        // A body written by an older domain shape is a miss, not an error: refetching
        // overwrites it.
        Ok(serde_json::from_str(&body).ok())
    }

    pub fn put<T: Serialize>(&self, key: &str, value: &T) -> Result<(), MbError> {
        let body = serde_json::to_string(value).map_err(cache_error)?;
        self.conn
            .lock()
            .expect("cache lock")
            .execute(
                "INSERT OR REPLACE INTO responses (key, fetched_at, body) VALUES (?1, ?2, ?3)",
                params![key, now_secs(), body],
            )
            .map_err(cache_error)?;
        Ok(())
    }
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn cache_error(error: impl std::fmt::Display) -> MbError {
    MbError::Cache(error.to_string())
}
