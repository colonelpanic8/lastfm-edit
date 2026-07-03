//! scrobble-scrubber CLI: plan metadata edits from the local scrobble store, review the
//! intent queue, and execute edits against last.fm through a rate-limit-paced lane.

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use clap::{Parser, Subcommand};
use scrobble_scrubber::{
    approve_intent, approve_pending_rule, load_comprehensive_default_rules, reject_intent,
    reject_pending_rule, Executor, ExecutorOptions, FsScrubberState, IntentState, Planner, Policy,
    RewriteRulesScrubActionProvider, ScrubFeed, ScrubberEvent, ScrubberEventBus, ScrubberState,
};
use scrobble_store::{ApiSource, FsStorage, ScrobbleId, Storage, SyncEngine};
use serde::Deserialize;
use std::io::Write as _;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Parser)]
#[command(
    name = "scrobble-scrubber",
    about = "Rule-driven cleanup of last.fm scrobble metadata over a local scrobble-store",
    version
)]
struct Cli {
    /// Config file (default: ~/.config/scrobble-scrubber/config.toml)
    #[arg(long, global = true, env = "SCROBBLE_SCRUBBER_CONFIG")]
    config: Option<PathBuf>,

    /// scrobble-store root (default: ~/.local/share/scrobble-store/<username>)
    #[arg(long, global = true, env = "SCROBBLE_STORE_DIR")]
    store_root: Option<PathBuf>,

    /// Last.fm username
    #[arg(long, global = true, env = "LASTFM_EDIT_USERNAME")]
    username: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Analyze scrobbles and queue edit intents (never touches last.fm)
    Plan {
        #[command(subcommand)]
        feed: FeedCommand,
        /// Report suggestions without queueing anything
        #[arg(long, global = true)]
        dry_run: bool,
        /// Queue every suggestion as awaiting approval
        #[arg(long, global = true)]
        require_confirmation: bool,
    },
    /// Drain ready intents through last.fm (the paced lane)
    Execute {
        /// Stop after this many upstream edit attempts
        #[arg(long)]
        max_edits: Option<u32>,
        /// Keep draining until cancelled instead of one pass
        #[arg(long)]
        follow: bool,
    },
    /// Continuous mode: sync the store, plan incrementally, and execute — concurrently
    Run {
        /// Seconds between sync+plan cycles
        #[arg(long, default_value_t = 300)]
        interval: u64,
    },
    /// Inspect and manage the edit-intent queue
    Queue {
        #[command(subcommand)]
        action: QueueCommand,
    },
    /// Manage active rewrite rules
    Rules {
        #[command(subcommand)]
        action: RulesCommand,
    },
    /// Review provider-proposed rewrite rules
    PendingRules {
        #[command(subcommand)]
        action: PendingRulesCommand,
    },
    /// Inspect or reset per-provider planning coverage
    Coverage {
        #[command(subcommand)]
        action: CoverageCommand,
    },
}

#[derive(Subcommand)]
enum FeedCommand {
    /// New work: synced time not yet planned
    Incremental,
    /// The whole store, or a time slice
    Store {
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
    },
    /// Every stored scrobble of one artist
    Artist { name: String },
    /// One album by one artist
    Album { artist: String, album: String },
    /// Explicit scrobble ids
    Ids { ids: Vec<String> },
}

#[derive(Subcommand)]
enum QueueCommand {
    /// List intents (optionally filtered by state)
    List {
        /// One of: awaiting, ready, in-progress, applied, rejected, abandoned
        #[arg(long)]
        state: Option<String>,
    },
    Show {
        id: Uuid,
    },
    Approve {
        id: Uuid,
    },
    Reject {
        id: Uuid,
        /// Also never suggest this subject again
        #[arg(long)]
        dismiss: bool,
    },
}

#[derive(Subcommand)]
enum RulesCommand {
    Show,
    /// Seed the active rule set with the embedded default corpus
    EnableDefaults,
    /// Import rules from a JSON file (array of RewriteRule)
    Import {
        path: PathBuf,
    },
    /// Remove a rule by (0-based) index shown in `rules show`
    Remove {
        index: usize,
    },
}

#[derive(Subcommand)]
enum PendingRulesCommand {
    List,
    Approve { id: Uuid },
    Reject { id: Uuid },
}

