//! MusicBrainz-backed scrobble identity: resolve each scrobble key to a MusicBrainz
//! master and its canonical album, let an LLM choose among options when the evidence
//! can't, and turn conclusions into ordinary edit intents.
//!
//! Three parts with one owner each:
//! - [`IdentityLane`]: a paced background task. The only caller of MusicBrainz and the
//!   LLM, and the only writer of `resolutions.jsonl`. As a last resort it lets an LLM
//!   with web search name a MusicBrainz recording, which is verified before use.
//! - [`ResolutionReader`]: the folded log, shared read-only.
//! - [`IdentityProvider`]: a pure planning provider that derives edits from the log and
//!   exposes a change feed so keys resolved after their time range was planned still get
//!   planned.

mod adjudicate;
mod book;
mod canonical;
mod lane;
mod llm;
mod mb;
mod outcome;
mod provider;
mod research;
mod resolve;
#[cfg(test)]
mod testing;
mod title;

pub use adjudicate::{Case, CaseOption, Choice, Confidence, OptionEvidence, OptionId, Verdict};
pub use book::{
    KeyEntry, KeyStatus, ResearchNote, ResolutionEvent, ResolutionEventKind, ResolutionReader,
    StatusCounts,
};
pub use canonical::{AlbumMove, Canonical, Rung};
pub use lane::{IdentityLane, LaneEnded, LaneOptions, LaneReport};
pub use mb::{ArtistCredit, ReleaseGroup, UserAgent};
pub use outcome::{MoveMode, MovePolicy, Outcome, Proof, Target};
pub use provider::{IdentityProvider, IDENTITY_PROVIDER};
pub use resolve::ResolveLimits;

use crate::state::ScrubberState;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// Bump whenever resolve/canonical/title/outcome policy changes meaning. Older records
/// fold to `Stale`: planning ignores them and the lane re-resolves them (mostly from
/// cache).
pub const RESOLVER_VERSION: u32 = 2;

