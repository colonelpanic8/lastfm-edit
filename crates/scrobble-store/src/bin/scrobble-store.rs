//! scrobble-store CLI: synchronize, inspect, and query a local mirror of a Last.fm
//! scrobble history.

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use clap::{Parser, Subcommand, ValueEnum};
use scrobble_store::source::ScrobbleSource;
use scrobble_store::{
    ApiSource, EditState, FsStorage, PauseReason, ScrapeSource, Storage, SyncEngine, SyncEvent,
    SyncOptions,
};
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Parser)]
#[command(
    name = "scrobble-store",
    about = "Synchronizable local mirror of a Last.fm scrobble history",
    version
)]
struct Cli {
    /// Store directory (default: ~/.local/share/scrobble-store/<username>)
    #[arg(long, global = true, env = "SCROBBLE_STORE_DIR")]
    data_dir: Option<PathBuf>,

    /// Last.fm username
    #[arg(long, global = true, env = "LASTFM_EDIT_USERNAME")]
    username: Option<String>,

    /// Data source for sync operations
    #[arg(long, global = true, value_enum, default_value_t = Via::Api)]
    via: Via,

    /// Commit data changes to git after successful write commands
    #[arg(long, global = true)]
    git_commit: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Via {
    /// Official JSON API (needs LASTFM_EDIT_API_KEY); preferred for bulk sync
    Api,
    /// Web scraping via a saved lastfm-edit session
    Scrape,
}

#[derive(Subcommand)]
enum Command {
    /// Create the store directory skeleton (and optionally a git repository)
    Init {
        /// Also run `git init` in the store directory
        #[arg(long)]
        git: bool,
    },
    /// Extend coverage to the present, then fill interior gaps
    Sync {
        /// Skip the gap-filling phase
        #[arg(long)]
        no_gaps: bool,
        /// Stop after fetching this many pages
        #[arg(long)]
        max_pages: Option<u32>,
    },
    /// Fill historical gaps, newest first
    Backfill {
        /// Stop once history back to this date (YYYY-MM-DD or unix ts) is covered
        #[arg(long)]
        until: Option<String>,
        /// Stop after fetching this many pages
        #[arg(long)]
        max_pages: Option<u32>,
    },
    /// Show store statistics and sync state
    Status,
    /// Show coverage segments and gaps
    Coverage {
        #[arg(long)]
        json: bool,
    },
    /// Re-fetch a covered range and reconcile out-of-band changes
    Verify {
        #[arg(long)]
        from: String,
        #[arg(long)]
        to: String,
    },
    /// Remove a range from coverage so the next sync re-fetches it
    Invalidate {
        #[arg(long)]
        from: String,
        #[arg(long)]
        to: String,
    },
    /// Show the edit log
    Log {
        /// Only pending entries
        #[arg(long)]
        pending: bool,
        #[arg(long)]
        json: bool,
    },
    /// Rewrite storage keeping only current records (drops superseded lines)
    Compact,
    /// Drop and rebuild the derived query index
    Reindex,
    /// Indexed queries against the local mirror
    Query {
        #[command(subcommand)]
        query: QueryCommand,
    },
}

#[derive(Subcommand)]
enum QueryCommand {
    /// Most-scrobbled artists
    TopArtists {
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Most-scrobbled tracks, optionally within one artist
    TopTracks {
        #[arg(long)]
        artist: Option<String>,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Every stored scrobble of an artist
    Artist { name: String },
}

fn main() -> anyhow_lite::Result<()> {
    env_logger::init();
    let cli = Cli::parse();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let local = tokio::task::LocalSet::new();
    runtime.block_on(local.run_until(run(cli)))
}

/// Tiny local stand-in for anyhow so the CLI can bubble heterogeneous errors.
mod anyhow_lite {
    pub type Error = Box<dyn std::error::Error>;
    pub type Result<T> = std::result::Result<T, Error>;
}

async fn run(cli: Cli) -> anyhow_lite::Result<()> {
    let username = cli
        .username
        .clone()
        .ok_or("username required (--username or LASTFM_EDIT_USERNAME)")?;
    let data_dir = match &cli.data_dir {
        Some(dir) => dir.clone(),
        None => dirs::data_dir()
            .ok_or("cannot determine data directory; pass --data-dir")?
            .join("scrobble-store")
            .join(&username),
    };

    match &cli.command {
        Command::Init { git } => {
            let store = FsStorage::open(&data_dir)?;
            if *git && !data_dir.join(".git").exists() {
                git_run(&data_dir, &["init"])?;
            }
            println!("initialized store at {}", store.root().display());
            return Ok(());
        }
        Command::Status => return status(&data_dir).await,
        Command::Coverage { json } => return coverage(&data_dir, *json).await,
        Command::Log { pending, json } => return edit_log(&data_dir, *pending, *json).await,
        Command::Compact => {
            let store = FsStorage::open(&data_dir)?;
            let dropped = store.compact().await?;
            println!("compacted: dropped {dropped} superseded lines");
            maybe_commit(&cli, &data_dir, "compact")?;
            return Ok(());
        }
        Command::Reindex => {
            let store = FsStorage::open(&data_dir)?;
            store.reindex().await?;
            println!("index rebuilt");
            return Ok(());
        }
        Command::Query { query } => return run_query(&data_dir, query).await,
        Command::Invalidate { from, to } => {
            let store = Arc::new(FsStorage::open(&data_dir)?);
            let source = build_source(&cli, &username)?;
            let engine = SyncEngine::new(store, source);
            let range = parse_ts(from)?..parse_ts(to)?;
            engine.invalidate(range.clone()).await?;
            println!(
                "invalidated {} .. {}",
                fmt_ts(range.start),
                fmt_ts(range.end)
            );
            maybe_commit(&cli, &data_dir, "invalidate")?;
            return Ok(());
        }
        _ => {}
    }

    // Sync-flavored commands share the engine + progress rendering.
    let store = Arc::new(FsStorage::open(&data_dir)?);
    let source = build_source(&cli, &username)?;
    let max_pages = match &cli.command {
        Command::Sync { max_pages, .. } | Command::Backfill { max_pages, .. } => *max_pages,
        _ => None,
    };
    let engine = SyncEngine::with_options(
        store.clone(),
        source,
        SyncOptions {
            max_pages,
            ..SyncOptions::default()
        },
    );
    let printer = tokio::task::spawn_local(render_events(engine.subscribe()));

    let result: anyhow_lite::Result<String> = match &cli.command {
        Command::Sync { no_gaps, .. } => {
            let mut stats = engine.extend_to_present().await?;
            if !*no_gaps {
                let gap_stats = engine.fill_gaps(None).await?;
                stats.pages_fetched += gap_stats.pages_fetched;
                stats.scrobbles_new += gap_stats.scrobbles_new;
                stats.scrobbles_updated += gap_stats.scrobbles_updated;
            }
            Ok(format!(
                "sync complete: {} pages, {} new, {} updated",
                stats.pages_fetched, stats.scrobbles_new, stats.scrobbles_updated
            ))
        }
        Command::Backfill { until, .. } => {
            let until = until.as_ref().map(|value| parse_ts(value)).transpose()?;
            let stats = engine.backfill(until).await?;
            Ok(format!(
                "backfill complete: {} pages, {} new ({} seconds of timeline covered)",
                stats.pages_fetched, stats.scrobbles_new, stats.seconds_covered
            ))
        }
        Command::Verify { from, to } => {
            let report = engine.verify(parse_ts(from)?..parse_ts(to)?).await?;
            Ok(format!(
                "verify complete: {} upstream, {} written, {} tombstoned",
                report.upstream_count, report.written, report.tombstoned
            ))
        }
        _ => unreachable!("handled above"),
    };

    printer.abort();
    eprintln!();
    match result {
        Ok(message) => {
            println!("{message}");
            let command_name = match &cli.command {
                Command::Sync { .. } => "sync",
                Command::Backfill { .. } => "backfill",
                Command::Verify { .. } => "verify",
                _ => "update",
            };
            maybe_commit(&cli, &data_dir, command_name)?;
            Ok(())
        }
        Err(err) => Err(err),
    }
}

fn build_source(cli: &Cli, username: &str) -> anyhow_lite::Result<Arc<dyn ScrobbleSource>> {
    match cli.via {
        Via::Api => {
            let api_key = std::env::var("LASTFM_EDIT_API_KEY")
                .map_err(|_| "LASTFM_EDIT_API_KEY must be set for --via api")?;
            let client = lastfm_edit::LastFmApiClientImpl::new(
                Box::new(http_client::native::NativeClient::new()),
                username.to_string(),
                api_key,
            );
            Ok(Arc::new(ApiSource::new(client)))
        }
        Via::Scrape => {
            let session = lastfm_edit::SessionPersistence::load_session(username)
                .map_err(|e| format!(
                    "no saved lastfm-edit session for {username} ({e}); log in once with the lastfm-edit CLI first"
                ))?;
            let client = lastfm_edit::LastFmEditClientImpl::from_session(
                Box::new(http_client::native::NativeClient::new()),
                session,
            );
            Ok(Arc::new(ScrapeSource::new(client)))
        }
    }
}

/// Render sync progress on stderr: one `\r`-rewritten status line, with a live countdown
/// while rate-limited.
async fn render_events(mut rx: scrobble_store::SyncEventReceiver) {
    let mut paused_until: Option<u64> = None;
    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(1));
    loop {
        tokio::select! {
            event = rx.recv() => {
                let Ok(event) = event else { break };
                match event {
                    SyncEvent::PageFetched { window, page, count } => {
                        status_line(&format!(
                            "page {page}  [{} .. {}]  {count} tracks",
                            fmt_ts(window.start),
                            fmt_ts(window.end)
                        ));
                    }
                    SyncEvent::ScrobblesDiscovered { new, updated, .. } if new + updated > 0 => {
                        status_line(&format!("  +{new} new, {updated} updated"));
                    }
                    SyncEvent::SyncPaused { reason } => {
                        paused_until = match reason {
                            PauseReason::RateLimited { until_estimate } => until_estimate,
                            PauseReason::Backoff { delay_ms } => Some(now() + delay_ms / 1000),
                        };
                    }
                    SyncEvent::SyncResumed => {
                        paused_until = None;
                        status_line("resumed");
                    }
                    _ => {}
                }
            }
            _ = ticker.tick() => {
                if let Some(until) = paused_until {
                    let remaining = until.saturating_sub(now());
                    status_line(&format!("rate-limited, resuming in {remaining}s..."));
                }
            }
        }
    }
}

fn status_line(message: &str) {
    eprint!("\r\x1b[2K{message}");
    let _ = std::io::stderr().flush();
}

async fn status(data_dir: &PathBuf) -> anyhow_lite::Result<()> {
    let store = FsStorage::open(data_dir)?;
    let all = store.scrobbles_in_range(0..u64::MAX).await?;
    let coverage = store.load_coverage().await?;
    let sync_state = store.load_sync_state().await?;
    let log = store.load_edit_log().await?;
    let pending = log.iter().filter(|e| e.state.is_pending()).count();

    println!("store:        {}", data_dir.display());
    println!("scrobbles:    {}", all.len());
    if let (Some(first), Some(last)) = (all.first(), all.last()) {
        println!("oldest:       {}", fmt_ts(first.uts));
        println!("newest:       {}", fmt_ts(last.uts));
    }
    println!("coverage:     {} segment(s)", coverage.segments().len());
    if let Some(last) = coverage.last() {
        let staleness = now().saturating_sub(last.end);
        println!(
            "synced up to: {} ({staleness}s behind now)",
            fmt_ts(last.end)
        );
    }
    if let Some(start) = sync_state.history_start_uts {
        println!("history from: {}", fmt_ts(start));
    }
    if let Some(at) = sync_state.last_sync_at {
        println!("last sync:    {}", fmt_ts(at));
    }
    println!("edits:        {} total, {pending} pending", log.len());
    Ok(())
}

async fn coverage(data_dir: &PathBuf, json: bool) -> anyhow_lite::Result<()> {
    let store = FsStorage::open(data_dir)?;
    let coverage = store.load_coverage().await?;
    if json {
        println!("{}", serde_json::to_string_pretty(&coverage)?);
        return Ok(());
    }
    if coverage.is_empty() {
        println!("no coverage yet — run `scrobble-store sync`");
        return Ok(());
    }
    println!("covered segments:");
    for seg in coverage.segments() {
        println!(
            "  {} .. {}  (verified {})",
            fmt_ts(seg.start),
            fmt_ts(seg.end),
            fmt_ts(seg.verified_at)
        );
    }
    let bounds = coverage.first().unwrap().start..coverage.last().unwrap().end;
    let gaps = coverage.gaps(bounds);
    if gaps.is_empty() {
        println!("no interior gaps");
    } else {
        println!("gaps:");
        for gap in gaps {
            println!("  {} .. {}", fmt_ts(gap.start), fmt_ts(gap.end));
        }
    }
    Ok(())
}

async fn edit_log(data_dir: &PathBuf, pending_only: bool, json: bool) -> anyhow_lite::Result<()> {
    let store = FsStorage::open(data_dir)?;
    let mut entries = store.load_edit_log().await?;
    if pending_only {
        entries.retain(|e| e.state.is_pending());
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&entries)?);
        return Ok(());
    }
    if entries.is_empty() {
        println!("edit log is empty");
        return Ok(());
    }
    for entry in entries {
        let state = match &entry.state {
            EditState::Pending { attempts, .. } => format!("pending ({attempts} attempts)"),
            EditState::Applied { .. } => "applied".to_string(),
            EditState::Abandoned { reason } => format!("abandoned: {reason}"),
        };
        println!(
            "{}  {}  [{}]  {:?}",
            fmt_ts(entry.created_at),
            entry.edit_id,
            state,
            entry.op
        );
    }
    Ok(())
}

