//! Mirrored edits: apply changes to Last.fm and the local store in lockstep, with a durable
//! append-only edit log that makes the process crash-safe and auditable.
//!
//! ## Lifecycle
//!
//! 1. Intent is appended to the log (`Queued`) *before* touching Last.fm.
//! 2. The edit is applied upstream.
//! 3. On success, the local store is updated (new record; tombstone the old id if the edit
//!    changed identity) and `Applied` is appended.
//!
//! A crash between 2 and 3 leaves a `Pending` entry; [`MirroredEditor::resume_pending`]
//! resolves the ambiguity by checking the one-second window around the scrobble upstream
//! (the *crash-window rule*) instead of blindly re-submitting.
//!
//! Only **exact single-scrobble operations** are supported — never `edit_all` wildcards,
//! which would mutate remote scrobbles we cannot enumerate and silently corrupt coverage.
//! Bulk edits are expressed by querying the local store (cheap, offline) and emitting one
//! exact edit per record.

mod log_types;

pub use log_types::{fold_edit_log, EditEventKind, EditLogEntry, EditLogEvent, EditOp, EditState};

use crate::error::{Result, StoreError};
use crate::id::ScrobbleId;
use crate::record::{Provenanced, RecordSource, ScrobbleRecord};
use crate::source::ScrobbleSource;
use crate::storage::Storage;
use crate::sync::events::{SyncEvent, SyncEventBus};
use lastfm_edit::{ExactScrobbleEdit, LastFmEditClient};
use std::sync::Arc;

/// Outcome of applying (or resuming) one mirrored operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditOutcome {
    /// Applied upstream and mirrored locally; these are the ids now live in the store.
    /// `edit_id` identifies the entry in the durable edit log for cross-referencing.
    Applied {
        result_ids: Vec<ScrobbleId>,
        edit_id: String,
    },
    /// Resume found the operation already reflected upstream; only the mirror was updated.
    AlreadyApplied {
        result_ids: Vec<ScrobbleId>,
        edit_id: String,
    },
    /// The attempt failed (recorded in the log; retriable via `resume_pending`).
    Failed { error: String },
}

/// Applies exact single-scrobble edits/deletes to Last.fm and the local store together.
pub struct MirroredEditor<C> {
    store: Arc<dyn Storage>,
    client: C,
    events: SyncEventBus,
    max_retries: u32,
}

impl<C: LastFmEditClient> MirroredEditor<C> {
    pub fn new(store: Arc<dyn Storage>, client: C) -> Self {
        Self {
            store,
            client,
            events: SyncEventBus::new(),
            max_retries: 3,
        }
    }

    /// Share an existing event bus (e.g. the sync engine's) so consumers see one stream.
    pub fn with_event_bus(mut self, events: SyncEventBus) -> Self {
        self.events = events;
        self
    }

    pub fn subscribe(&self) -> crate::sync::events::SyncEventReceiver {
        self.events.subscribe()
    }

    fn now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    /// Ensure a record's album artist is `Verified`, scraping the edit-form values if
    /// necessary, and persist what was learned. Returns the (possibly upgraded) record.
    pub async fn enrich(&self, id: &ScrobbleId) -> Result<ScrobbleRecord> {
        let record = self
            .store
            .get_scrobble(id)
            .await?
            .ok_or_else(|| StoreError::NotFound(id.to_string()))?;
        if record.deleted {
            return Err(StoreError::NotFound(format!("{id} (tombstoned)")));
        }
        if record.album_artist.is_verified() {
            return Ok(record);
        }

        let variations = self
            .client
            .get_scrobble_edit_variations(&record.track, &record.artist)
            .await?;
        let variation = variations
            .iter()
            .find(|v| Some(&v.album_name_original) == record.album.as_ref())
            .or_else(|| variations.first())
            .ok_or_else(|| {
                StoreError::NotFound(format!(
                    "no edit-form variations upstream for {} - {}",
                    record.artist, record.track
                ))
            })?;

        let mut upgraded = record.clone();
        upgraded.album_artist = Provenanced::Verified(variation.album_artist_name_original.clone());
        if upgraded.album.is_none() {
            upgraded.album = Some(variation.album_name_original.clone());
        }
        upgraded.source = RecordSource::Scrape;
        upgraded.fetched_at = Self::now();
        self.store
            .append_scrobbles(std::slice::from_ref(&upgraded))
            .await?;
        Ok(upgraded)
    }

    /// Build a fully-specified edit for a stored scrobble (enriching album artist first).
    /// The returned edit has new values equal to originals; mutate the `*_name` fields you
    /// want to change, then pass it to [`MirroredEditor::apply_edit`].
    pub async fn prepare_edit(&self, id: &ScrobbleId) -> Result<ExactScrobbleEdit> {
        let record = self.enrich(id).await?;
        let album = record.album.clone().unwrap_or_default();
        let album_artist = record
            .album_artist
            .value()
            .cloned()
            .unwrap_or_else(|| record.artist.clone());
        Ok(ExactScrobbleEdit {
            track_name_original: record.track.clone(),
            album_name_original: album.clone(),
            artist_name_original: record.artist.clone(),
            album_artist_name_original: album_artist.clone(),
            track_name: record.track,
            album_name: album,
            artist_name: record.artist,
            album_artist_name: album_artist,
            timestamp: record.uts,
            edit_all: false,
        })
    }

