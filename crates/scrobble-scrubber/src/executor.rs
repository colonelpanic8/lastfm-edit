//! The executor: the single paced lane through which ALL last.fm write traffic flows.
//!
//! Drains `Ready`/`InProgress` intents from the durable queue, oldest first. Per intent it
//! re-expands the subject against the *live* store (so instances discovered after planning
//! are included), then per instance: waits for rate-limit clearance, enriches + applies via
//! the store's crash-safe [`MirroredEditor`], and records per-instance progress back into
//! the queue — a crash resumes exactly where it stopped.
//!
//! Rate limits are never "failures": a propagated `RateLimit` error pauses the executor
//! (emitting [`ScrubberEvent::ExecutorPaused`]) and retries the same instance once the
//! client is no longer parked. An inter-edit delay paces even successful traffic.

use crate::error::{Result, ScrubberError};
use crate::events::{ExecReport, ScrubberEvent, ScrubberEventBus, ScrubberEventReceiver};
use crate::queue::{EditIntent, InstanceStatus, IntentState, QueueEvent, QueueEventKind};
use crate::state::ScrubberState;
use lastfm_edit::{ExactScrobbleEdit, LastFmEditClient, RateLimitState};
use scrobble_store::{EditOutcome, MirroredEditor, ScrobbleId, Storage, StoreError};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use uuid::Uuid;

/// Executor tuning knobs.
#[derive(Clone, Debug)]
pub struct ExecutorOptions {
    /// Pause between consecutive upstream edit operations (pacing beyond rate limits).
    pub inter_edit_delay: std::time::Duration,
    /// Stop after this many upstream edit attempts (bounded sessions). `None` = no cap.
    pub max_edits: Option<u32>,
    /// Give up on an intent once every remaining instance has failed this many times.
    pub max_attempts_per_instance: u32,
}

impl Default for ExecutorOptions {
    fn default() -> Self {
        Self {
            inter_edit_delay: std::time::Duration::from_secs(2),
            max_edits: None,
            max_attempts_per_instance: 3,
        }
    }
}

pub struct Executor<C: LastFmEditClient> {
    store: Arc<dyn Storage>,
    state: Arc<dyn ScrubberState>,
    editor: MirroredEditor<C>,
    client: C,
    events: ScrubberEventBus,
    options: ExecutorOptions,
    cancelled: Arc<AtomicBool>,
}

impl<C: LastFmEditClient + Clone> Executor<C> {
    pub fn new(store: Arc<dyn Storage>, state: Arc<dyn ScrubberState>, client: C) -> Self {
        let editor = MirroredEditor::new(store.clone(), client.clone());
        Self::from_parts(store, state, editor, client)
    }
}