#[derive(Subcommand)]
enum CoverageCommand {
    Show {
        #[arg(long)]
        provider: Option<String>,
    },
    /// Forget planning coverage so ranges get re-analyzed
    Reset {
        #[arg(long)]
        provider: String,
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
    },
}

// =====================================================================================
// Config file
// =====================================================================================

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct FileConfig {
    scrubber: ScrubberSection,
    store: StoreSection,
    executor: ExecutorSection,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
struct ScrubberSection {
    interval: u64,
    dry_run: bool,
    require_confirmation: bool,
    batch_size: usize,
}

impl Default for ScrubberSection {
    fn default() -> Self {
        Self {
            interval: 300,
            dry_run: false,
            require_confirmation: false,
            batch_size: 50,
        }
    }
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
}

impl Default for ExecutorSection {
    fn default() -> Self {
        Self {
            inter_edit_delay_secs: 2,
            max_attempts_per_instance: 3,
        }
    }
}

fn load_config(path: Option<&PathBuf>) -> Result<FileConfig> {
    let path = match path {
        Some(path) => Some(path.clone()),
        None => dirs::config_dir().map(|dir| dir.join("scrobble-scrubber").join("config.toml")),
    };
    match path {
        Some(path) if path.exists() => {
            let content = std::fs::read_to_string(&path)?;
            Ok(toml::from_str(&content)
                .map_err(|e| format!("bad config {}: {e}", path.display()))?)
        }
        _ => Ok(FileConfig::default()),
    }
}

// =====================================================================================
// Main
// =====================================================================================

type Error = Box<dyn std::error::Error>;
type Result<T> = std::result::Result<T, Error>;

fn main() -> Result<()> {
    env_logger::init();
    let cli = Cli::parse();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let local = tokio::task::LocalSet::new();
    runtime.block_on(local.run_until(run(cli)))
}

struct Context {
    store: Arc<FsStorage>,
    state: Arc<FsScrubberState>,
    username: String,
    config: FileConfig,
}

fn context(cli: &Cli) -> Result<Context> {
    let config = load_config(cli.config.as_ref())?;
    let username = cli
        .username
        .clone()
        .ok_or("username required (--username or LASTFM_EDIT_USERNAME)")?;
    let store_root = cli
        .store_root
        .clone()
        .or_else(|| config.store.root.clone())
        .or_else(|| dirs::data_dir().map(|dir| dir.join("scrobble-store").join(&username)))
        .ok_or("cannot determine store root; pass --store-root")?;
    let state_dir = config
        .store
        .state_dir
        .clone()
        .unwrap_or_else(|| store_root.join("scrubber"));
    Ok(Context {
        store: Arc::new(FsStorage::open(&store_root)?),
        state: Arc::new(FsScrubberState::open(state_dir)?),
        username,
        config,
    })
}

async fn run(cli: Cli) -> Result<()> {
    let ctx = context(&cli)?;
    match &cli.command {
        Command::Plan {
            feed,
            dry_run,
            require_confirmation,
        } => plan(&ctx, feed, *dry_run, *require_confirmation).await,
        Command::Execute { max_edits, follow } => execute(&ctx, *max_edits, *follow).await,
        Command::Run { interval } => run_continuous(&ctx, *interval).await,
        Command::Queue { action } => queue_cmd(&ctx, action).await,
        Command::Rules { action } => rules_cmd(&ctx, action).await,
        Command::PendingRules { action } => pending_rules_cmd(&ctx, action).await,
        Command::Coverage { action } => coverage_cmd(&ctx, action).await,
    }
}

// =====================================================================================
// Plan
// =====================================================================================

fn parse_feed(feed: &FeedCommand) -> Result<ScrubFeed> {
    Ok(match feed {
        FeedCommand::Incremental => ScrubFeed::Incremental { window: None },
        FeedCommand::Store { from, to } => {
            let start = from.as_deref().map(parse_ts).transpose()?.unwrap_or(0);
            let end = to.as_deref().map(parse_ts).transpose()?.unwrap_or(u64::MAX);
            ScrubFeed::StoreRange {
                range: Some(start..end),
            }
        }
        FeedCommand::Artist { name } => ScrubFeed::Artist {
            name: name.clone(),
            range: None,
        },
        FeedCommand::Album { artist, album } => ScrubFeed::Album {
            artist: artist.clone(),
            album: album.clone(),
        },
        FeedCommand::Ids { ids } => ScrubFeed::Ids(
            ids.iter()
                .map(|raw| ScrobbleId::from_str(raw).map_err(|e| format!("bad id '{raw}': {e}")))
                .collect::<std::result::Result<Vec<_>, _>>()?,
        ),
    })
}

async fn build_planner(ctx: &Context, policy: Policy, bus: ScrubberEventBus) -> Result<Planner> {
    let rules = ctx.state.load_rules().await?;
    if rules.is_empty() {
        return Err(
            "no active rules — run `scrobble-scrubber rules enable-defaults` (or `rules import`)"
                .into(),
        );
    }
    Ok(Planner::new(
        ctx.store.clone(),
        ctx.state.clone() as Arc<dyn ScrubberState>,
    )
    .with_provider(RewriteRulesScrubActionProvider::from_rules(rules))
    .with_policy(policy)
    .with_batch_hint(ctx.config.scrubber.batch_size)
    .with_event_bus(bus))
}

async fn plan(
    ctx: &Context,
    feed: &FeedCommand,
    dry_run: bool,
    require_confirmation: bool,
) -> Result<()> {
    let policy = Policy {
        dry_run: dry_run || ctx.config.scrubber.dry_run,
        require_confirmation_all: require_confirmation || ctx.config.scrubber.require_confirmation,
        auto_approve_rules: false,
    };
    let bus = ScrubberEventBus::new();
    let planner = build_planner(ctx, policy, bus.clone()).await?;
    let printer = tokio::task::spawn_local(render_events(bus.subscribe()));

    let feed = parse_feed(feed)?;
    let report = planner.plan(&feed).await?;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await; // let the printer drain
    printer.abort();
    eprintln!();
    println!(
        "plan complete: {} subjects, {} suggestions → {} ready, {} awaiting approval, {} rules proposed{}",
        report.subjects_seen,
        report.suggestions,
        report.queued_ready,
        report.queued_awaiting_approval,
        report.rules_proposed,
        if report.reported_only > 0 {
            format!(", {} reported (dry run)", report.reported_only)
        } else {
            String::new()
        }
    );
    Ok(())
}

// =====================================================================================
// Execute
// =====================================================================================

fn build_edit_client(username: &str) -> Result<lastfm_edit::LastFmEditClientImpl> {
    let session = lastfm_edit::SessionPersistence::load_session(username).map_err(|e| {
        format!(
            "no saved lastfm-edit session for {username} ({e}); log in once with the lastfm-edit CLI"
        )
    })?;
    let client = lastfm_edit::LastFmEditClientImpl::from_session(
        Box::new(http_client::native::NativeClient::new()),
        session,
    );
    // Non-blocking: rate limits surface as errors so the executor owns all pacing.
    Ok(client.non_blocking())
}

async fn execute(ctx: &Context, max_edits: Option<u32>, follow: bool) -> Result<()> {
    let client = build_edit_client(&ctx.username)?;
    let bus = ScrubberEventBus::new();
    let executor = Executor::new(
        ctx.store.clone() as Arc<dyn Storage>,
        ctx.state.clone() as Arc<dyn ScrubberState>,
        client,
    )
    .with_options(ExecutorOptions {
        inter_edit_delay: std::time::Duration::from_secs(ctx.config.executor.inter_edit_delay_secs),
        max_edits,
        max_attempts_per_instance: ctx.config.executor.max_attempts_per_instance,
    })
    .with_event_bus(bus.clone());

    let cancel = executor.cancel_handle();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        eprintln!("\ncancelling after the current operation…");
    });

    let printer = tokio::task::spawn_local(render_events(bus.subscribe()));
    let mut totals = (0u64, 0u64);
    loop {
        let report = executor.run_once().await?;
        totals.0 += report.instances_applied;
        totals.1 += report.instances_failed;
        if !follow || report.intents_processed == 0 {
            if follow {
                tokio::time::sleep(std::time::Duration::from_secs(15)).await;
                continue;
            }
            break;
        }
    }
    printer.abort();
    eprintln!();
    println!(
        "execute complete: {} applied, {} failed",
        totals.0, totals.1
    );
    Ok(())
}