    /// Apply one exact edit to Last.fm and mirror it locally. `edit_all` must be `false`.
    pub async fn apply_edit(&self, edit: ExactScrobbleEdit) -> Result<EditOutcome> {
        if edit.edit_all {
            return Err(StoreError::NeedsRebase(
                "edit_all edits cannot be mirrored; expand them into exact per-scrobble edits"
                    .to_string(),
            ));
        }
        let target_id = ScrobbleId::new(
            edit.timestamp,
            &edit.artist_name_original,
            &edit.track_name_original,
        );
        // The mirror must know the scrobble, with a verified album artist that matches what
        // the edit claims as original — otherwise the edit was built on stale data.
        let record = self.enrich(&target_id).await?;
        if record.album_artist.value() != Some(&edit.album_artist_name_original) {
            return Err(StoreError::NeedsRebase(format!(
                "album_artist_name_original {:?} does not match verified value {:?}",
                edit.album_artist_name_original,
                record.album_artist.value()
            )));
        }

        let op = EditOp::Edit(Box::new(edit.clone()));
        let entry = self.queue(op.clone(), vec![target_id.clone()]).await?;
        self.attempt(entry, op).await
    }

    /// Delete one scrobble upstream and tombstone it locally.
    pub async fn apply_delete(&self, id: &ScrobbleId) -> Result<EditOutcome> {
        let record = self
            .store
            .get_scrobble(id)
            .await?
            .ok_or_else(|| StoreError::NotFound(id.to_string()))?;
        let op = EditOp::Delete {
            artist: record.artist.clone(),
            track: record.track.clone(),
            uts: record.uts,
        };
        let entry = self.queue(op.clone(), vec![id.clone()]).await?;
        self.attempt(entry, op).await
    }

    /// Retry all pending log entries, resolving crash ambiguity against `source` first:
    /// if the one-second window around the scrobble already reflects the operation, the
    /// entry is marked applied without re-submitting.
    pub async fn resume_pending(
        &self,
        source: &dyn ScrobbleSource,
    ) -> Result<Vec<(String, EditOutcome)>> {
        let log = self.store.load_edit_log().await?;
        let mut outcomes = Vec::new();
        for entry in log.into_iter().filter(|e| e.state.is_pending()) {
            let op = entry.op.clone();
            let outcome = match self.check_already_applied(&op, source).await? {
                Some(result_ids) => {
                    self.mirror_locally(&op, &entry.target_ids).await?;
                    self.append_event(
                        &entry.edit_id,
                        EditEventKind::Applied {
                            result_ids: result_ids.clone(),
                        },
                    )
                    .await?;
                    self.events.emit(SyncEvent::EditApplied {
                        edit_id: entry.edit_id.clone(),
                    });
                    EditOutcome::AlreadyApplied {
                        result_ids,
                        edit_id: entry.edit_id.clone(),
                    }
                }
                None => self.attempt(entry.clone(), op).await?,
            };
            outcomes.push((entry.edit_id, outcome));
        }
        Ok(outcomes)
    }

    // ---- internals ---------------------------------------------------------------------

    async fn queue(&self, op: EditOp, target_ids: Vec<ScrobbleId>) -> Result<EditLogEntry> {
        let at = Self::now();
        let edit_id = log_types::new_edit_id(&op, at);
        let event = EditLogEvent {
            edit_id: edit_id.clone(),
            at,
            kind: EditEventKind::Queued {
                op: op.clone(),
                target_ids: target_ids.clone(),
            },
        };
        self.store.append_edit_events(&[event]).await?;
        self.events.emit(SyncEvent::EditQueued {
            edit_id: edit_id.clone(),
        });
        Ok(EditLogEntry {
            edit_id,
            op,
            target_ids,
            state: EditState::Pending {
                attempts: 0,
                last_error: None,
            },
            created_at: at,
            updated_at: at,
        })
    }

    async fn append_event(&self, edit_id: &str, kind: EditEventKind) -> Result<()> {
        self.store
            .append_edit_events(&[EditLogEvent {
                edit_id: edit_id.to_string(),
                at: Self::now(),
                kind,
            }])
            .await
    }

