//! Backend wiring: the store + scrubber stack runs on a dedicated thread with its own
//! current-thread tokio runtime (the CLI's shape), because the Dioxus main-thread
//! executor stops polling tasks while the window is hidden on Wayland. The UI keeps only
//! Send handles: [`ScrubberHandle`] for scrubber commands, a [`BackendCommand`] channel
//! for sync/continuous control, and `Arc` storage for read-only queries.

use lastfm_edit::LastFmEditClientImpl;
use scrobble_scrubber::{
    approve_intent, load_comprehensive_default_rules, reinstate_intent, reject_intent, Executor,
    ExecutorOptions, FsScrubberState, Planner, RewriteRulesScrubActionProvider, ScrubFeed,
    ScrubberActor, ScrubberCommand, ScrubberEvent, ScrubberEventBus, ScrubberHandle, ScrubberState,
};
use scrobble_store::{ApiSource, FsStorage, Storage, SyncEngine, SyncEventReceiver};
use serde::Deserialize;
use std::cell::Cell;
use std::path::PathBuf;
use std::process::Command;
use std::rc::Rc;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

/// Why the app can't start; rendered as a full-page notice.
#[derive(Clone, Debug)]
pub enum StartupError {
    NoUsername,
    NoSession(String),
    Other(String),
}

impl std::fmt::Display for StartupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StartupError::NoUsername => {
                write!(f, "no username — set LASTFM_EDIT_USERNAME")
            }
            StartupError::NoSession(msg) => write!(
                f,
                "no saved last.fm session ({msg}) — log in once with the lastfm-edit CLI"
            ),
            StartupError::Other(msg) => write!(f, "{msg}"),
        }
    }
}

/// Control messages for the backend thread (things a [`ScrubberCommand`] can't express).
#[derive(Clone, Debug)]
pub enum BackendCommand {
    /// Extend sync coverage to now (no-op without an API key).
    SyncNow,
    /// Turn the sync → plan → execute loop on/off.
    SetContinuous { enabled: bool, interval_secs: u64 },
    /// Interrupt an in-flight execute pass (out-of-band — the actor's command channel
    /// is serial, so a `ScrubberCommand` would sit behind the runaway pass).
    StopExecution,
    /// Release an awaiting-approval intent. Runs directly against the durable queue
    /// (not the actor's serial channel), so it stays responsive during an execute pass.
    Approve(Uuid),
    /// Decline an open intent, optionally dismissing its subject. Same out-of-band path.
    Reject { id: Uuid, dismiss: bool },
    /// Un-reject a rejected intent. Same out-of-band path.
    Reinstate(Uuid),
    /// Seed the embedded cleanup rules, then relaunch so the planner is rebuilt with them.
    EnableDefaultRules,
}

/// Everything the UI needs to talk to the backend. All Send; lives in dioxus context.
#[derive(Clone)]
pub struct AppCore {
    pub store: Arc<FsStorage>,
    pub state: Arc<FsScrubberState>,
    pub handle: ScrubberHandle,
    pub backend: mpsc::Sender<BackendCommand>,
    pub username: String,
    pub store_root: PathBuf,
    /// False when LASTFM_EDIT_API_KEY is unset; sync controls disabled.
    pub sync_available: bool,
    pub rules_empty: bool,
    /// Hash of the active rule set (None when empty) — for spotting stale planning
    /// coverage in the UI before the planner resets it.
    pub rules_hash: Option<String>,
}