// =====================================================================================
// Continuous run
// =====================================================================================

async fn run_continuous(ctx: &Context, interval: u64) -> Result<()> {
    let api_key = std::env::var("LASTFM_EDIT_API_KEY")
        .map_err(|_| "LASTFM_EDIT_API_KEY must be set for continuous mode (store sync)")?;
    let api_client = lastfm_edit::LastFmApiClientImpl::new(
        Box::new(http_client::native::NativeClient::new()),
        ctx.username.clone(),
        api_key,
    );
    let source = Arc::new(ApiSource::new(api_client));
    let engine = SyncEngine::new(ctx.store.clone() as Arc<dyn Storage>, source);

    let bus = ScrubberEventBus::new();
    let edit_client = build_edit_client(&ctx.username)?;
    let executor = Executor::new(
        ctx.store.clone() as Arc<dyn Storage>,
        ctx.state.clone() as Arc<dyn ScrubberState>,
        edit_client,
    )
    .with_options(ExecutorOptions {
        inter_edit_delay: std::time::Duration::from_secs(ctx.config.executor.inter_edit_delay_secs),
        max_edits: None,
        max_attempts_per_instance: ctx.config.executor.max_attempts_per_instance,
    })
    .with_event_bus(bus.clone());
    let exec_cancel = executor.cancel_handle();

    let policy = Policy {
        dry_run: ctx.config.scrubber.dry_run,
        require_confirmation_all: ctx.config.scrubber.require_confirmation,
        auto_approve_rules: false,
    };
    let planner = build_planner(ctx, policy, bus.clone()).await?;

    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let stop = stop.clone();
        let exec_cancel = exec_cancel.clone();
        tokio::spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
            stop.store(true, std::sync::atomic::Ordering::Relaxed);
            exec_cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            eprintln!("\nstopping…");
        });
    }

    let printer = tokio::task::spawn_local(render_events(bus.subscribe()));

    // Executor task: drains whatever the planner queues, forever.
    let executor = Arc::new(executor);
    let exec_task = {
        let executor = executor.clone();
        let stop = stop.clone();
        tokio::task::spawn_local(async move {
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                match executor.run_once().await {
                    Ok(_) => {}
                    Err(scrobble_scrubber::ScrubberError::Cancelled) => break,
                    Err(err) => log::error!("executor: {err}"),
                }
                tokio::time::sleep(std::time::Duration::from_secs(15)).await;
            }
        })
    };

    // Sync + plan cycle.
    let mut cycle: u64 = 0;
    while !stop.load(std::sync::atomic::Ordering::Relaxed) {
        cycle += 1;
        bus.emit(ScrubberEvent::CycleStarted { n: cycle });
        if let Err(err) = engine.extend_to_present().await {
            log::error!("sync: {err}");
        }
        if let Err(err) = planner.plan(&ScrubFeed::Incremental { window: None }).await {
            log::error!("plan: {err}");
        }
        bus.emit(ScrubberEvent::CycleCompleted { n: cycle });
        bus.emit(ScrubberEvent::Sleeping { seconds: interval });
        let mut waited = 0;
        while waited < interval && !stop.load(std::sync::atomic::Ordering::Relaxed) {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            waited += 1;
        }
    }

    let _ = exec_task.await;
    printer.abort();
    eprintln!();
    println!("stopped after {cycle} cycle(s)");
    Ok(())
}