impl<C: LastFmEditClient> Executor<C> {
    /// Assemble from separately-constructed parts (useful when the client isn't `Clone`,
    /// e.g. mocks in tests). `client` is used only for rate-limit state watching; in
    /// production it should share the editor client's broadcaster.
    pub fn from_parts(
        store: Arc<dyn Storage>,
        state: Arc<dyn ScrubberState>,
        editor: MirroredEditor<C>,
        client: C,
    ) -> Self {
        Self {
            editor,
            store,
            state,
            client,
            events: ScrubberEventBus::new(),
            options: ExecutorOptions::default(),
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn with_options(mut self, options: ExecutorOptions) -> Self {
        self.options = options;
        self
    }

    pub fn with_event_bus(mut self, events: ScrubberEventBus) -> Self {
        self.events = events;
        self
    }

    pub fn subscribe(&self) -> ScrubberEventReceiver {
        self.events.subscribe()
    }

    pub fn event_bus(&self) -> ScrubberEventBus {
        self.events.clone()
    }

    /// A handle that cancels in-flight and future executor calls when flipped.
    pub fn cancel_handle(&self) -> Arc<AtomicBool> {
        self.cancelled.clone()
    }

    /// Access the underlying mirrored editor (e.g. for startup `resume_pending`).
    pub fn editor(&self) -> &MirroredEditor<C> {
        &self.editor
    }

    fn now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    fn check_cancelled(&self) -> Result<()> {
        if self.cancelled.load(Ordering::Relaxed) {
            Err(ScrubberError::Cancelled)
        } else {
            Ok(())
        }
    }

    /// Drain currently-executable intents once. Returns what happened; leaves anything
    /// unfinished (budget, failures) in the queue for a later pass.
    pub async fn run_once(&self) -> Result<ExecReport> {
        self.run_once_with_budget(self.options.max_edits).await
    }

    /// Like [`Executor::run_once`], with a per-call attempt budget overriding the
    /// configured one (used by command-driven hosts).
    pub async fn run_once_with_budget(&self, max_edits: Option<u32>) -> Result<ExecReport> {
        self.events.emit(ScrubberEvent::ExecStarted);
        let result = self.run_once_inner(max_edits).await;
        match &result {
            Ok(report) => self.events.emit(ScrubberEvent::ExecCompleted {
                report: report.clone(),
            }),
            Err(err) => self.events.emit(ScrubberEvent::Error {
                error: err.to_string(),
            }),
        }
        result
    }

    async fn run_once_inner(&self, max_edits: Option<u32>) -> Result<ExecReport> {
        let mut report = ExecReport::default();
        let mut attempts_used: u32 = 0;

        let queue = self.state.load_queue().await?;
        for intent in queue.into_iter().filter(|i| i.state.is_executable()) {
            self.check_cancelled()?;
            if budget_exhausted(max_edits, attempts_used) {
                break;
            }
            report.intents_processed += 1;
            self.execute_intent(intent, max_edits, &mut report, &mut attempts_used)
                .await?;
        }
        Ok(report)
    }

    /// Live instances of an intent's subject, oldest first.
    async fn live_instances(&self, intent: &EditIntent) -> Result<Vec<ScrobbleId>> {
        let records = self
            .store
            .artist_scrobbles(&intent.subject.artist, None)
            .await?;
        Ok(records
            .into_iter()
            .filter(|record| intent.subject.matches_record(record))
            .map(|record| record.id)
            .collect())
    }

    async fn execute_intent(
        &self,
        intent: EditIntent,
        max_edits: Option<u32>,
        report: &mut ExecReport,
        attempts_used: &mut u32,
    ) -> Result<()> {
        // Re-expand against the live store — NOT the planning-time snapshot.
        let live = self.live_instances(&intent).await?;
        if live.is_empty() {
            self.append(intent.id, QueueEventKind::Completed).await?;
            self.events.emit(ScrubberEvent::IntentCompleted {
                id: intent.id,
                state: IntentState::Applied,
            });
            report.intents_completed += 1;
            return Ok(());
        }
        self.append(
            intent.id,
            QueueEventKind::Expanded {
                instance_ids: live.clone(),
            },
        )
        .await?;
        self.events.emit(ScrubberEvent::IntentExpanded {
            id: intent.id,
            subject: intent.subject.clone(),
            instances: live.len(),
        });

        let mut progress = intent.instances.clone();
        for id in &live {
            progress
                .entry(id.clone())
                .or_insert(InstanceStatus::Pending);
        }

        for instance in &live {
            self.check_cancelled()?;
            if budget_exhausted(max_edits, *attempts_used) {
                return Ok(()); // stays InProgress for a later run
            }
            match progress.get(instance) {
                Some(InstanceStatus::Applied { .. }) => continue,
                Some(InstanceStatus::Failed { attempts, .. })
                    if *attempts >= self.options.max_attempts_per_instance =>
                {
                    continue
                }
                _ => {}
            }

            *attempts_used += 1;
            match self.apply_instance(&intent, instance).await? {
                InstanceOutcome::Applied { edit_id } => {
                    progress.insert(
                        instance.clone(),
                        InstanceStatus::Applied {
                            edit_id: edit_id.clone(),
                        },
                    );
                    self.append(
                        intent.id,
                        QueueEventKind::InstanceApplied {
                            instance: instance.clone(),
                            edit_id: edit_id.clone(),
                        },
                    )
                    .await?;
                    report.instances_applied += 1;
                    self.events.emit(ScrubberEvent::EditApplied {
                        intent: intent.id,
                        subject: intent.subject.clone(),
                        instance: instance.clone(),
                        edit_id,
                    });
                }
                InstanceOutcome::Gone => {
                    // No longer live (edited/deleted since expansion); the completion
                    // check below re-queries liveness, so simply move on.
                    log::debug!("instance {instance} vanished during execution; skipping");
                }
                InstanceOutcome::Failed { error } => {
                    let attempts = match progress.get(instance) {
                        Some(InstanceStatus::Failed { attempts, .. }) => attempts + 1,
                        _ => 1,
                    };
                    progress.insert(
                        instance.clone(),
                        InstanceStatus::Failed {
                            attempts,
                            last_error: error.clone(),
                        },
                    );
                    self.append(
                        intent.id,
                        QueueEventKind::InstanceFailed {
                            instance: instance.clone(),
                            error: error.clone(),
                        },
                    )
                    .await?;
                    report.instances_failed += 1;
                    self.events.emit(ScrubberEvent::EditFailed {
                        intent: intent.id,
                        subject: intent.subject.clone(),
                        instance: instance.clone(),
                        error,
                    });
                }
                InstanceOutcome::AbandonIntent { reason } => {
                    self.append(
                        intent.id,
                        QueueEventKind::Abandoned {
                            reason: reason.clone(),
                        },
                    )
                    .await?;
                    self.events.emit(ScrubberEvent::IntentCompleted {
                        id: intent.id,
                        state: IntentState::Abandoned { reason },
                    });
                    report.intents_abandoned += 1;
                    return Ok(());
                }
            }

            tokio::time::sleep(self.options.inter_edit_delay).await;
        }

        // Completion: every *currently live* instance applied?
        let remaining = self.live_instances(&intent).await?;
        let all_applied = remaining
            .iter()
            .all(|id| matches!(progress.get(id), Some(InstanceStatus::Applied { .. })));
        if all_applied {
            self.append(intent.id, QueueEventKind::Completed).await?;
            self.events.emit(ScrubberEvent::IntentCompleted {
                id: intent.id,
                state: IntentState::Applied,
            });
            report.intents_completed += 1;
            return Ok(());
        }

        // Abandon when everything left has exhausted its attempts.
        let exhausted = remaining.iter().all(|id| match progress.get(id) {
            Some(InstanceStatus::Applied { .. }) => true,
            Some(InstanceStatus::Failed { attempts, .. }) => {
                *attempts >= self.options.max_attempts_per_instance
            }
            _ => false,
        });
        if exhausted {
            let reason = format!(
                "{} instance(s) failed after {} attempts",
                progress
                    .values()
                    .filter(|s| matches!(s, InstanceStatus::Failed { .. }))
                    .count(),
                self.options.max_attempts_per_instance
            );
            self.append(
                intent.id,
                QueueEventKind::Abandoned {
                    reason: reason.clone(),
                },
            )
            .await?;
            self.events.emit(ScrubberEvent::IntentCompleted {
                id: intent.id,
                state: IntentState::Abandoned { reason },
            });
            report.intents_abandoned += 1;
        }
        // Otherwise: stays InProgress (budget/cancel/pending retries) for a later pass.
        Ok(())
    }

    /// One instance: clearance → prepare (enrichment scrape) → overlay → apply.
    /// Rate limits pause-and-retry indefinitely; they never consume an attempt.
    async fn apply_instance(
        &self,
        intent: &EditIntent,
        instance: &ScrobbleId,
    ) -> Result<InstanceOutcome> {
        loop {
            self.check_cancelled()?;
            self.await_rate_limit_clearance().await?;

            let prepared = match self.editor.prepare_edit(instance).await {
                Ok(prepared) => prepared,
                Err(StoreError::LastFm(lastfm_edit::LastFmError::RateLimit { retry_after })) => {
                    self.pause_for_rate_limit(retry_after).await?;
                    continue;
                }
                Err(StoreError::NotFound(_)) => return Ok(InstanceOutcome::Gone),
                Err(err) => {
                    return Ok(InstanceOutcome::Failed {
                        error: format!("prepare failed: {err}"),
                    })
                }
            };

            let exact = overlay_proposal(prepared, &intent.proposed);

            match self.editor.apply_edit(exact).await {
                Ok(EditOutcome::Applied { edit_id, .. })
                | Ok(EditOutcome::AlreadyApplied { edit_id, .. }) => {
                    return Ok(InstanceOutcome::Applied { edit_id });
                }
                Ok(EditOutcome::Failed { error }) => {
                    return Ok(InstanceOutcome::Failed { error });
                }
                Err(StoreError::LastFm(lastfm_edit::LastFmError::RateLimit { retry_after })) => {
                    self.pause_for_rate_limit(retry_after).await?;
                    continue;
                }
                Err(StoreError::NeedsRebase(reason)) => {
                    // The store changed under the proposal; the next incremental plan
                    // re-suggests from current state.
                    return Ok(InstanceOutcome::AbandonIntent {
                        reason: format!("needs rebase: {reason}"),
                    });
                }
                Err(StoreError::NotFound(_)) => return Ok(InstanceOutcome::Gone),
                Err(err) => {
                    return Ok(InstanceOutcome::Failed {
                        error: err.to_string(),
                    })
                }
            }
        }
    }

    async fn pause_for_rate_limit(&self, retry_after: u64) -> Result<()> {
        let until = Self::now() + retry_after;
        self.events.emit(ScrubberEvent::ExecutorPaused {
            reason: scrobble_store::PauseReason::RateLimited {
                until_estimate: Some(until),
            },
        });
        self.sleep_or_cancelled(retry_after.min(60)).await?;
        self.events.emit(ScrubberEvent::ExecutorResumed);
        Ok(())
    }

    /// Wait until the client's rate-limit state clears (same watch-loop pattern as the
    /// store's sync engine).
    async fn await_rate_limit_clearance(&self) -> Result<()> {
        let mut watcher = self.client.watch_rate_limit_state();
        let mut paused = false;
        loop {
            let state = watcher.borrow_and_update().clone();
            let now = Self::now();
            if !state.is_rate_limited_at(now) {
                if paused {
                    self.events.emit(ScrubberEvent::ExecutorResumed);
                }
                return Ok(());
            }
            if let RateLimitState::RateLimited { until_estimate, .. } = &state {
                if !paused {
                    paused = true;
                    self.events.emit(ScrubberEvent::ExecutorPaused {
                        reason: scrobble_store::PauseReason::RateLimited {
                            until_estimate: Some(*until_estimate),
                        },
                    });
                }
                let wait = state
                    .remaining_at(now)
                    .map(|d| d.as_secs().clamp(1, 30))
                    .unwrap_or(1);
                tokio::select! {
                    _ = watcher.changed() => {}
                    _ = tokio::time::sleep(std::time::Duration::from_secs(wait)) => {}
                }
                self.check_cancelled()?;
            }
        }
    }

    async fn sleep_or_cancelled(&self, secs: u64) -> Result<()> {
        let deadline = std::time::Duration::from_secs(secs);
        let step = std::time::Duration::from_millis(200);
        let mut waited = std::time::Duration::ZERO;
        while waited < deadline {
            self.check_cancelled()?;
            let chunk = step.min(deadline - waited);
            tokio::time::sleep(chunk).await;
            waited += chunk;
        }
        self.check_cancelled()
    }

    async fn append(&self, id: Uuid, kind: QueueEventKind) -> Result<()> {
        self.state
            .append_queue_events(&[QueueEvent {
                id,
                at: Self::now(),
                kind,
            }])
            .await
    }
}

fn budget_exhausted(max_edits: Option<u32>, attempts_used: u32) -> bool {
    max_edits.is_some_and(|cap| attempts_used >= cap)
}

enum InstanceOutcome {
    Applied {
        edit_id: String,
    },
    Failed {
        error: String,
    },
    /// Instance no longer live in the store; not an error.
    Gone,
    /// The whole intent is stale; abandon it.
    AbandonIntent {
        reason: String,
    },
}

/// Overlay the proposal's *changed* fields onto a freshly-prepared exact edit.
///
/// Only fields where the proposal's new value differs from its original are copied; the
/// originals (including the enriched, authoritative `album_artist_name_original`) come
/// from [`MirroredEditor::prepare_edit`].
fn overlay_proposal(
    mut exact: ExactScrobbleEdit,
    proposal: &lastfm_edit::ScrobbleEdit,
) -> ExactScrobbleEdit {
    if let (Some(original), Some(new)) = (&proposal.track_name_original, &proposal.track_name) {
        if original != new {
            exact.track_name = new.clone();
        }
    }
    if proposal.artist_name_original != proposal.artist_name {
        exact.artist_name = proposal.artist_name.clone();
    }
    if proposal.album_name_original != proposal.album_name {
        if let Some(new) = &proposal.album_name {
            exact.album_name = new.clone();
        }
    }
    if proposal.album_artist_name_original != proposal.album_artist_name {
        if let Some(new) = &proposal.album_artist_name {
            exact.album_artist_name = new.clone();
        }
    }
    exact
}