/// Built on the backend thread; `!Send` parts never leave it.
struct BackendParts {
    core: AppCore,
    actor: ScrubberActor<LastFmEditClientImpl>,
    sync: Option<Rc<SyncEngine>>,
    sync_events: Option<SyncEventReceiver>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct FileConfig {
    store: StoreSection,
    executor: ExecutorSection,
    scrubber: ScrubberSection,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct StoreSection {
    root: Option<PathBuf>,
    state_dir: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
struct ExecutorSection {
    inter_edit_delay_secs: u64,
    max_attempts_per_instance: u32,
    max_rate_limit_pauses_per_pass: u32,
}

impl Default for ExecutorSection {
    fn default() -> Self {
        Self {
            inter_edit_delay_secs: 2,
            max_attempts_per_instance: 3,
            max_rate_limit_pauses_per_pass: 3,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default)]
struct ScrubberSection {
    batch_size: usize,
}

impl Default for ScrubberSection {
    fn default() -> Self {
        Self { batch_size: 50 }
    }
}

fn pass_entry() -> String {
    std::env::var("LASTFM_EDIT_PASS_ENTRY").unwrap_or_else(|_| "last.fm".to_string())
}

fn parse_pass_field(output: &str, field: Option<&str>) -> Option<String> {
    match field {
        None => output.lines().next(),
        Some(field) => output.lines().skip(1).find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.trim().eq_ignore_ascii_case(field).then(|| value.trim())
        }),
    }
    .filter(|value| !value.is_empty())
    .map(str::to_string)
}

fn credential_from_pass(field: Option<&str>) -> Result<Option<String>, String> {
    let entry = pass_entry();
    let output = Command::new("pass")
        .args(["show", &entry])
        .output()
        .map_err(|error| format!("could not run pass: {error}"))?;
    if !output.status.success() {
        return Err(format!("pass could not read entry {entry}"));
    }
    let output = String::from_utf8(output.stdout)
        .map_err(|_| format!("pass entry {entry} is not valid UTF-8"))?;
    Ok(parse_pass_field(&output, field))
}

fn env_or_pass(name: &str, pass_field: Option<&str>) -> Result<Option<String>, String> {
    match std::env::var(name) {
        Ok(value) if !value.is_empty() => Ok(Some(value)),
        _ => credential_from_pass(pass_field),
    }
}

fn load_config() -> FileConfig {
    let path = std::env::var("SCROBBLE_SCRUBBER_CONFIG")
        .map(PathBuf::from)
        .ok()
        .or_else(|| {
            dirs::config_dir().map(|dir| dir.join("scrobble-scrubber").join("config.toml"))
        });
    match path {
        Some(path) if path.exists() => std::fs::read_to_string(&path)
            .ok()
            .and_then(|content| toml::from_str(&content).ok())
            .unwrap_or_default(),
        _ => FileConfig::default(),
    }
}

/// Spawn the backend thread. The receiver resolves once the stack is up (or failed);
/// a dropped sender means the backend thread died before reporting.
pub fn start() -> oneshot::Receiver<Result<AppCore, StartupError>> {
    let (ready_tx, ready_rx) = oneshot::channel();
    let (command_tx, command_rx) = mpsc::channel::<BackendCommand>(16);

    std::thread::Builder::new()
        .name("scrubber-backend".into())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("backend tokio runtime");
            let local = tokio::task::LocalSet::new();
            local.block_on(&runtime, backend_main(command_tx, command_rx, ready_tx));
        })
        .expect("spawn backend thread");

    ready_rx
}