// =====================================================================================
// Queue / rules / coverage commands
// =====================================================================================

fn state_matches(state: &IntentState, filter: &str) -> bool {
    matches!(
        (state, filter),
        (IntentState::AwaitingApproval, "awaiting")
            | (IntentState::Ready, "ready")
            | (IntentState::InProgress, "in-progress")
            | (IntentState::Applied, "applied")
            | (IntentState::Rejected { .. }, "rejected")
            | (IntentState::Abandoned { .. }, "abandoned")
    )
}

async fn queue_cmd(ctx: &Context, action: &QueueCommand) -> Result<()> {
    match action {
        QueueCommand::List { state } => {
            let queue = ctx.state.load_queue().await?;
            let mut shown = 0;
            for intent in &queue {
                if let Some(filter) = state {
                    if !state_matches(&intent.state, filter) {
                        continue;
                    }
                }
                shown += 1;
                println!(
                    "{}  {:?}  [{}]  {}  ({} done / {} failed)",
                    intent.id,
                    intent.state,
                    intent.provider,
                    intent.subject,
                    intent.done_count(),
                    intent.failed_count(),
                );
            }
            if shown == 0 {
                println!(
                    "queue is empty{}",
                    state
                        .as_deref()
                        .map(|s| format!(" (state {s})"))
                        .unwrap_or_default()
                );
            }
        }
        QueueCommand::Show { id } => {
            let queue = ctx.state.load_queue().await?;
            let intent = queue
                .iter()
                .find(|intent| intent.id == *id)
                .ok_or("no such intent")?;
            println!("{}", serde_json::to_string_pretty(intent)?);
        }
        QueueCommand::Approve { id } => {
            approve_intent(ctx.state.as_ref(), *id).await?;
            println!("approved {id}");
        }
        QueueCommand::Reject { id, dismiss } => {
            reject_intent(ctx.state.as_ref(), *id, *dismiss).await?;
            println!(
                "rejected {id}{}",
                if *dismiss { " (subject dismissed)" } else { "" }
            );
        }
    }
    Ok(())
}

