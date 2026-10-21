//! Durable JSONL framing and atomic snapshots shared by store and scrubber state.

use serde::de::DeserializeOwned;
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

fn sync_parent(path: &Path) -> io::Result<()> {
    File::open(path.parent().unwrap_or(Path::new(".")))?.sync_all()
}

pub fn create_dir_all(path: &Path) -> io::Result<()> {
    if path.as_os_str().is_empty() || path.exists() {
        return Ok(());
    }
    ensure_parent(path)?;
    match std::fs::create_dir(path) {
        Ok(()) => sync_parent(path),
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(err) => Err(err),
    }
}

fn ensure_parent(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        create_dir_all(parent)?;
    }
    Ok(())
}

fn lock(path: &Path, exclusive: bool) -> io::Result<File> {
    ensure_parent(path)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path.with_extension("lock"))?;
    if exclusive {
        lock.lock()?;
    } else {
        lock.lock_shared()?;
    }
    Ok(lock)
}

/// Acquire a process-scoped lease without blocking. Dropping the file releases it.
/// The lease path must be distinct from files locked by operations inside the lease.
pub fn try_lease(path: &Path) -> io::Result<Option<File>> {
    ensure_parent(path)?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(error)) => Err(error),
    }
}

/// Parse complete records. Only an invalid, unterminated final fragment is recoverable.
pub fn parse_jsonl<T: DeserializeOwned>(bytes: &[u8]) -> io::Result<Vec<T>> {
    let mut items = Vec::new();
    for line in bytes.split_inclusive(|b| *b == b'\n') {
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        match serde_json::from_slice(line) {
            Ok(item) => items.push(item),
            Err(_)
                if !line.ends_with(b"\n")
                    && serde_json::from_slice::<serde_json::Value>(line).is_err() =>
            {
                break
            }
            Err(err) => return Err(io::Error::new(io::ErrorKind::InvalidData, err)),
        }
    }
    Ok(items)
}

pub fn read_jsonl<T: DeserializeOwned>(path: &Path) -> io::Result<Vec<T>> {
    let _lock = lock(path, false)?;
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };
    parse_jsonl(&bytes)
}

/// Repair a torn tail and append complete JSONL records under a cross-process lock.
pub fn append_jsonl(path: &Path, lines: &[u8]) -> io::Result<()> {
    if lines.is_empty() {
        return Ok(());
    }
    if !lines.ends_with(b"\n") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "append requires complete JSONL records",
        ));
    }
    parse_jsonl::<serde_json::Value>(lines)?;
    let _lock = lock(path, true)?;
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    parse_jsonl::<serde_json::Value>(&bytes)?;
    if !bytes.is_empty() && !bytes.ends_with(b"\n") {
        let start = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
        if serde_json::from_slice::<serde_json::Value>(&bytes[start..]).is_ok() {
            file.write_all(b"\n")?;
        } else {
            file.set_len(start as u64)?;
            file.seek(SeekFrom::Start(start as u64))?;
        }
    }
    file.write_all(lines)?;
    file.sync_all()?;
    sync_parent(path)
}

/// Replace a snapshot with a synced sibling, then sync its directory entry.
pub fn atomic_write(path: &Path, contents: &[u8]) -> io::Result<()> {
    let _lock = lock(path, true)?;
    atomic_write_locked(path, contents)
}

/// Transform a file while excluding appends throughout the read, fold, and replacement.
/// Return `None` from `rewrite` to leave its bytes unchanged.
pub fn rewrite_locked(
    path: &Path,
    rewrite: impl FnOnce(&[u8]) -> io::Result<Option<Vec<u8>>>,
) -> io::Result<()> {
    let _lock = lock(path, true)?;
    if let Some(contents) = rewrite(&std::fs::read(path)?)? {
        atomic_write_locked(path, &contents)?;
    }
    Ok(())
}

fn atomic_write_locked(path: &Path, contents: &[u8]) -> io::Result<()> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let (tmp, mut file) = loop {
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let tmp = path.with_extension(format!("{}.{}.tmp", std::process::id(), id));
        match OpenOptions::new().create_new(true).write(true).open(&tmp) {
            Ok(file) => break (tmp, file),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    };
    let result = (|| {
        file.write_all(contents)?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)?;
        sync_parent(path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(tmp);
    }
    result
}

#[cfg(test)]
#[path = "durable_tests.rs"]
mod tests;
