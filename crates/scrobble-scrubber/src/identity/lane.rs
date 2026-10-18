//! The paced, resumable background lane: the only caller of MusicBrainz and the LLM, and
//! the only writer of the resolution log. A peer of the executor; planning never waits
//! on it.

use super::adjudicate::{Adjudication, Adjudicator, Case, Precedent, PROMPT_VERSION};
use super::book::{
    KeyStatus, ResolutionEvent, ResolutionEventKind, ResolutionReader, ResolutionWriter, Snapshot,
};
use super::mb::{MbError, MbLookup, MusicBrainz};
use super::outcome::Target;
use super::provider::IDENTITY_PROVIDER;
use super::resolve::{Resolution, ResolveLimits, Resolver};
use super::title::parse_title;
use super::RESOLVER_VERSION;
use crate::events::{ScrubberEvent, ScrubberEventBus};
use crate::queue::{EditIntent, IntentState};
use crate::run::RunGeneration;
use crate::state::ScrubberState;
use crate::subject::ScrobbleKey;
use scrobble_store::Storage;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LaneReport {
    pub keys_attempted: u64,
    pub resolved: u64,
    pub ambiguous: u64,
    pub adjudicated: u64,
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
    events: ScrubberEventBus,
    options: LaneOptions,
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
            events: ScrubberEventBus::new(),
            options: LaneOptions::default(),
            _lock: None,
        }
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
    /// rate-limited keys end the round as `Deferred`.
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
        let worklist = self.worklist(&library, &snapshot, now, round.llm_calls_left > 0);

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

        let (requests_after, hits_after) = self.mb.counters();
        round.report.mb_requests = requests_after - requests_before;
        round.report.cache_hits = hits_after - hits_before;
        self.events.emit(ScrubberEvent::IdentityRoundCompleted {
            report: round.report.clone(),
        });
        Ok(round.report)
    }

    async fn record_rejections(&mut self, queue: &[EditIntent]) -> crate::Result<u64> {
        let consumed = &self.reader().snapshot().consumed_rejections;
        let at = unix_now();
        let events: Vec<ResolutionEvent> = queue
            .iter()
            .filter(|intent| {
                matches!(intent.state, IntentState::Rejected { .. })
                    && !consumed.contains(&intent.id)
            })
            .map(|intent| ResolutionEvent {
                key: intent.subject.key(),
                at,
                resolver: RESOLVER_VERSION,
                kind: ResolutionEventKind::HumanRejected {
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
                },
            })
            .collect();
        let recorded = events.len() as u64;
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
        now: u64,
        llm_budget_left: bool,
    ) -> Vec<WorkItem> {
        let not_found_retry = self.options.not_found_retry.as_secs();
        let mut items: Vec<(bool, WorkItem)> = library
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
                    trigger,
                    WorkItem {
                        key: key.clone(),
                        plays,
                        task,
                    },
                ))
            })
            .collect();
        items.sort_by(|(a_trigger, a), (b_trigger, b)| {
            b_trigger.cmp(a_trigger).then(b.plays.cmp(&a.plays))
        });
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