pub struct IdentityConfig {
    pub state: Arc<dyn ScrubberState>,
    /// Holds the rebuildable `musicbrainz.sqlite` cache. Keep it out of synced state.
    pub cache_dir: PathBuf,
    pub http: Arc<dyn http_client::HttpClient>,
    pub user_agent: UserAgent,
    pub llm: Option<LlmConfig>,
    pub moves: MovePolicy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LlmProvider {
    /// A local CLI (by default `claude -p`), which uses whatever login that CLI has, such
    /// as a Claude subscription. See [`LlmConfig::command`].
    Command,
    /// Native Messages API with structured outputs.
    Anthropic,
    /// Any `/chat/completions` server honoring `response_format: json_schema`.
    OpenAiCompat,
}

/// `claude -p` with no tools, settings, MCP servers or session history, answering in the
/// choice schema. Runs on the CLI's own login (a Claude subscription works).
pub const CLAUDE_CLI_COMMAND: &[&str] = &[
    "claude",
    "-p",
    "--output-format",
    "json",
    "--json-schema",
    "{schema}",
    "--model",
    "{model}",
    "--effort",
    "low",
    "--system-prompt",
    "{system}",
    "--tools",
    "",
    "--setting-sources",
    "",
    "--strict-mcp-config",
    "--no-session-persistence",
];

/// `claude -p` with only web search and fetch, answering in the research schema. Runs on
/// the CLI's own login.
pub const CLAUDE_CLI_RESEARCH_COMMAND: &[&str] = &[
    "claude",
    "-p",
    "--output-format",
    "json",
    "--json-schema",
    "{schema}",
    "--model",
    "{model}",
    "--effort",
    "medium",
    "--system-prompt",
    "{system}",
    "--tools",
    "WebSearch",
    "WebFetch",
    "--allowedTools",
    "WebSearch",
    "WebFetch",
    "--setting-sources",
    "",
    "--strict-mcp-config",
    "--no-session-persistence",
];

fn argv(template: &[&str]) -> Vec<String> {
    template.iter().map(|arg| arg.to_string()).collect()
}

#[derive(Clone, Debug)]
pub struct LlmConfig {
    pub provider: LlmProvider,
    /// For [`LlmProvider::Command`]: argv with `{schema}`, `{system}` and `{model}`
    /// placeholders. The case is written to stdin; stdout is either the answer JSON or a
    /// `claude -p --output-format json` envelope.
    pub command: Vec<String>,
    /// e.g. `https://api.anthropic.com/v1` or `https://api.openai.com/v1`.
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
    pub daily_call_budget: u32,
    pub min_interval: Duration,
    /// For [`LlmProvider::Command`]: argv for web research, with the same placeholders as
    /// `command`. Empty disables research.
    pub research_command: Vec<String>,
    pub research_daily_budget: u32,
}

impl LlmConfig {
    /// `SCRUBBER_LLM_PROVIDER` picks the backend; `None` when it's unset.
    /// - `claude-cli`: [`CLAUDE_CLI_COMMAND`] on the CLI's own login (no API key), model
    ///   from `SCRUBBER_LLM_MODEL` (default `opus`).
    /// - `command`: the JSON argv in `SCRUBBER_LLM_COMMAND` (any harness CLI).
    /// - `anthropic`: the Messages API; key from `SCRUBBER_LLM_API_KEY` or
    ///   `ANTHROPIC_API_KEY`, model default `claude-opus-5-5`.
    /// - `openai`: chat completions; needs `SCRUBBER_LLM_MODEL`, key from
    ///   `SCRUBBER_LLM_API_KEY` or `OPENAI_API_KEY` unless `SCRUBBER_LLM_BASE_URL` is set.
    ///
    /// `SCRUBBER_LLM_DAILY_BUDGET` caps calls per day (default 200). Research runs only
    /// with a command backend: `claude-cli` defaults to [`CLAUDE_CLI_RESEARCH_COMMAND`],
    /// `command` takes a JSON argv from `SCRUBBER_LLM_RESEARCH_COMMAND`, and
    /// `SCRUBBER_LLM_RESEARCH_BUDGET` caps research calls per day (default 20).
    pub fn from_env() -> Option<Self> {
        let var = |name: &str| {
            std::env::var(name)
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        };
        let provider_name = var("SCRUBBER_LLM_PROVIDER")?.to_ascii_lowercase();
        let provider = match provider_name.as_str() {
            "claude-cli" | "claude-code" | "command" => LlmProvider::Command,
            "anthropic" => LlmProvider::Anthropic,
            "openai" | "openai-compat" => LlmProvider::OpenAiCompat,
            other => {
                log::warn!(
                    "ignoring SCRUBBER_LLM_PROVIDER={other:?}: expected claude-cli, command, anthropic or openai"
                );
                return None;
            }
        };
        let budget = |name: &str, default: u32| match var(name) {
            Some(raw) => raw.parse().unwrap_or_else(|_| {
                log::warn!("ignoring {name}={raw:?}: not a whole number");
                default
            }),
            None => default,
        };
        let daily_call_budget = budget("SCRUBBER_LLM_DAILY_BUDGET", 200);
        let research_daily_budget = budget("SCRUBBER_LLM_RESEARCH_BUDGET", 20);
        let json_argv = |name: &str| match var(name) {
            Some(raw) => match serde_json::from_str::<Vec<String>>(&raw) {
                Ok(argv) if !argv.is_empty() => Ok(Some(argv)),
                _ => Err(format!("{name} must be a JSON array of strings")),
            },
            None => Ok(None),
        };
        if provider == LlmProvider::Command {
            let command = match json_argv("SCRUBBER_LLM_COMMAND") {
                Ok(Some(argv)) => argv,
                Err(err) => {
                    log::warn!("{err}; LLM disabled");
                    return None;
                }
                Ok(None) if provider_name == "command" => {
                    log::warn!(
                        "SCRUBBER_LLM_PROVIDER=command needs SCRUBBER_LLM_COMMAND; LLM disabled"
                    );
                    return None;
                }
                Ok(None) => argv(CLAUDE_CLI_COMMAND),
            };
            let research_command = match json_argv("SCRUBBER_LLM_RESEARCH_COMMAND") {
                Ok(Some(argv)) => argv,
                Err(err) => {
                    log::warn!("{err}; research disabled");
                    Vec::new()
                }
                Ok(None) if provider_name == "command" => Vec::new(),
                Ok(None) => argv(CLAUDE_CLI_RESEARCH_COMMAND),
            };
            return Some(Self {
                command,
                model: var("SCRUBBER_LLM_MODEL").unwrap_or_else(|| "opus".to_string()),
                daily_call_budget,
                research_command,
                research_daily_budget,
                ..Self::claude_cli()
            });
        }
        let (default_base, default_model, fallback_key) = match provider {
            LlmProvider::Command => unreachable!("handled above"),
            LlmProvider::Anthropic => (
                "https://api.anthropic.com/v1",
                Some("claude-opus-5-5"),
                var("ANTHROPIC_API_KEY"),
            ),
            LlmProvider::OpenAiCompat => ("https://api.openai.com/v1", None, var("OPENAI_API_KEY")),
        };
        let model = var("SCRUBBER_LLM_MODEL").or_else(|| default_model.map(str::to_string));
        let Some(model) = model else {
            log::warn!("SCRUBBER_LLM_PROVIDER=openai needs SCRUBBER_LLM_MODEL; LLM disabled");
            return None;
        };
        let custom_base = var("SCRUBBER_LLM_BASE_URL");
        let api_key = var("SCRUBBER_LLM_API_KEY").or(fallback_key);
        if api_key.is_none() && custom_base.is_none() {
            log::warn!("SCRUBBER_LLM_PROVIDER is set but no API key was found; LLM disabled");
            return None;
        }
        Some(Self {
            provider,
            command: Vec::new(),
            base_url: custom_base.unwrap_or_else(|| default_base.to_string()),
            api_key,
            model,
            daily_call_budget,
            min_interval: Duration::from_secs(1),
            research_command: Vec::new(),
            research_daily_budget,
        })
    }