    /// One upstream attempt for a queued entry, mirroring locally on success.
    ///
    /// Rate-limit errors propagate WITHOUT consuming an attempt: the entry stays pending
    /// and the caller (or `resume_pending`) retries once the client is no longer parked.
    async fn attempt(&self, entry: EditLogEntry, op: EditOp) -> Result<EditOutcome> {
        let upstream_result = match &op {
            EditOp::Edit(edit) => self
                .client
                .edit_scrobble_single(edit, self.max_retries)
                .await
                .map(|response| {
                    let ok = response.all_successful();
                    let message = response
                        .individual_results
                        .iter()
                        .find(|r| !r.success)
                        .and_then(|r| r.message.clone());
                    (ok, message)
                }),
            EditOp::Delete { artist, track, uts } => self
                .client
                .delete_scrobble(artist, track, *uts)
                .await
                .map(|ok| (ok, None)),
        };

        match upstream_result {
            Ok((true, _)) => {
                let result_ids = self.mirror_locally(&op, &entry.target_ids).await?;
                self.append_event(
                    &entry.edit_id,
                    EditEventKind::Applied {
                        result_ids: result_ids.clone(),
                    },
                )
                .await?;
                self.events.emit(SyncEvent::EditApplied {
                    edit_id: entry.edit_id.clone(),
                });
                Ok(EditOutcome::Applied {
                    result_ids,
                    edit_id: entry.edit_id.clone(),
                })
            }
            Ok((false, message)) => {
                let error = message.unwrap_or_else(|| "edit rejected by last.fm".to_string());
                self.append_event(
                    &entry.edit_id,
                    EditEventKind::AttemptFailed {
                        error: error.clone(),
                    },
                )
                .await?;
                self.events.emit(SyncEvent::EditFailed {
                    edit_id: entry.edit_id.clone(),
                    error: error.clone(),
                    will_retry: true,
                });
                Ok(EditOutcome::Failed { error })
            }
            Err(err @ lastfm_edit::LastFmError::RateLimit { .. }) => {
                // Not a failed attempt — the request never really ran. Stay pending.
                Err(StoreError::LastFm(err))
            }
            Err(err) => {
                let error = err.to_string();
                self.append_event(
                    &entry.edit_id,
                    EditEventKind::AttemptFailed {
                        error: error.clone(),
                    },
                )
                .await?;
                self.events.emit(SyncEvent::EditFailed {
                    edit_id: entry.edit_id.clone(),
                    error: error.clone(),
                    will_retry: true,
                });
                Ok(EditOutcome::Failed { error })
            }
        }
    }

    /// Write the local mirror of a successful upstream operation. Returns the ids now live.
    async fn mirror_locally(
        &self,
        op: &EditOp,
        target_ids: &[ScrobbleId],
    ) -> Result<Vec<ScrobbleId>> {
        let now = Self::now();
        match op {
            EditOp::Edit(edit) => {
                let new_id = ScrobbleId::new(edit.timestamp, &edit.artist_name, &edit.track_name);
                let new_record = ScrobbleRecord {
                    id: new_id.clone(),
                    uts: edit.timestamp,
                    artist: edit.artist_name.clone(),
                    track: edit.track_name.clone(),
                    album: Some(edit.album_name.clone()),
                    album_artist: Provenanced::Verified(edit.album_artist_name.clone()),
                    source: RecordSource::EditMirror,
                    fetched_at: now,
                    deleted: false,
                    v: 1,
                };
                let mut batch = vec![new_record];
                for old_id in target_ids {
                    if old_id != &new_id {
                        if let Some(old) = self.store.get_scrobble(old_id).await? {
                            batch.push(old.into_tombstone(now));
                        }
                    }
                }
                self.store.append_scrobbles(&batch).await?;
                Ok(vec![new_id])
            }
            EditOp::Delete { .. } => {
                let mut batch = Vec::new();
                for id in target_ids {
                    if let Some(record) = self.store.get_scrobble(id).await? {
                        batch.push(record.into_tombstone(now));
                    }
                }
                self.store.append_scrobbles(&batch).await?;
                Ok(Vec::new())
            }
        }
    }

    /// Crash-window rule: inspect `[uts, uts + 1)` upstream and decide whether the
    /// operation is already reflected there.
    async fn check_already_applied(
        &self,
        op: &EditOp,
        source: &dyn ScrobbleSource,
    ) -> Result<Option<Vec<ScrobbleId>>> {
        let uts = match op {
            EditOp::Edit(edit) => edit.timestamp,
            EditOp::Delete { uts, .. } => *uts,
        };
        let page = source.fetch_window(Some(uts), Some(uts + 1), 1).await?;
        let at_second: Vec<_> = page
            .tracks
            .iter()
            .filter(|t| t.timestamp == Some(uts))
            .collect();
        match op {
            EditOp::Edit(edit) => {
                // "Applied" means the post-edit values (including album) are what's
                // upstream; comparing album as well keeps album-only edits honest.
                let new_present = at_second.iter().any(|t| {
                    t.name == edit.track_name
                        && t.artist == edit.artist_name
                        && t.album.as_deref() == Some(edit.album_name.as_str())
                });
                let old_present = at_second.iter().any(|t| {
                    t.name == edit.track_name_original
                        && t.artist == edit.artist_name_original
                        && t.album.as_deref() == Some(edit.album_name_original.as_str())
                });
                if new_present && !old_present {
                    Ok(Some(vec![ScrobbleId::new(
                        edit.timestamp,
                        &edit.artist_name,
                        &edit.track_name,
                    )]))
                } else {
                    Ok(None)
                }
            }
            EditOp::Delete { artist, track, .. } => {
                let still_there = at_second
                    .iter()
                    .any(|t| &t.name == track && &t.artist == artist);
                if still_there {
                    Ok(None)
                } else {
                    Ok(Some(Vec::new()))
                }
            }
        }
    }
}