/// The most recent human decisions on identity proposals for this artist.
fn precedents(queue: &[EditIntent], artist: &str) -> Vec<Precedent> {
    let mut decided: Vec<&EditIntent> = queue
        .iter()
        .filter(|intent| {
            intent.provider == IDENTITY_PROVIDER
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
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::adjudicate::{Choice, Confidence, Verdict};
    use crate::identity::llm::scripted::ScriptedBackend;
    use crate::identity::llm::RawChoice;
    use crate::identity::testing::FixtureMb;
    use crate::queue::{QueueEvent, QueueEventKind};
    use crate::rewrite::create_no_op_edit;
    use crate::state::MemoryScrubberState;
    use crate::subject::Subject;
    use scrobble_store::{MemoryStorage, Provenanced, RecordSource, ScrobbleId, ScrobbleRecord};
    use uuid::Uuid;

    struct Harness {
        lane: IdentityLane,
        state: Arc<MemoryScrubberState>,
    }

    /// `(artist, track, album, plays)` per key.
    async fn harness(
        mb: FixtureMb,
        library: &[(&str, &str, Option<&str>, u32)],
        adjudicator: Option<Adjudicator>,
    ) -> Harness {
        let store = Arc::new(MemoryStorage::new());
        let mut records = Vec::new();
        for (artist, track, album, plays) in library {
            for _ in 0..*plays {
                let uts = 1_600_000_000 + records.len() as u64;
                records.push(ScrobbleRecord {
                    id: ScrobbleId::new(uts, artist, track),
                    uts,
                    artist: artist.to_string(),
                    track: track.to_string(),
                    album: album.map(str::to_string),
                    album_artist: Provenanced::Unknown,
                    source: RecordSource::Api,
                    fetched_at: 1,
                    deleted: false,
                    v: 1,
                });
            }
        }
        store.append_scrobbles(&records).await.unwrap();
        let state = Arc::new(MemoryScrubberState::new());
        let (writer, _) = crate::identity::book::open(state.clone()).await.unwrap();
        let lane = IdentityLane::new(store, state.clone(), writer, Arc::new(mb), adjudicator);
        Harness { lane, state }
    }

    /// `(track, album, event)` per logged event.
    async fn log(state: &MemoryScrubberState) -> Vec<(String, Option<String>, &'static str)> {
        state
            .load_resolution_events()
            .await
            .unwrap()
            .into_iter()
            .map(|e| (e.key.track, e.key.album, event_name(&e.kind)))
            .collect()
    }

    fn event_name(kind: &ResolutionEventKind) -> &'static str {
        match kind {
            ResolutionEventKind::Ambiguous { .. } => "ambiguous",
            other => status_label(other),
        }
    }

    fn row(
        track: &str,
        album: &str,
        event: &'static str,
    ) -> (String, Option<String>, &'static str) {
        (track.into(), Some(album.into()), event)
    }

    #[tokio::test]
    async fn round_resolves_each_key_once_trigger_first_and_then_rests() {
        let mb = FixtureMb::default().cocaine().wild_horses().four_on_six();
        let mut h = harness(
            mb,
            &[
                (
                    "Wes Montgomery",
                    "Four on Six",
                    Some("Smokin' at the Half Note"),
                    5,
                ),
                ("Eric Clapton", "Cocaine", Some("The Cream of Clapton"), 3),
                (
                    "Eric Clapton",
                    "Cocaine",
                    Some("Slowhand 35th Anniversary (Super Deluxe)"),
                    1,
                ),
                (
                    "The Rolling Stones",
                    "Wild Horses - 2009 Re-Mastered Digital Version",
                    Some("Sticky Fingers"),
                    1,
                ),
            ],
            None,
        )
        .await;

        let first = h.lane.run_round(10).await.unwrap();
        assert_eq!(
            first,
            LaneReport {
                keys_attempted: 4,
                resolved: 4,
                mb_requests: 11,
                ..LaneReport::default()
            }
        );
        let expected = vec![
            row("Cocaine", "The Cream of Clapton", "resolved"),
            row(
                "Cocaine",
                "Slowhand 35th Anniversary (Super Deluxe)",
                "resolved",
            ),
            row(
                "Wild Horses - 2009 Re-Mastered Digital Version",
                "Sticky Fingers",
                "resolved",
            ),
            row("Four on Six", "Smokin' at the Half Note", "resolved"),
        ];
        assert_eq!(log(&h.state).await, expected);

        let second = h.lane.run_round(10).await.unwrap();
        assert_eq!(second, LaneReport::default());
        assert_eq!(log(&h.state).await, expected);
    }

    #[tokio::test]
    async fn round_stops_at_its_key_budget() {
        let mb = FixtureMb::default().cocaine().four_on_six();
        let mut h = harness(
            mb,
            &[
                (
                    "Wes Montgomery",
                    "Four on Six",
                    Some("Smokin' at the Half Note"),
                    5,
                ),
                ("Eric Clapton", "Cocaine", Some("The Cream of Clapton"), 3),
            ],
            None,
        )
        .await;
        let report = h.lane.run_round(1).await.unwrap();
        assert_eq!(
            (report.keys_attempted, report.ended),
            (1, LaneEnded::BudgetExhausted)
        );
        let report = h.lane.run_round(1).await.unwrap();
        assert_eq!(
            (report.keys_attempted, report.ended),
            (1, LaneEnded::WorklistEmpty)
        );
    }

    #[tokio::test]
    async fn human_rejection_of_an_identity_intent_is_recorded_once() {
        let mut h = harness(FixtureMb::default(), &[], None).await;
        let subject = Subject {
            artist: "Eric Clapton".into(),
            track: "Cocaine".into(),
            album: Some("The Cream of Clapton".into()),
            album_artist: None,
        };
        let mut edit = create_no_op_edit(&subject.representative_track(0, None));
        edit.album_name = Some("Slowhand".into());
        edit.album_artist_name = Some("Eric Clapton".into());
        let id = Uuid::new_v4();
        h.state
            .append_queue_events(&[
                QueueEvent {
                    id,
                    at: 1,
                    kind: QueueEventKind::Created {
                        subject: subject.clone(),
                        proposed: Box::new(edit),
                        provider: IDENTITY_PROVIDER.into(),
                        requires_approval: true,
                        rationale: None,
                    },
                },
                QueueEvent {
                    id,
                    at: 2,
                    kind: QueueEventKind::Rejected {
                        dismiss_subject: false,
                    },
                },
            ])
            .await
            .unwrap();

        assert_eq!(h.lane.run_round(10).await.unwrap().rejections_recorded, 1);
        assert_eq!(h.lane.run_round(10).await.unwrap().rejections_recorded, 0);
        let events = h.state.load_resolution_events().await.unwrap();
        let kinds: Vec<_> = events.into_iter().map(|e| (e.key, e.kind)).collect();
        assert_eq!(
            kinds,
            vec![(
                subject.key(),
                ResolutionEventKind::HumanRejected {
                    intent: id,
                    target: Target {
                        title: "Cocaine".into(),
                        album: Some("Slowhand".into()),
                        album_artist: Some("Eric Clapton".into()),
                    },
                },
            )]
        );
    }

    #[tokio::test]
    async fn three_rate_limited_keys_in_a_row_defer_the_round() {
        let mb = FixtureMb {
            rate_limited: true,
            ..FixtureMb::default()
        };
        let mut h = harness(
            mb,
            &[
                ("A", "One", Some("X"), 5),
                ("A", "Two", Some("X"), 4),
                ("A", "Three", Some("X"), 3),
                ("A", "Four", Some("X"), 2),
            ],
            None,
        )
        .await;
        let report = h.lane.run_round(10).await.unwrap();
        assert_eq!(
            report,
            LaneReport {
                keys_attempted: 3,
                deferred: 3,
                mb_requests: 3,
                ended: LaneEnded::Deferred,
                ..LaneReport::default()
            }
        );
        assert_eq!(
            log(&h.state).await,
            vec![
                row("One", "X", "deferred"),
                row("Two", "X", "deferred"),
                row("Three", "X", "deferred"),
            ]
        );
        let entry = h.lane.reader().get(&ScrobbleKey {
            artist: "A".into(),
            track: "One".into(),
            album: Some("X".into()),
        });
        let Some(KeyStatus::Deferred { retry_after }) = entry.map(|e| e.status) else {
            panic!("expected a deferred key");
        };
        assert!(retry_after >= unix_now() + 29);
    }

    #[tokio::test]
    async fn ambiguous_cases_are_adjudicated_within_the_daily_budget() {
        let backend = Arc::new(ScriptedBackend::default());
        backend.answers.lock().unwrap().push(RawChoice {
            choice: "A".into(),
            confidence: "high".into(),
            reason: "the 2009 mix is the Sticky Fingers master".into(),
            text: r#"{"choice":"A"}"#.into(),
        });
        let adjudicator = Adjudicator::new(Box::new(backend.clone()), 1);
        let mut h = harness(
            FixtureMb::default().wild_horses_on_honk(),
            &[
                ("The Rolling Stones", "Wild Horses", Some("Honk"), 2),
                (
                    "The Rolling Stones",
                    "Wild Horses",
                    Some("Honk (Deluxe)"),
                    1,
                ),
            ],
            Some(adjudicator),
        )
        .await;

        let report = h.lane.run_round(10).await.unwrap();
        assert_eq!(
            (report.keys_attempted, report.ambiguous, report.adjudicated),
            (2, 2, 1)
        );
        assert_eq!(backend.requests.lock().unwrap().len(), 1);

        let events = h.state.load_resolution_events().await.unwrap();
        let summary: Vec<_> = events
            .iter()
            .map(|e| match &e.kind {
                ResolutionEventKind::Ambiguous { case, awaiting_llm } => (
                    e.key.album.clone(),
                    format!(
                        "ambiguous awaiting_llm={awaiting_llm} siblings={}",
                        case.siblings.len()
                    ),
                ),
                ResolutionEventKind::Adjudicated { verdict, by, .. } => (
                    e.key.album.clone(),
                    format!("adjudicated {:?} by {by}", verdict.choice),
                ),
                other => (e.key.album.clone(), status_label(other).to_string()),
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                (
                    Some("Honk".into()),
                    "ambiguous awaiting_llm=false siblings=1".into()
                ),
                (
                    Some("Honk".into()),
                    "adjudicated Option(OptionId('A')) by scripted".into()
                ),
                (
                    Some("Honk (Deluxe)".into()),
                    "ambiguous awaiting_llm=true siblings=1".into()
                ),
            ]
        );
        let entry = h.lane.reader().get(&events[0].key).unwrap();
        let KeyStatus::Open { verdict, .. } = entry.status else {
            panic!("expected an open case");
        };
        assert_eq!(
            verdict,
            Some(Verdict {
                choice: Choice::Option(crate::identity::OptionId('A')),
                confidence: Confidence::High,
                reason: "the 2009 mix is the Sticky Fingers master".into(),
                invalid: false,
            })
        );

        let report = h.lane.run_round(10).await.unwrap();
        assert_eq!(report.keys_attempted, 0);
        assert_eq!(backend.requests.lock().unwrap().len(), 1);
    }
}