    /// `claude -p` on Opus, on the CLI's own login: choices at low effort, research at
    /// medium effort with web search.
    pub fn claude_cli() -> Self {
        Self {
            provider: LlmProvider::Command,
            command: argv(CLAUDE_CLI_COMMAND),
            base_url: String::new(),
            api_key: None,
            model: "opus".to_string(),
            daily_call_budget: 200,
            min_interval: Duration::from_secs(1),
            research_command: argv(CLAUDE_CLI_RESEARCH_COMMAND),
            research_daily_budget: 20,
        }
    }
}

/// Holds the single resolution writer until [`Identity::lane`] takes it.
pub struct Identity {
    writer: Option<book::ResolutionWriter>,
    reader: ResolutionReader,
    config: IdentityConfig,
}

impl Identity {
    /// Folds `resolutions.jsonl`. No network.
    pub async fn open(config: IdentityConfig) -> crate::Result<Self> {
        let (writer, reader) = book::open(config.state.clone()).await?;
        Ok(Self {
            writer: Some(writer),
            reader,
            config,
        })
    }

    pub fn reader(&self) -> ResolutionReader {
        self.reader.clone()
    }

    /// Pure: never touches the network.
    pub fn provider(&self) -> IdentityProvider {
        IdentityProvider::new(self.reader.clone(), self.config.moves)
    }