async fn rules_cmd(ctx: &Context, action: &RulesCommand) -> Result<()> {
    match action {
        RulesCommand::Show => {
            let rules = ctx.state.load_rules().await?;
            if rules.is_empty() {
                println!("no active rules — run `rules enable-defaults`");
            }
            for (index, rule) in rules.iter().enumerate() {
                println!(
                    "{index:>3}  {}{}{}",
                    rule.name.as_deref().unwrap_or("(unnamed)"),
                    if rule.requires_confirmation {
                        "  [confirm]"
                    } else {
                        ""
                    },
                    if rule.requires_musicbrainz_confirmation {
                        "  [musicbrainz]"
                    } else {
                        ""
                    },
                );
            }
        }
        RulesCommand::EnableDefaults => {
            let mut rules = ctx.state.load_rules().await?;
            let before = rules.len();
            let existing: std::collections::HashSet<_> =
                rules.iter().filter_map(|r| r.name.clone()).collect();
            for rule in load_comprehensive_default_rules() {
                if rule.name.as_ref().is_none_or(|n| !existing.contains(n)) {
                    rules.push(rule);
                }
            }
            ctx.state.save_rules(&rules).await?;
            println!("rules: {before} → {}", rules.len());
        }
        RulesCommand::Import { path } => {
            let imported: Vec<scrobble_scrubber::RewriteRule> =
                serde_json::from_str(&std::fs::read_to_string(path)?)?;
            let mut rules = ctx.state.load_rules().await?;
            let count = imported.len();
            rules.extend(imported);
            ctx.state.save_rules(&rules).await?;
            println!("imported {count} rule(s); {} active", rules.len());
        }
        RulesCommand::Remove { index } => {
            let mut rules = ctx.state.load_rules().await?;
            if *index >= rules.len() {
                return Err(format!("no rule at index {index}").into());
            }
            let removed = rules.remove(*index);
            ctx.state.save_rules(&rules).await?;
            println!(
                "removed '{}'",
                removed.name.as_deref().unwrap_or("(unnamed)")
            );
        }
    }
    Ok(())
}

async fn pending_rules_cmd(ctx: &Context, action: &PendingRulesCommand) -> Result<()> {
    match action {
        PendingRulesCommand::List => {
            let pending = ctx.state.load_pending_rules().await?;
            let open: Vec<_> = pending
                .iter()
                .filter(|rule| rule.state == scrobble_scrubber::PendingRuleState::Open)
                .collect();
            if open.is_empty() {
                println!("no pending rules");
            }
            for rule in open {
                println!(
                    "{}  [{}]  {}  — {}",
                    rule.id,
                    rule.provider,
                    rule.rule.name.as_deref().unwrap_or("(unnamed)"),
                    rule.motivation,
                );
            }
        }
        PendingRulesCommand::Approve { id } => {
            approve_pending_rule(ctx.state.as_ref(), *id).await?;
            println!("approved {id} — merged into active rules (history will re-plan)");
        }
        PendingRulesCommand::Reject { id } => {
            reject_pending_rule(ctx.state.as_ref(), *id).await?;
            println!("rejected {id}");
        }
    }
    Ok(())
}

