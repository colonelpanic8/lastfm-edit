//! The actor-style front door: drive the scrubber by sending it messages instead of
//! calling into it.
//!
//! A [`ScrubberActor`] owns a planner and an executor and processes [`ScrubberCommand`]s
//! from an mpsc inbox one at a time; progress flows out the shared
//! [`ScrubberEventBus`]. The cheap [`ScrubberHandle`] is the sending half — clone it into
//! UIs, bridges, or other tasks. Work can be *pushed* (`Consider` a batch of records from
//! anywhere) or *pulled* (`PlanFeed` the store-driven feeds); both run the same pipeline.
//!
//! The durable queue remains the seam between planning and execution, so commands are
//! fire-and-forget: a crash after `Consider` was processed loses nothing.

use crate::error::{Result, ScrubberError};
use crate::events::{ScrubberEvent, ScrubberEventBus, ScrubberEventReceiver};
use crate::executor::Executor;
use crate::feed::ScrubFeed;
use crate::planner::Planner;
use crate::state::ScrubberState;
use lastfm_edit::LastFmEditClient;
use scrobble_store::{ScrobbleRecord, Storage, SyncEvent, SyncEventReceiver};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;
use uuid::Uuid;

/// Messages the scrubber actor understands.
#[derive(Debug)]
pub enum ScrubberCommand {
    /// Push records for analysis from anywhere (no planning-coverage claims).
    Consider(Vec<ScrobbleRecord>),
    /// Run a store-driven planning pass.
    PlanFeed(ScrubFeed),
    /// Release an awaiting-approval intent for execution.
    Approve(Uuid),
    /// Decline an open intent, optionally dismissing its subject for good.
    Reject { id: Uuid, dismiss: bool },
    /// Un-reject a rejected intent, restoring it to its open state.
    Reinstate(Uuid),
    /// Drain ready intents through last.fm, optionally with an attempt budget.
    ExecuteOnce { max_edits: Option<u32> },
    /// Approve a provider-proposed rewrite rule (merges into the active set).
    ApproveRule(Uuid),
    /// Decline a provider-proposed rewrite rule.
    RejectRule(Uuid),
    /// Shut the actor down after the current command.
    Stop,
}

/// Cloneable sending half: commands in, events out.
#[derive(Clone)]
pub struct ScrubberHandle {
    commands: mpsc::Sender<ScrubberCommand>,
    events: ScrubberEventBus,
    cancel: Arc<AtomicBool>,
}

impl ScrubberHandle {
    /// Send a command, waiting for inbox space.
    pub async fn send(&self, command: ScrubberCommand) -> Result<()> {
        self.commands
            .send(command)
            .await
            .map_err(|_| ScrubberError::Cancelled)
    }

    /// Send without waiting; fails if the inbox is full or the actor stopped.
    pub fn try_send(&self, command: ScrubberCommand) -> Result<()> {
        self.commands
            .try_send(command)
            .map_err(|_| ScrubberError::Cancelled)
    }

    pub fn subscribe(&self) -> ScrubberEventReceiver {
        self.events.subscribe()
    }

    pub fn event_bus(&self) -> ScrubberEventBus {
        self.events.clone()
    }