    /// The background lane. There is exactly one writer of the resolution log: this errors
    /// if the lane was already taken, or if another process on this machine holds it.
    /// Once locked, the log is re-folded, so readers see what other processes appended
    /// since [`Identity::open`].
    pub async fn lane(
        &mut self,
        store: Arc<dyn scrobble_store::Storage>,
    ) -> crate::Result<IdentityLane> {
        let Some(writer) = self.writer.as_mut() else {
            return Err(crate::ScrubberError::InvalidState(
                "the identity lane was already taken".into(),
            ));
        };
        std::fs::create_dir_all(&self.config.cache_dir)?;
        let lock_path = self.config.cache_dir.join("lane.lock");
        let lock = std::fs::File::create(&lock_path)?;
        if lock.try_lock().is_err() {
            return Err(crate::ScrubberError::InvalidState(format!(
                "another process is already resolving identities ({} is locked)",
                lock_path.display()
            )));
        }
        writer.reload().await?;
        let mb = mb::MusicBrainz::new(
            self.config.http.clone(),
            self.config.user_agent.clone(),
            &self.config.cache_dir.join("musicbrainz.sqlite"),
        )
        .map_err(|err| crate::ScrubberError::Provider {
            provider: IDENTITY_PROVIDER.into(),
            message: format!("opening the MusicBrainz cache: {err}"),
        })?;
        let adjudicator = self.config.llm.clone().map(|llm| {
            let budget = llm.daily_call_budget;
            adjudicate::Adjudicator::new(llm::backend(self.config.http.clone(), llm), budget)
        });
        let researcher = self
            .config
            .llm
            .clone()
            .filter(|llm| {
                llm.provider == LlmProvider::Command
                    && !llm.research_command.is_empty()
                    && llm.research_daily_budget > 0
            })
            .map(|llm| {
                let budget = llm.research_daily_budget;
                research::Researcher::new(llm::backend(self.config.http.clone(), llm), budget)
            });
        let writer = self.writer.take().expect("checked above");
        Ok(IdentityLane::new(
            store,
            self.config.state.clone(),
            writer,
            Arc::new(mb),
            adjudicator,
        )
        .with_researcher(researcher)
        .holding(lock))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::MemoryScrubberState;
    use crate::subject::ScrobbleKey;
    use scrobble_store::{
        MemoryStorage, Provenanced, RecordSource, ScrobbleId, ScrobbleRecord, Storage,
    };

    #[derive(Debug)]
    struct Offline;

    #[async_trait::async_trait]
    impl http_client::HttpClient for Offline {
        async fn send(
            &self,
            _: http_client::Request,
        ) -> Result<http_client::Response, http_client::Error> {
            Err(http_client::Error::from_str(
                http_client::http_types::StatusCode::ServiceUnavailable,
                "offline",
            ))
        }
    }

    fn key(track: &str) -> ScrobbleKey {
        ScrobbleKey {
            artist: "Eric Clapton".into(),
            track: track.into(),
            album: Some("Slowhand".into()),
        }
    }

    fn not_found(track: &str) -> ResolutionEvent {
        ResolutionEvent {
            key: key(track),
            at: 1,
            resolver: RESOLVER_VERSION,
            kind: ResolutionEventKind::NotFound {
                queries: Vec::new(),
            },
        }
    }

    #[tokio::test]
    async fn the_lane_starts_from_the_log_as_it_is_when_locked() {
        let state = Arc::new(MemoryScrubberState::new());
        state
            .append_resolution_events(&[not_found("Cocaine")])
            .await
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let mut identity = Identity::open(IdentityConfig {
            state: state.clone(),
            cache_dir: dir.path().to_path_buf(),
            http: Arc::new(Offline),
            user_agent: UserAgent::new("scrobble-scrubber-test/0.2", "ivanmalison@gmail.com")
                .unwrap(),
            llm: None,
            moves: MovePolicy::default(),
        })
        .await
        .unwrap();
        let reader = identity.reader();
        state
            .append_resolution_events(&[not_found("Wonderful Tonight")])
            .await
            .unwrap();
        let store = Arc::new(MemoryStorage::new());
        store
            .append_scrobbles(&[ScrobbleRecord {
                id: ScrobbleId::new(1_600_000_000, "Eric Clapton", "Lay Down Sally"),
                uts: 1_600_000_000,
                artist: "Eric Clapton".into(),
                track: "Lay Down Sally".into(),
                album: Some("Slowhand".into()),
                album_artist: Provenanced::Unknown,
                source: RecordSource::Api,
                fetched_at: 1,
                deleted: false,
                v: 1,
            }])
            .await
            .unwrap();

        let mut lane = identity.lane(store).await.unwrap();

        assert_eq!(reader.changes_since(1), (vec![key("Wonderful Tonight")], 2));
        let report = lane.run_round(1).await.unwrap();
        assert_eq!(report.deferred, 1);
        assert_eq!(
            reader.changes_since(2),
            (vec![key("Lay Down Sally")], 3),
            "the lane's append continues from the head on disk"
        );
        let (_, reopened) = book::open(state).await.unwrap();
        assert_eq!(reopened.get(&key("Lay Down Sally")).map(|e| e.seq), Some(3));
    }
}