async fn coverage_cmd(ctx: &Context, action: &CoverageCommand) -> Result<()> {
    match action {
        CoverageCommand::Show { provider } => {
            let providers = match provider {
                Some(name) => vec![name.clone()],
                None => vec!["rewrite_rules".to_string()],
            };
            for name in providers {
                let coverage = ctx.state.load_provider_coverage(&name).await?;
                println!("{name}:");
                if coverage.coverage.is_empty() {
                    println!("  (no planning coverage)");
                }
                for segment in coverage.coverage.segments() {
                    println!("  {} .. {}", fmt_ts(segment.start), fmt_ts(segment.end));
                }
            }
        }
        CoverageCommand::Reset { provider, from, to } => {
            let mut coverage = ctx.state.load_provider_coverage(provider).await?;
            match (from, to) {
                (None, None) => {
                    coverage.coverage = scrobble_store::CoverageMap::new();
                }
                (from, to) => {
                    let start = from.as_deref().map(parse_ts).transpose()?.unwrap_or(0);
                    let end = to.as_deref().map(parse_ts).transpose()?.unwrap_or(u64::MAX);
                    coverage.coverage.subtract(start..end);
                }
            }
            ctx.state
                .save_provider_coverage(provider, &coverage)
                .await?;
            println!("reset planning coverage for {provider}");
        }
    }
    Ok(())
}

// =====================================================================================
// Rendering & helpers
// =====================================================================================

async fn render_events(mut rx: scrobble_scrubber::ScrubberEventReceiver) {
    let mut paused_until: Option<u64> = None;
    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(1));
    loop {
        tokio::select! {
            event = rx.recv() => {
                let Ok(event) = event else { break };
                match event {
                    ScrubberEvent::SubjectsFound { count, .. } if count > 0 => {
                        status_line(&format!("analyzing {count} subject(s)…"));
                    }
                    ScrubberEvent::SuggestionReported { subject, summary, .. } => {
                        eprintln!("\rwould edit {subject}: {summary}");
                    }
                    ScrubberEvent::IntentQueued { subject, state, .. } => {
                        eprintln!(
                            "\rqueued {subject}{}",
                            if state == IntentState::AwaitingApproval { "  (awaiting approval)" } else { "" }
                        );
                    }
                    ScrubberEvent::EditApplied { subject, instance, .. } => {
                        eprintln!("\redited {subject} @ {instance}");
                    }
                    ScrubberEvent::EditFailed { subject, error, .. } => {
                        eprintln!("\rFAILED {subject}: {error}");
                    }
                    ScrubberEvent::ExecutorPaused {
                        reason: scrobble_store::PauseReason::RateLimited { until_estimate },
                    } => {
                        paused_until = until_estimate;
                    }
                    ScrubberEvent::ExecutorResumed => {
                        paused_until = None;
                        status_line("resumed");
                    }
                    ScrubberEvent::CycleStarted { n } => status_line(&format!("cycle {n}: syncing…")),
                    ScrubberEvent::Sleeping { seconds } => status_line(&format!("idle, next cycle in {seconds}s")),
                    _ => {}
                }
            }
            _ = ticker.tick() => {
                if let Some(until) = paused_until {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    status_line(&format!("rate-limited, resuming in {}s…", until.saturating_sub(now)));
                }
            }
        }
    }
}

fn status_line(message: &str) {
    eprint!("\r\x1b[2K{message}");
    let _ = std::io::stderr().flush();
}

fn parse_ts(value: &str) -> Result<u64> {
    if let Ok(ts) = value.parse::<u64>() {
        return Ok(ts);
    }
    let date = NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| format!("cannot parse '{value}' as unix timestamp or YYYY-MM-DD"))?;
    Ok(Utc
        .from_utc_datetime(&date.and_hms_opt(0, 0, 0).expect("midnight is valid"))
        .timestamp() as u64)
}

fn fmt_ts(ts: u64) -> String {
    DateTime::<Utc>::from_timestamp(ts as i64, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| ts.to_string())
}