async fn backend_main(
    command_tx: mpsc::Sender<BackendCommand>,
    mut commands: mpsc::Receiver<BackendCommand>,
    ready: oneshot::Sender<Result<AppCore, StartupError>>,
) {
    let parts = match build(command_tx).await {
        Ok(parts) => parts,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    let BackendParts {
        core,
        actor,
        sync,
        sync_events,
    } = parts;

    let handle = core.handle.clone();
    let store = core.store.clone();
    let state = core.state.clone();
    if ready.send(Ok(core)).is_err() {
        return; // UI gone before boot finished
    }

    tokio::task::spawn_local(actor.run());
    if let Some(events) = sync_events {
        tokio::task::spawn_local(scrobble_scrubber::bridge_sync_events(
            events,
            store as Arc<dyn Storage>,
            handle.clone(),
        ));
    }

    let continuous = Rc::new(Cell::new(false));
    let interval_secs = Rc::new(Cell::new(300u64));
    tokio::task::spawn_local(continuous_loop(
        continuous.clone(),
        interval_secs.clone(),
        sync.clone(),
        handle.clone(),
    ));

    while let Some(command) = commands.recv().await {
        match command {
            BackendCommand::SyncNow => {
                if let Some(engine) = &sync {
                    let engine = engine.clone();
                    tokio::task::spawn_local(async move {
                        if let Err(error) = engine.extend_to_present().await {
                            tracing::warn!(%error, "sync failed");
                        }
                    });
                }
            }
            BackendCommand::SetContinuous {
                enabled,
                interval_secs: secs,
            } => {
                tracing::info!(enabled, interval_secs = secs, "continuous mode");
                continuous.set(enabled);
                interval_secs.set(secs.max(10));
                if !enabled {
                    // Toggling off must also interrupt an in-flight execute, not just
                    // future iterations.
                    handle.cancel_execution();
                }
            }
            BackendCommand::StopExecution => {
                tracing::info!("stop execution requested");
                handle.cancel_execution();
            }
            // Queue mutations run as their own tasks against the shared state, off the
            // actor's serial command channel, so they resolve during an execute pass's
            // await points instead of queueing behind it. Appends are atomic and the fold
            // is order-tolerant, so this is safe alongside the executor's own appends.
            BackendCommand::Approve(id) => {
                let state = state.clone();
                let handle = handle.clone();
                tokio::task::spawn_local(async move {
                    match approve_intent(state.as_ref(), id).await {
                        Ok(()) => handle
                            .event_bus()
                            .emit(ScrubberEvent::IntentApproved { id }),
                        Err(error) => tracing::warn!(%error, %id, "approve failed"),
                    }
                });
            }
            BackendCommand::Reject { id, dismiss } => {
                let state = state.clone();
                let handle = handle.clone();
                tokio::task::spawn_local(async move {
                    match reject_intent(state.as_ref(), id, dismiss).await {
                        Ok(()) => handle.event_bus().emit(ScrubberEvent::IntentRejected {
                            id,
                            dismissed: dismiss,
                        }),
                        Err(error) => tracing::warn!(%error, %id, "reject failed"),
                    }
                });
            }
            BackendCommand::Reinstate(id) => {
                let state = state.clone();
                let handle = handle.clone();
                tokio::task::spawn_local(async move {
                    match reinstate_intent(state.as_ref(), id).await {
                        Ok(()) => handle
                            .event_bus()
                            .emit(ScrubberEvent::IntentReinstated { id }),
                        Err(error) => tracing::warn!(%error, %id, "reinstate failed"),
                    }
                });
            }
            BackendCommand::EnableDefaultRules => {
                match enable_default_rules(state.as_ref()).await {
                    Ok(count) => {
                        tracing::info!(count, "default rules enabled; relaunching");
                        match std::env::current_exe().and_then(|exe| Command::new(exe).spawn()) {
                            Ok(_) => std::process::exit(0),
                            Err(error) => tracing::warn!(%error, "could not relaunch app"),
                        }
                    }
                    Err(error) => tracing::warn!(%error, "could not enable default rules"),
                }
            }
        }
    }
}

async fn enable_default_rules(state: &FsScrubberState) -> scrobble_scrubber::Result<usize> {
    let mut rules = state.load_rules().await?;
    let existing: std::collections::HashSet<_> =
        rules.iter().filter_map(|rule| rule.name.clone()).collect();
    for rule in load_comprehensive_default_rules() {
        if rule
            .name
            .as_ref()
            .is_none_or(|name| !existing.contains(name))
        {
            rules.push(rule);
        }
    }
    let count = rules.len();
    state.save_rules(&rules).await?;
    Ok(count)
}

/// While enabled: sync (when available) → plan incremental → execute, then wait the
/// interval. 1s ticks so toggling off takes effect quickly.
///
/// Emits `CycleStarted`/`CycleCompleted`/`Sleeping` so continuous mode is visible in
/// the UI. Caveat: "completed" here means the cycle's work was *enqueued* — the actor
/// serializes the actual plan/execute; the pass events show real progress.
async fn continuous_loop(
    enabled: Rc<Cell<bool>>,
    interval_secs: Rc<Cell<u64>>,
    sync: Option<Rc<SyncEngine>>,
    handle: ScrubberHandle,
) {
    let mut next_run = 0u64;
    let mut was_enabled = false;
    let mut cycle = 0u64;
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        if !enabled.get() {
            if was_enabled {
                // Disabled mid-cycle: interrupt any execute still in flight too.
                handle.cancel_execution();
                was_enabled = false;
            }
            next_run = 0; // run immediately on re-enable
            continue;
        }
        was_enabled = true;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if now < next_run {
            continue;
        }
        next_run = now + interval_secs.get();
        tracing::debug!(cycle, "continuous: starting cycle");
        handle
            .event_bus()
            .emit(ScrubberEvent::CycleStarted { n: cycle });

        if let Some(engine) = &sync {
            if let Err(error) = engine.extend_to_present().await {
                tracing::warn!(%error, "continuous: sync failed");
            }
        }
        // The actor serializes these; enqueueing is enough.
        if handle
            .send(ScrubberCommand::PlanFeed(ScrubFeed::Incremental {
                window: None,
            }))
            .await
            .is_err()
        {
            break; // actor gone
        }
        if handle
            .send(ScrubberCommand::ExecuteOnce { max_edits: None })
            .await
            .is_err()
        {
            break;
        }
        handle
            .event_bus()
            .emit(ScrubberEvent::CycleCompleted { n: cycle });
        handle.event_bus().emit(ScrubberEvent::Sleeping {
            seconds: interval_secs.get(),
        });
        cycle += 1;
    }
}

