//! The paced, resumable background lane: the only caller of MusicBrainz and the LLM, and
//! the only writer of the resolution log. A peer of the executor; planning never waits
//! on it.

use super::adjudicate::{
    Adjudication, Adjudicator, Case, Choice, Precedent, Verdict, PROMPT_VERSION,
};
use super::book::{
    KeyStatus, ResolutionEvent, ResolutionEventKind, ResolutionReader, ResolutionWriter, Snapshot,
};
use super::mb::{MbError, MbLookup, MusicBrainz};
use super::naming::{
    self, naming_questions, NamingJudge, NamingKind, NamingQuestion, NamingVerdict,
    NAMING_PROMPT_VERSION,
};
use super::outcome::{MovePolicy, Proof, Target};
use super::provider::IDENTITY_PROVIDER;
use super::research::{Prior, ResearchCase, Researcher};
use super::resolve::{queries, Resolution, ResolveLimits, Resolver};
use super::title::{fold, parse_title};
use super::RESOLVER_VERSION;
use crate::events::{ScrubberEvent, ScrubberEventBus};
use crate::queue::{EditIntent, IntentState};
use crate::run::RunGeneration;
use crate::state::ScrubberState;
use crate::subject::ScrobbleKey;
use scrobble_store::Storage;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LaneReport {
    pub keys_attempted: u64,
    pub resolved: u64,
    pub ambiguous: u64,
    pub adjudicated: u64,
    /// LLM answers to album naming questions.
    pub naming_judged: u64,
    /// LLM research answers recorded, verified or not.
    pub researched: u64,
    pub not_found: u64,
    pub deferred: u64,
    pub rejections_recorded: u64,
    pub mb_requests: u64,
    pub cache_hits: u64,
    pub ended: LaneEnded,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LaneEnded {
    #[default]
    WorklistEmpty,
    BudgetExhausted,
    /// Sustained MB rate limiting or outage.
    Deferred,
    Cancelled,
}

#[derive(Clone, Copy, Debug)]
pub struct LaneOptions {
    /// Sleep between rounds when the worklist is empty or MB deferred us.
    pub idle_sleep: Duration,
    /// Keys per round in `run`.
    pub round_size: usize,
    /// Retry not-found keys after this long (new MB data appears).
    pub not_found_retry: Duration,
    pub limits: ResolveLimits,
}

impl Default for LaneOptions {
    fn default() -> Self {
        Self {
            idle_sleep: Duration::from_secs(600),
            round_size: 200,
            not_found_retry: Duration::from_secs(30 * 24 * 3600),
            limits: ResolveLimits::default(),
        }
    }
}

/// Consecutive rate-limited keys that end a round.
const RATE_LIMIT_STREAK: u32 = 3;
/// Retry delay after a non-rate-limit MusicBrainz failure.
const ERROR_BACKOFF: Duration = Duration::from_secs(3600);
/// An unparseable response won't fix itself soon; retry after MB data changes.
const PARSE_ERROR_BACKOFF: Duration = Duration::from_secs(7 * 24 * 3600);
const DAY_SECS: u64 = 24 * 3600;
/// A researched key isn't researched again for this long, whatever the answer.
const RESEARCH_RETRY_SECS: u64 = 90 * DAY_SECS;
const MAX_PRECEDENTS: usize = 5;

pub(crate) trait LaneMb: MbLookup {
    /// `(requests sent, cache hits)` since creation.
    fn counters(&self) -> (u64, u64);
}

impl LaneMb for MusicBrainz {
    fn counters(&self) -> (u64, u64) {
        let stats = self.stats();
        (stats.requests, stats.cache_hits)
    }
}

pub struct IdentityLane {
    store: Arc<dyn Storage>,
    state: Arc<dyn ScrubberState>,
    writer: ResolutionWriter,
    mb: Arc<dyn LaneMb>,
    adjudicator: Option<Adjudicator>,
    researcher: Option<Researcher>,
    events: ScrubberEventBus,
    options: LaneOptions,
    /// Which album moves derivation proposes, so only questions that bear on an edit are
    /// asked.
    moves: MovePolicy,
    /// Held for the lane's lifetime so only one process writes the log and paces MB.
    _lock: Option<std::fs::File>,
}

/// Live play counts per key, scanned from the store once per round. Ordered so every
/// album of one artist + title is contiguous.
struct Library {
    plays: BTreeMap<ScrobbleKey, u32>,
}

impl Library {
    async fn scan(store: &dyn Storage) -> crate::Result<Self> {
        let mut plays = BTreeMap::new();
        for record in store.scrobbles_in_range(0..u64::MAX).await? {
            if record.deleted {
                continue;
            }
            let key = ScrobbleKey {
                artist: record.artist,
                track: record.track,
                album: record.album,
            };
            *plays.entry(key).or_insert(0) += 1;
        }
        Ok(Self { plays })
    }

    /// The same artist + title under other albums.
    fn siblings(&self, key: &ScrobbleKey) -> Vec<ScrobbleKey> {
        let first = ScrobbleKey {
            album: None,
            ..key.clone()
        };
        self.plays
            .range(first..)
            .map(|(other, _)| other)
            .take_while(|other| other.artist == key.artist && other.track == key.track)
            .filter(|other| *other != key)
            .cloned()
            .collect()
    }
}

struct WorkItem {
    key: ScrobbleKey,
    plays: u32,
    task: Task,
}

enum Task {
    Resolve,
    /// An open case still waiting for an LLM answer.
    Adjudicate(Case),
}

struct Round {
    report: LaneReport,
    rate_limited_streak: u32,
    llm_calls_left: u32,
    /// The LLM failed this round; leave remaining cases waiting for the next.
    llm_down: bool,
}

impl Round {
    fn llm_available(&self) -> bool {
        !self.llm_down && self.llm_calls_left > 0
    }
}

impl IdentityLane {
    pub(crate) fn new(
        store: Arc<dyn Storage>,
        state: Arc<dyn ScrubberState>,
        writer: ResolutionWriter,
        mb: Arc<dyn LaneMb>,
        adjudicator: Option<Adjudicator>,
    ) -> Self {
        Self {
            store,
            state,
            writer,
            mb,
            adjudicator,
            researcher: None,
            events: ScrubberEventBus::new(),
            options: LaneOptions::default(),
            moves: MovePolicy::default(),
            _lock: None,
        }
    }

    pub(crate) fn with_researcher(mut self, researcher: Option<Researcher>) -> Self {
        self.researcher = researcher;
        self
    }

    pub(crate) fn with_moves(mut self, moves: MovePolicy) -> Self {
        self.moves = moves;
        self
    }

    pub(crate) fn holding(mut self, lock: std::fs::File) -> Self {
        self._lock = Some(lock);
        self
    }

    pub fn with_event_bus(mut self, events: ScrubberEventBus) -> Self {
        self.events = events;
        self
    }

    pub fn with_options(mut self, options: LaneOptions) -> Self {
        self.options = options;
        self
    }

    pub fn reader(&self) -> ResolutionReader {
        self.writer.reader()
    }

    /// Rounds back to back until `generation` is cancelled; sleeps `idle_sleep` when idle
    /// or deferred. Each key is atomic, so cancellation is checked between keys.
    pub async fn run(mut self, generation: RunGeneration) {
        while !generation.is_cancelled() {
            let idle = match self.round(self.options.round_size, Some(&generation)).await {
                Ok(report) => match report.ended {
                    LaneEnded::BudgetExhausted => false,
                    LaneEnded::Cancelled => break,
                    LaneEnded::WorklistEmpty | LaneEnded::Deferred => true,
                },
                Err(err) => {
                    log::warn!("identity lane: round failed: {err}");
                    self.events.emit(ScrubberEvent::Error {
                        error: format!("identity lane: {err}"),
                    });
                    true
                }
            };
            if idle {
                let idle_sleep = self.options.idle_sleep;
                let _ = generation
                    .run(async {
                        tokio::time::sleep(idle_sleep).await;
                        Ok(())
                    })
                    .await;
            }
        }
    }

    /// Records new human rejections of identity proposals, then processes at most
    /// `max_keys` keys, each durable once processed. Keys whose title appears under several
    /// albums or carries noise go first, then by plays. `RATE_LIMIT_STREAK` consecutive
    /// rate-limited keys end the round as `Deferred`. Unless deferred or cancelled, it then
    /// researches what stayed unsettled, within the daily research budget.
    pub async fn run_round(&mut self, max_keys: usize) -> crate::Result<LaneReport> {
        self.round(max_keys, None).await
    }

    async fn round(
        &mut self,
        max_keys: usize,
        stop: Option<&RunGeneration>,
    ) -> crate::Result<LaneReport> {
        let (requests_before, hits_before) = self.mb.counters();
        let queue = self.state.load_queue().await?;
        let rejections_recorded = self.record_rejections(&queue).await?;
        let library = Library::scan(&*self.store).await?;
        let now = unix_now();
        let snapshot = self.reader().snapshot();
        let mut round = Round {
            report: LaneReport {
                rejections_recorded,
                ..LaneReport::default()
            },
            rate_limited_streak: 0,
            llm_calls_left: self.llm_calls_left(&snapshot, now),
            llm_down: false,
        };
        let queued: BTreeSet<ScrobbleKey> = queue
            .iter()
            .filter(|intent| intent.state.is_open())
            .map(|intent| intent.subject.key())
            .collect();
        let worklist = self.worklist(&library, &snapshot, &queued, now, round.llm_calls_left > 0);

        for item in worklist {
            if stop.is_some_and(RunGeneration::is_cancelled) {
                round.report.ended = LaneEnded::Cancelled;
                break;
            }
            if matches!(item.task, Task::Adjudicate(_))
                && self.adjudicator.is_some()
                && !round.llm_available()
            {
                continue;
            }
            if round.report.keys_attempted as usize >= max_keys {
                round.report.ended = LaneEnded::BudgetExhausted;
                break;
            }
            round.report.keys_attempted += 1;

            let kinds = match item.task {
                Task::Resolve => self.resolve_key(&item, &library, &queue, &mut round).await,
                Task::Adjudicate(case) => self.open_case(case, &queue, &mut round).await,
            };
            let status = kinds.last().map(status_label).unwrap_or_default();
            let at = unix_now();
            let events = kinds
                .into_iter()
                .map(|kind| ResolutionEvent {
                    key: item.key.clone(),
                    at,
                    resolver: RESOLVER_VERSION,
                    kind,
                })
                .collect();
            self.writer.append(events).await?;
            self.events.emit(ScrubberEvent::IdentityKeyProcessed {
                key: item.key,
                status: status.to_string(),
            });

            if round.rate_limited_streak >= RATE_LIMIT_STREAK {
                round.report.ended = LaneEnded::Deferred;
                break;
            }
        }

        if round.report.ended != LaneEnded::Cancelled {
            self.judge_namings(&library, &mut round, stop).await?;
        }
        if !matches!(
            round.report.ended,
            LaneEnded::Deferred | LaneEnded::Cancelled
        ) {
            self.research(&library, &mut round.report, stop).await?;
        }

        let (requests_after, hits_after) = self.mb.counters();
        round.report.mb_requests = requests_after - requests_before;
        round.report.cache_hits = hits_after - hits_before;
        self.events.emit(ScrubberEvent::IdentityRoundCompleted {
            report: round.report.clone(),
        });
        Ok(round.report)
    }

    /// Asks the LLM the naming questions nobody has answered, most-played album first,
    /// within the round's share of the daily LLM budget. A failure leaves the rest for a
    /// later round.
    async fn judge_namings(
        &mut self,
        library: &Library,
        round: &mut Round,
        stop: Option<&RunGeneration>,
    ) -> crate::Result<()> {
        let Some(adjudicator) = &self.adjudicator else {
            return Ok(());
        };
        let snapshot = self.writer.reader().snapshot();
        let plays = library.plays.iter().map(|(key, plays)| (key, *plays));
        for (key, case) in naming::naming_cases(plays, &snapshot, &self.moves) {
            if stop.is_some_and(RunGeneration::is_cancelled) {
                round.report.ended = LaneEnded::Cancelled;
                break;
            }
            if !round.llm_available() {
                break;
            }
            let answer = match naming::judge(adjudicator.backend(), &case).await {
                Ok(answer) => answer,
                Err(err) => {
                    log::warn!(
                        "identity lane: naming question for {} failed: {err}",
                        case.question.users_title
                    );
                    round.llm_down = true;
                    break;
                }
            };
            round.llm_calls_left -= 1;
            round.report.naming_judged += 1;
            let kind = ResolutionEventKind::NamingJudged {
                question: case.question,
                verdict: answer.verdict,
                reason: answer.reason,
                by: NamingJudge::Llm {
                    model: answer.by,
                    prompt_version: NAMING_PROMPT_VERSION,
                },
                raw: answer.raw,
            };
            let status = status_label(&kind);
            self.writer
                .append(vec![ResolutionEvent {
                    key: key.clone(),
                    at: unix_now(),
                    resolver: RESOLVER_VERSION,
                    kind,
                }])
                .await?;
            self.events.emit(ScrubberEvent::IdentityKeyProcessed {
                key,
                status: status.to_string(),
            });
        }
        Ok(())
    }

    /// An LLM failure stops research for the round and leaves the key for a later one. A
    /// MusicBrainz error while verifying an answer also stops it, but the answer is still
    /// recorded unverified, so the call counts against the budget and the key waits out
    /// [`RESEARCH_RETRY_SECS`].
    async fn research(
        &mut self,
        library: &Library,
        report: &mut LaneReport,
        stop: Option<&RunGeneration>,
    ) -> crate::Result<()> {
        let Some(researcher) = &self.researcher else {
            return Ok(());
        };
        let now = unix_now();
        let snapshot = self.writer.reader().snapshot();
        let since = now.saturating_sub(DAY_SECS);
        let used = snapshot
            .research_times
            .iter()
            .filter(|at| **at > since)
            .count();
        let left = (researcher.daily_budget() as usize).saturating_sub(used);
        let resolver = Resolver {
            mb: self.mb.as_ref(),
            limits: self.options.limits,
        };
        for case in research_cases(library, &snapshot, now)
            .into_iter()
            .take(left)
        {
            if stop.is_some_and(RunGeneration::is_cancelled) {
                report.ended = LaneEnded::Cancelled;
                break;
            }
            let research = match researcher.research(&case).await {
                Ok(research) => research,
                Err(err) => {
                    log::warn!("identity lane: research of {} failed: {err}", case.key);
                    break;
                }
            };
            let (outcome, mb_failed) = match &research.recording {
                Some(recording) => match resolver
                    .verify_research(
                        &case.key,
                        recording,
                        research.release_group.as_ref(),
                        &research.by,
                    )
                    .await
                {
                    Ok(outcome) => (outcome, false),
                    Err(err) => {
                        log::warn!(
                            "identity lane: verifying research for {} failed: {err}",
                            case.key
                        );
                        (None, true)
                    }
                },
                None => (None, false),
            };
            report.researched += 1;
            let kind = ResolutionEventKind::Researched {
                outcome,
                recording: research.recording,
                release_group: research.release_group,
                confidence: research.confidence,
                reason: research.reason,
                by: research.by,
                raw: research.raw,
            };
            let status = status_label(&kind);
            self.writer
                .append(vec![ResolutionEvent {
                    key: case.key.clone(),
                    at: unix_now(),
                    resolver: RESOLVER_VERSION,
                    kind,
                }])
                .await?;
            self.events.emit(ScrubberEvent::IdentityKeyProcessed {
                key: case.key,
                status: status.to_string(),
            });
            if mb_failed {
                break;
            }
        }
        Ok(())
    }

    /// Also answers "keep" for the album of a rejected intent whose album was a retitle,
    /// so sibling tracks stop proposing it.
    async fn record_rejections(&mut self, queue: &[EditIntent]) -> crate::Result<u64> {
        let snapshot = self.reader().snapshot();
        let at = unix_now();
        let mut recorded = 0;
        let mut kept = BTreeSet::new();
        let mut events = Vec::new();
        let rejected = queue.iter().filter(|intent| {
            matches!(intent.state, IntentState::Rejected { .. })
                && !snapshot.consumed_rejections.contains(&intent.id)
        });
        for intent in rejected {
            recorded += 1;
            let event = |kind| ResolutionEvent {
                key: intent.subject.key(),
                at,
                resolver: RESOLVER_VERSION,
                kind,
            };
            events.push(event(ResolutionEventKind::HumanRejected {
                intent: intent.id,
                target: Target {
                    title: intent
                        .proposed
                        .track_name
                        .clone()
                        .unwrap_or_else(|| intent.subject.track.clone()),
                    album: intent.proposed.album_name.clone(),
                    album_artist: intent.proposed.album_artist_name.clone(),
                },
            }));
            for question in rejected_retitles(&snapshot, intent) {
                let already_kept = snapshot
                    .naming
                    .get(&question)
                    .is_some_and(|judgement| judgement.verdict == NamingVerdict::Keep);
                if kept.insert(question.clone()) && !already_kept {
                    events.push(event(ResolutionEventKind::NamingJudged {
                        question,
                        verdict: NamingVerdict::Keep,
                        reason: "A person rejected the retitle in review.".to_string(),
                        by: NamingJudge::Human { intent: intent.id },
                        raw: String::new(),
                    }));
                }
            }
        }
        self.writer.append(events).await?;
        Ok(recorded)
    }

    fn llm_calls_left(&self, snapshot: &Snapshot, now: u64) -> u32 {
        let Some(adjudicator) = &self.adjudicator else {
            return 0;
        };
        let since = now.saturating_sub(DAY_SECS);
        let used = snapshot
            .adjudication_times
            .iter()
            .filter(|at| **at > since)
            .count();
        adjudicator
            .daily_budget()
            .saturating_sub(u32::try_from(used).unwrap_or(u32::MAX))
    }

    fn worklist(
        &self,
        library: &Library,
        snapshot: &Snapshot,
        queued: &BTreeSet<ScrobbleKey>,
        now: u64,
        llm_budget_left: bool,
    ) -> Vec<WorkItem> {
        let not_found_retry = self.options.not_found_retry.as_secs();
        let mut items: Vec<((bool, bool), WorkItem)> = library
            .plays
            .iter()
            .filter_map(|(key, &plays)| {
                let task = match snapshot.keys.get(key) {
                    None => Task::Resolve,
                    Some(entry) => match &entry.status {
                        KeyStatus::Stale => Task::Resolve,
                        KeyStatus::Deferred { retry_after } if *retry_after <= now => Task::Resolve,
                        KeyStatus::NotFound if entry.at + not_found_retry <= now => Task::Resolve,
                        KeyStatus::Open {
                            case,
                            awaiting_llm: true,
                            ..
                        } if self.adjudicator.is_none() || llm_budget_left => {
                            Task::Adjudicate(case.clone())
                        }
                        _ => return None,
                    },
                };
                let parsed = parse_title(&key.track);
                let trigger = !parsed.packaging.is_empty()
                    || !parsed.identity.is_empty()
                    || !library.siblings(key).is_empty();
                Some((
                    (queued.contains(key), trigger),
                    WorkItem {
                        key: key.clone(),
                        plays,
                        task,
                    },
                ))
            })
            .collect();
        // Keys with an open intent first: their answer settles edits a human would otherwise
        // review.
        items.sort_by(|(a_rank, a), (b_rank, b)| b_rank.cmp(a_rank).then(b.plays.cmp(&a.plays)));
        items.into_iter().map(|(_, item)| item).collect()
    }

    async fn resolve_key(
        &self,
        item: &WorkItem,
        library: &Library,
        queue: &[EditIntent],
        round: &mut Round,
    ) -> Vec<ResolutionEventKind> {
        let resolver = Resolver {
            mb: self.mb.as_ref(),
            limits: self.options.limits,
        };
        let result = resolver.resolve(&item.key).await;
        if !matches!(result, Err(MbError::RateLimited(_))) {
            round.rate_limited_streak = 0;
        }
        match result {
            Ok(Resolution::Resolved(outcome)) => {
                round.report.resolved += 1;
                vec![ResolutionEventKind::Resolved { outcome }]
            }
            Ok(Resolution::Ambiguous(options)) => {
                round.report.ambiguous += 1;
                let case = Case::new(
                    item.key.clone(),
                    item.plays,
                    library.siblings(&item.key),
                    options,
                );
                self.open_case(case, queue, round).await
            }
            Ok(Resolution::NotFound { queries }) => {
                round.report.not_found += 1;
                vec![ResolutionEventKind::NotFound { queries }]
            }
            Err(err) => {
                round.report.deferred += 1;
                let wait = match &err {
                    MbError::RateLimited(retry_after) => {
                        round.rate_limited_streak += 1;
                        retry_after.as_secs().max(1)
                    }
                    MbError::Parse(_) => PARSE_ERROR_BACKOFF.as_secs(),
                    _ => ERROR_BACKOFF.as_secs(),
                };
                log::warn!("identity lane: deferring {}: {err}", item.key);
                vec![ResolutionEventKind::Deferred {
                    error: err.to_string(),
                    retry_after: unix_now() + wait,
                }]
            }
        }
    }

    /// Ask the LLM now when it's configured and has budget; otherwise the case waits for
    /// it, or (with no LLM at all) goes straight to the human walk.
    async fn open_case(
        &self,
        case: Case,
        queue: &[EditIntent],
        round: &mut Round,
    ) -> Vec<ResolutionEventKind> {
        let Some(adjudicator) = &self.adjudicator else {
            return vec![ResolutionEventKind::Ambiguous {
                case,
                awaiting_llm: false,
            }];
        };
        if !round.llm_available() {
            return vec![ResolutionEventKind::Ambiguous {
                case,
                awaiting_llm: true,
            }];
        }
        let precedents = precedents(queue, &case.key.artist);
        match adjudicator.adjudicate(&case, &precedents).await {
            Ok(Adjudication {
                case_hash,
                verdict,
                by,
                raw,
            }) => {
                round.llm_calls_left -= 1;
                round.report.adjudicated += 1;
                vec![
                    ResolutionEventKind::Ambiguous {
                        case,
                        awaiting_llm: false,
                    },
                    ResolutionEventKind::Adjudicated {
                        case_hash,
                        verdict,
                        by,
                        prompt_version: PROMPT_VERSION,
                        raw,
                    },
                ]
            }
            Err(err) => {
                log::warn!("identity lane: adjudication of {} failed: {err}", case.key);
                round.llm_down = true;
                vec![ResolutionEventKind::Ambiguous {
                    case,
                    awaiting_llm: true,
                }]
            }
        }
    }
}

/// Live keys research may help with, most-played first.
fn research_cases(library: &Library, snapshot: &Snapshot, now: u64) -> Vec<ResearchCase> {
    let mut cases: Vec<ResearchCase> = library
        .plays
        .iter()
        .filter_map(|(key, &plays)| {
            let entry = snapshot.keys.get(key)?;
            if entry
                .research
                .as_ref()
                .is_some_and(|note| note.at + RESEARCH_RETRY_SECS > now)
            {
                return None;
            }
            let prior = match &entry.status {
                KeyStatus::NotFound => Prior::NotFound {
                    queries: queries(key),
                },
                KeyStatus::Open {
                    case,
                    verdict:
                        Some(Verdict {
                            choice: Choice::Unsure,
                            reason,
                            ..
                        }),
                    ..
                } => Prior::Unsure {
                    options: case.options.clone(),
                    reason: reason.clone(),
                },
                KeyStatus::Settled { outcome }
                    if outcome.proof == Proof::TitleOnly && key.album.is_some() =>
                {
                    Prior::Unanchored {
                        outcome: Box::new(outcome.clone()),
                    }
                }
                _ => return None,
            };
            Some(ResearchCase {
                key: key.clone(),
                plays,
                siblings: library.siblings(key),
                prior,
            })
        })
        .collect();
    cases.sort_by_key(|case| std::cmp::Reverse(case.plays));
    cases
}

/// The naming questions whose retitles `intent` proposed for its album and track.
fn rejected_retitles(snapshot: &Snapshot, intent: &EditIntent) -> Vec<NamingQuestion> {
    let key = intent.subject.key();
    let Some(entry) = snapshot.keys.get(&key) else {
        return Vec::new();
    };
    let proposed = |kind| {
        let proposed = match kind {
            NamingKind::Album => intent.proposed.album_name.as_deref(),
            NamingKind::Track => intent.proposed.track_name.as_deref(),
        };
        proposed.map(fold)
    };
    let mut questions: Vec<NamingQuestion> = entry
        .plannable()
        .into_iter()
        .flat_map(|(outcome, chosen)| naming_questions(&key, outcome, chosen))
        .filter(|question| proposed(question.kind) == Some(fold(&question.title)))
        .collect();
    questions.sort();
    questions.dedup();
    questions
}

/// The most recent human decisions on identity proposals for this artist.
fn precedents(queue: &[EditIntent], artist: &str) -> Vec<Precedent> {
    let mut decided: Vec<&EditIntent> = queue
        .iter()
        .filter(|intent| {
            intent
                .sources
                .values()
                .any(|provider| provider == IDENTITY_PROVIDER)
                && intent.subject.artist == artist
                && matches!(
                    intent.state,
                    IntentState::Applied | IntentState::Rejected { .. }
                )
        })
        .collect();
    decided.sort_by_key(|intent| std::cmp::Reverse(intent.updated_at));
    decided
        .into_iter()
        .take(MAX_PRECEDENTS)
        .map(|intent| Precedent {
            key: intent.subject.key(),
            proposed_album: intent.proposed.album_name.clone(),
            proposed_title: intent.proposed.track_name.clone(),
            accepted: intent.state == IntentState::Applied,
        })
        .collect()
}

fn status_label(kind: &ResolutionEventKind) -> &'static str {
    match kind {
        ResolutionEventKind::Resolved { .. } => "resolved",
        ResolutionEventKind::Ambiguous {
            awaiting_llm: true, ..
        } => "awaiting_llm",
        ResolutionEventKind::Ambiguous { .. } => "open",
        ResolutionEventKind::Adjudicated { .. } => "adjudicated",
        ResolutionEventKind::NotFound { .. } => "not_found",
        ResolutionEventKind::Deferred { .. } => "deferred",
        ResolutionEventKind::HumanRejected { .. } => "human_rejected",
        ResolutionEventKind::Researched {
            outcome: Some(_), ..
        } => "researched",
        ResolutionEventKind::Researched { outcome: None, .. } => "research_unverified",
        ResolutionEventKind::NamingJudged { .. } => "naming_judged",
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
#[path = "lane_tests.rs"]
mod tests;