async fn run_query(data_dir: &PathBuf, query: &QueryCommand) -> anyhow_lite::Result<()> {
    let store = FsStorage::open(data_dir)?;
    match query {
        QueryCommand::TopArtists { limit } => {
            for entry in store.top_artists(*limit, None).await? {
                println!("{:>7}  {}", entry.count, entry.artist);
            }
        }
        QueryCommand::TopTracks { artist, limit } => {
            for entry in store.top_tracks(artist.as_deref(), *limit, None).await? {
                println!("{:>7}  {} — {}", entry.count, entry.artist, entry.track);
            }
        }
        QueryCommand::Artist { name } => {
            for rec in store.artist_scrobbles(name, None).await? {
                println!(
                    "{}  {} — {}{}",
                    fmt_ts(rec.uts),
                    rec.artist,
                    rec.track,
                    rec.album
                        .as_deref()
                        .map(|a| format!("  [{a}]"))
                        .unwrap_or_default()
                );
            }
        }
    }
    Ok(())
}

fn maybe_commit(cli: &Cli, data_dir: &PathBuf, action: &str) -> anyhow_lite::Result<()> {
    if !cli.git_commit {
        return Ok(());
    }
    if !data_dir.join(".git").exists() {
        eprintln!(
            "--git-commit: {} is not a git repository; skipping",
            data_dir.display()
        );
        return Ok(());
    }
    git_run(data_dir, &["add", "-A"])?;
    let status = std::process::Command::new("git")
        .current_dir(data_dir)
        .args(["diff", "--cached", "--quiet"])
        .status()?;
    if status.success() {
        return Ok(()); // nothing staged
    }
    git_run(
        data_dir,
        &["commit", "-m", &format!("scrobble-store: {action}")],
    )?;
    Ok(())
}

fn git_run(dir: &PathBuf, args: &[&str]) -> anyhow_lite::Result<()> {
    let status = std::process::Command::new("git")
        .current_dir(dir)
        .args(args)
        .status()?;
    if !status.success() {
        return Err(format!("git {args:?} failed with {status}").into());
    }
    Ok(())
}

fn parse_ts(value: &str) -> anyhow_lite::Result<u64> {
    if let Ok(ts) = value.parse::<u64>() {
        return Ok(ts);
    }
    let date = NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| format!("cannot parse '{value}' as unix timestamp or YYYY-MM-DD"))?;
    let dt = Utc.from_utc_datetime(&date.and_hms_opt(0, 0, 0).expect("midnight is valid"));
    Ok(dt.timestamp() as u64)
}

fn fmt_ts(ts: u64) -> String {
    DateTime::<Utc>::from_timestamp(ts as i64, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| ts.to_string())
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