/// Build the full backend stack; runs on the backend thread.
async fn build(command_tx: mpsc::Sender<BackendCommand>) -> Result<BackendParts, StartupError> {
    let config = load_config();

    let username = env_or_pass("LASTFM_EDIT_USERNAME", Some("user"))
        .map_err(StartupError::Other)?
        .ok_or(StartupError::NoUsername)?;

    let store_root = std::env::var("SCROBBLE_STORE_DIR")
        .map(PathBuf::from)
        .ok()
        .or_else(|| config.store.root.clone())
        .or_else(|| dirs::data_dir().map(|dir| dir.join("scrobble-store").join(&username)))
        .ok_or_else(|| StartupError::Other("cannot determine store root".into()))?;
    let state_dir = config
        .store
        .state_dir
        .clone()
        .unwrap_or_else(|| store_root.join("scrubber"));

    let store =
        Arc::new(FsStorage::open(&store_root).map_err(|e| StartupError::Other(e.to_string()))?);
    let state =
        Arc::new(FsScrubberState::open(state_dir).map_err(|e| StartupError::Other(e.to_string()))?);

    // Rules: empty is allowed (planner just finds nothing); surface a banner instead.
    let rules = state
        .load_rules()
        .await
        .map_err(|e| StartupError::Other(e.to_string()))?;
    let rules_empty = rules.is_empty();
    let active_rules_hash = (!rules_empty).then(|| scrobble_scrubber::rules_hash(&rules));

    // Edit client: prefer the saved session, but bootstrap it from pass when absent.
    // The Nix app wrapper supplies pass on PATH even when launched from Finder.
    let client = match lastfm_edit::SessionPersistence::load_session(&username) {
        Ok(session) => LastFmEditClientImpl::from_session(
            Box::new(http_client::native::NativeClient::new()),
            session,
        ),
        Err(session_error) => {
            let password = env_or_pass("LASTFM_EDIT_PASSWORD", None)
                .map_err(StartupError::NoSession)?
                .ok_or_else(|| StartupError::NoSession(session_error.to_string()))?;
            let client = LastFmEditClientImpl::login_with_credentials(
                Box::new(http_client::native::NativeClient::new()),
                &username,
                &password,
            )
            .await
            .map_err(|error| StartupError::NoSession(error.to_string()))?;
            lastfm_edit::SessionPersistence::save_session(&client.get_session())
                .map_err(|error| StartupError::NoSession(error.to_string()))?;
            client
        }
    }
    .non_blocking();

    let bus = ScrubberEventBus::new();
    let planner = Planner::new(
        store.clone() as Arc<dyn Storage>,
        state.clone() as Arc<dyn ScrubberState>,
    )
    .with_provider(RewriteRulesScrubActionProvider::from_rules(rules))
    .with_batch_hint(config.scrubber.batch_size)
    .with_event_bus(bus.clone());

    let executor = Executor::new(
        store.clone() as Arc<dyn Storage>,
        state.clone() as Arc<dyn ScrubberState>,
        client,
    )
    .with_options(ExecutorOptions {
        inter_edit_delay: std::time::Duration::from_secs(config.executor.inter_edit_delay_secs),
        max_edits: None,
        max_attempts_per_instance: config.executor.max_attempts_per_instance,
        max_rate_limit_pauses_per_pass: config.executor.max_rate_limit_pauses_per_pass,
    })
    .with_event_bus(bus);

    let (handle, actor) =
        ScrubberActor::new(planner, executor, state.clone() as Arc<dyn ScrubberState>);

    // Sync engine only when an API key is available.
    let api_key =
        env_or_pass("LASTFM_EDIT_API_KEY", Some("api-key")).map_err(StartupError::Other)?;
    let (sync, sync_events) = match api_key {
        Some(api_key) => {
            let api_client = lastfm_edit::LastFmApiClientImpl::new(
                Box::new(http_client::native::NativeClient::new()),
                username.clone(),
                api_key,
            );
            let engine = SyncEngine::new(
                store.clone() as Arc<dyn Storage>,
                Arc::new(ApiSource::new(api_client)),
            );
            let events = engine.subscribe();
            (Some(Rc::new(engine)), Some(events))
        }
        _ => (None, None),
    };

    Ok(BackendParts {
        core: AppCore {
            store,
            state,
            handle,
            backend: command_tx,
            username,
            store_root,
            sync_available: sync.is_some(),
            rules_empty,
            rules_hash: active_rules_hash,
        },
        actor,
        sync,
        sync_events,
    })
}

#[cfg(test)]
mod tests {
    use super::parse_pass_field;

    const ENTRY: &str = "secret\nuser: IvanMalison\nurl: https://last.fm\napi-key: key\n";

    #[test]
    fn parses_password_from_first_line() {
        assert_eq!(parse_pass_field(ENTRY, None).as_deref(), Some("secret"));
    }

    #[test]
    fn parses_named_fields_case_insensitively() {
        assert_eq!(
            parse_pass_field(ENTRY, Some("USER")).as_deref(),
            Some("IvanMalison")
        );
        assert_eq!(
            parse_pass_field(ENTRY, Some("api-key")).as_deref(),
            Some("key")
        );
    }
}