    /// Interrupt an in-flight `ExecuteOnce` pass. Out-of-band on purpose: the actor
    /// processes commands serially, so a command couldn't reach a runaway pass. The
    /// cancelled pass returns cleanly (intents left `InProgress`), and the executor
    /// resets the flag at the start of every pass, so later executes run normally.
    pub fn cancel_execution(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// The actor: owns the planner + executor and serializes command processing.
pub struct ScrubberActor<C: LastFmEditClient> {
    planner: Planner,
    executor: Executor<C>,
    state: Arc<dyn ScrubberState>,
    commands: mpsc::Receiver<ScrubberCommand>,
    events: ScrubberEventBus,
}

impl<C: LastFmEditClient> ScrubberActor<C> {
    /// Wire an actor around a planner and executor. Their event buses are unified onto
    /// the returned handle's bus (pass the same bus to both constructors for one ordered
    /// stream — this asserts they already share it).
    pub fn new(
        planner: Planner,
        executor: Executor<C>,
        state: Arc<dyn ScrubberState>,
    ) -> (ScrubberHandle, Self) {
        let events = planner.event_bus();
        let (tx, rx) = mpsc::channel(64);
        (
            ScrubberHandle {
                commands: tx,
                events: events.clone(),
                cancel: executor.cancel_handle(),
            },
            Self {
                planner,
                executor,
                state,
                commands: rx,
                events,
            },
        )
    }

    /// Process commands until `Stop` or every handle is dropped.
    ///
    /// The future is `!Send` (it drives lastfm-edit operations); spawn it on a
    /// current-thread runtime with `tokio::task::spawn_local`.
    pub async fn run(mut self) {
        while let Some(command) = self.commands.recv().await {
            let stop = matches!(command, ScrubberCommand::Stop);
            if let Err(err) = self.dispatch(command).await {
                // The actor outlives individual command failures; report and continue.
                self.events.emit(ScrubberEvent::Error {
                    error: err.to_string(),
                });
            }
            if stop {
                break;
            }
        }
        self.events.emit(ScrubberEvent::Stopped {
            reason: "actor shut down".into(),
        });
    }

    async fn dispatch(&mut self, command: ScrubberCommand) -> Result<()> {
        match command {
            ScrubberCommand::Consider(records) => {
                self.planner.plan_records(&records).await?;
            }
            ScrubberCommand::PlanFeed(feed) => {
                self.planner.plan(&feed).await?;
            }
            ScrubberCommand::Approve(id) => {
                crate::ops::approve_intent(self.state.as_ref(), id).await?;
                self.events.emit(ScrubberEvent::IntentApproved { id });
            }
            ScrubberCommand::Reject { id, dismiss } => {
                crate::ops::reject_intent(self.state.as_ref(), id, dismiss).await?;
                self.events.emit(ScrubberEvent::IntentRejected {
                    id,
                    dismissed: dismiss,
                });
            }
            ScrubberCommand::Reinstate(id) => {
                crate::ops::reinstate_intent(self.state.as_ref(), id).await?;
                self.events.emit(ScrubberEvent::IntentReinstated { id });
            }
            ScrubberCommand::ExecuteOnce { max_edits } => {
                self.executor.run_once_with_budget(max_edits).await?;
            }
            ScrubberCommand::ApproveRule(id) => {
                crate::ops::approve_pending_rule(self.state.as_ref(), id).await?;
            }
            ScrubberCommand::RejectRule(id) => {
                crate::ops::reject_pending_rule(self.state.as_ref(), id).await?;
            }
            ScrubberCommand::Stop => {}
        }
        Ok(())
    }
}

/// Bridge the store's sync events into the scrubber: newly-discovered scrobbles become
/// `Consider` commands (low-latency reaction), and every sync event is forwarded onto the
/// scrubber bus as [`ScrubberEvent::Sync`] so consumers get one ordered stream.
///
/// The broadcast channel is lossy; on lag the bridge just continues — the coverage-driven
/// `Incremental` feed is the durable reconciler that catches anything dropped here.
pub async fn bridge_sync_events(
    mut sync_events: SyncEventReceiver,
    store: Arc<dyn Storage>,
    handle: ScrubberHandle,
) {
    loop {
        let event = match sync_events.recv().await {
            Ok(event) => event,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                log::warn!(
                    "sync bridge lagged {missed} events; incremental planning will reconcile"
                );
                continue;
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
        };

        if let SyncEvent::ScrobblesDiscovered {
            new,
            oldest: Some(oldest),
            newest: Some(newest),
            ..
        } = &event
        {
            if *new > 0 {
                match store
                    .scrobbles_in_range(*oldest..newest.saturating_add(1))
                    .await
                {
                    Ok(records) if !records.is_empty() => {
                        if handle
                            .send(ScrubberCommand::Consider(records))
                            .await
                            .is_err()
                        {
                            break; // actor gone
                        }
                    }
                    Ok(_) => {}
                    Err(err) => log::warn!("sync bridge: failed to load records: {err}"),
                }
            }
        }

        handle.event_bus().emit(ScrubberEvent::Sync(event));
    }
}
