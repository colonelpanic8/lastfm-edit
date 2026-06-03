use crate::{LastFmEditClient, Track};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteManifest {
    pub version: u32,
    pub generated_at_unix: u64,
    pub source: DeleteManifestSource,
    pub scrobbles: Vec<DeleteManifestEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteManifestSource {
    pub kind: String,
    pub range: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteManifestEntry {
    pub index: usize,
    pub offset: Option<u64>,
    pub artist: String,
    pub track: String,
    pub album: Option<String>,
    pub timestamp: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteTarget {
    pub offset: Option<u64>,
    pub artist: String,
    pub track: String,
    pub album: Option<String>,
    pub timestamp: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeleteAttemptResult {
    Deleted,
    NotDeleted { message: String },
    Error { message: String },
}

impl DeleteAttemptResult {
    pub fn success(&self) -> bool {
        matches!(self, Self::Deleted)
    }

    pub fn message(&self) -> Option<&str> {
        match self {
            Self::Deleted => None,
            Self::NotDeleted { message } | Self::Error { message } => Some(message),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteExecutionSummary {
    pub total_found: usize,
    pub successful_deletions: usize,
    pub failed_deletions: usize,
}

pub fn target_from_track(track: &Track, offset: Option<u64>, timestamp: u64) -> DeleteTarget {
    DeleteTarget {
        offset,
        artist: track.artist.clone(),
        track: track.name.clone(),
        album: track.album.clone(),
        timestamp,
    }
}

impl DeleteManifest {
    pub fn new(source: DeleteManifestSource, targets: &[DeleteTarget]) -> crate::Result<Self> {
        let generated_at_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| crate::LastFmError::Parse(e.to_string()))?
            .as_secs();

        Ok(Self {
            version: 1,
            generated_at_unix,
            source,
            scrobbles: targets
                .iter()
                .enumerate()
                .map(|(i, target)| DeleteManifestEntry {
                    index: i + 1,
                    offset: target.offset,
                    artist: target.artist.clone(),
                    track: target.track.clone(),
                    album: target.album.clone(),
                    timestamp: target.timestamp,
                })
                .collect(),
        })
    }

    pub fn targets(&self) -> Vec<DeleteTarget> {
        self.scrobbles
            .iter()
            .map(|entry| DeleteTarget {
                offset: entry.offset,
                artist: entry.artist.clone(),
                track: entry.track.clone(),
                album: entry.album.clone(),
                timestamp: entry.timestamp,
            })
            .collect()
    }
}

pub fn read_manifest(path: &Path) -> crate::Result<DeleteManifest> {
    let contents = fs::read_to_string(path)?;
    let manifest: DeleteManifest =
        serde_json::from_str(&contents).map_err(|e| crate::LastFmError::Parse(e.to_string()))?;

    if manifest.version != 1 {
        return Err(crate::LastFmError::Parse(format!(
            "Unsupported delete manifest version {} in '{}'",
            manifest.version,
            path.display()
        )));
    }

    Ok(manifest)
}

pub fn write_manifest(
    path: &Path,
    source: DeleteManifestSource,
    targets: &[DeleteTarget],
) -> crate::Result<()> {
    let manifest = DeleteManifest::new(source, targets)?;
    let json = serde_json::to_string_pretty(&manifest)
        .map_err(|e| crate::LastFmError::Parse(e.to_string()))?;
    fs::write(path, format!("{json}\n"))?;
    Ok(())
}

pub async fn execute_delete_targets<C, F>(
    client: &C,
    targets: &[DeleteTarget],
    delete_delay: Duration,
    mut on_attempt: F,
) -> crate::Result<DeleteExecutionSummary>
where
    C: LastFmEditClient + ?Sized,
    F: FnMut(usize, &DeleteTarget, &DeleteAttemptResult),
{
    let mut successful_deletions = 0;
    let mut failed_deletions = 0;

    for (i, target) in targets.iter().enumerate() {
        let result = match client
            .delete_scrobble(&target.artist, &target.track, target.timestamp)
            .await
        {
            Ok(true) => {
                successful_deletions += 1;
                DeleteAttemptResult::Deleted
            }
            Ok(false) => {
                failed_deletions += 1;
                let message = "Deletion failed; the scrobble may already be missing".to_string();
                log::warn!(
                    "Could not delete '{}' by '{}' at timestamp {}; {}",
                    target.track,
                    target.artist,
                    target.timestamp,
                    message
                );
                DeleteAttemptResult::NotDeleted { message }
            }
            Err(e) => {
                failed_deletions += 1;
                let message = e.to_string();
                log::warn!(
                    "Error deleting '{}' by '{}' at timestamp {}: {}",
                    target.track,
                    target.artist,
                    target.timestamp,
                    message
                );
                DeleteAttemptResult::Error { message }
            }
        };

        on_attempt(i + 1, target, &result);

        if i < targets.len() - 1 {
            tokio::time::sleep(delete_delay).await;
        }
    }

    Ok(DeleteExecutionSummary {
        total_found: targets.len(),
        successful_deletions,
        failed_deletions,
    })
}
