//! The planner: conceives edits without ever touching last.fm.
//!
//! Resolves a [`ScrubFeed`] against the store, dedupes records into
//! [`Subject`]s, runs the ranked correctors over representative tracks, merges their
//! edits per subject (see [`crate::merge`]), and records the outcomes as durable *edit
//! intents* in the queue (or pending rule proposals). All network the planner may cause
//! belongs to providers (MusicBrainz/LLM); by construction it holds no lastfm-edit client
//! at all.
//!
//! ## Coverage
//!
//! Each provider records time coverage and fingerprints of analyzed record revisions.
//! Incremental passes compare current records even inside covered time, so lost discovery
//! events and late metadata updates cannot hide work. Checkpoints are saved after the
//! pass's suggestions are durably enqueued; an interrupted pass safely repeats analysis. The rules provider's
//! coverage is invalidated automatically when the active rule set changes (tracked by
//! [`rules_hash`]); other providers' coverage only resets explicitly.

use crate::error::{Result, ScrubberError};
use crate::events::{PlanReport, ScrubberEvent, ScrubberEventBus, ScrubberEventReceiver};
use crate::feed::{FeedBatch, ScrubFeed};
use crate::merge::{changes, merge, restrict, EditField, Proposal};
use crate::policy::{Ceiling, EditDecision, Policy};
use crate::provider::{BoxedProvider, ScrubActionSuggestion};
use crate::queue::{IntentState, QueueEvent, QueueEventKind};
use crate::rewrite::RewriteRule;
use crate::state::{rules_hash, ProviderCoverage, ScrubberState};
use crate::subject::{group_by_subject, ScrobbleKey, Subject};
use lastfm_edit::ScrobbleEdit;
use scrobble_store::{CoverageMap, ScrobbleId, ScrobbleRecord, Segment, Storage};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use uuid::Uuid;

/// The provider name whose coverage is keyed to the active rule set.
pub const RULES_PROVIDER: &str = "rewrite_rules";

/// Everything a planning pass consults besides the records themselves.
struct PlanContext {
    rule_snapshots: HashMap<String, BoxedProvider>,
    coverages: HashMap<String, ProviderCoverage>,
    open_intents: Vec<crate::queue::EditIntent>,
    pending_rules: Vec<crate::queue::PendingRule>,
    dismissed: HashSet<Subject>,
    /// Subjects with an open intent (loaded or queued this pass).
    occupants: HashMap<Subject, Occupant>,
    /// Edits a human already rejected, by subject and target; never re-queued.
    rejected: HashSet<(Subject, EditTarget)>,
    /// Per key, the best rank that set each field in an applied edit that produced it.
    owned: HashMap<ScrobbleKey, BTreeMap<EditField, usize>>,
}

/// The open intent on a subject.
enum Occupant {
    /// Ready or in progress. The executor may be claiming it, so planning never touches it.
    Locked,
    /// Awaiting approval: a merge that differs replaces it.
    Supersedable {
        id: Uuid,
        edit: Box<ScrobbleEdit>,
        sources: BTreeMap<EditField, String>,
        rationale: Option<String>,
    },
}

/// The fields an edit sets, which is what a human rejected (timestamps and originals
/// vary between proposals of the same edit).
type EditTarget = (Option<String>, String, Option<String>, Option<String>);

fn edit_target(edit: &ScrobbleEdit) -> EditTarget {
    (
        edit.track_name.clone(),
        edit.artist_name.clone(),
        edit.album_name.clone(),
        edit.album_artist_name.clone(),
    )
}

/// The key of the records an applied edit of `subject` produced.
fn resulting_key(subject: &Subject, edit: &ScrobbleEdit) -> ScrobbleKey {
    let changed = changes(edit);
    let value = |field| changed.get(&field).map(|value| value.to_string());
    ScrobbleKey {
        artist: value(EditField::Artist).unwrap_or_else(|| subject.artist.clone()),
        track: value(EditField::Track).unwrap_or_else(|| subject.track.clone()),
        album: value(EditField::Album).or_else(|| subject.album.clone()),
    }
}

/// A provider and the most it may decide on its own. Its rank is its position.
struct Corrector {
    provider: BoxedProvider,
    ceiling: Ceiling,
}

pub struct Planner {
    store: Arc<dyn Storage>,
    state: Arc<dyn ScrubberState>,
    correctors: Vec<Corrector>,
    policy: Policy,
    events: ScrubberEventBus,
    batch_hint: usize,
}

impl Planner {
    pub fn new(store: Arc<dyn Storage>, state: Arc<dyn ScrubberState>) -> Self {
        Self {
            store,
            state,
            correctors: Vec::new(),
            policy: Policy::default(),
            events: ScrubberEventBus::new(),
            batch_hint: 50,
        }
    }

    /// Add a corrector at `Ceiling::Auto`, ranked after those already added.
    pub fn with_provider<P>(self, provider: P) -> Self
    where
        P: crate::provider::ScrubActionProvider + 'static,
        P::Error: Into<crate::provider::ActionProviderError>,
    {
        self.with_corrector(provider, Ceiling::Auto)
    }

    /// Add a corrector ranked after those already added. When several correctors edit one
    /// subject, each field takes the best-ranked value.
    pub fn with_corrector<P>(mut self, provider: P, ceiling: Ceiling) -> Self
    where
        P: crate::provider::ScrubActionProvider + 'static,
        P::Error: Into<crate::provider::ActionProviderError>,
    {
        self.correctors.push(Corrector {
            provider: crate::provider::boxed(provider),
            ceiling,
        });
        self
    }

    fn rank_of(&self, provider: &str) -> Option<usize> {
        self.correctors
            .iter()
            .position(|corrector| corrector.provider.provider_name() == provider)
    }

    pub fn with_policy(mut self, policy: Policy) -> Self {
        self.policy = policy;
        self
    }

    pub fn with_event_bus(mut self, events: ScrubberEventBus) -> Self {
        self.events = events;
        self
    }

    pub fn with_batch_hint(mut self, batch_hint: usize) -> Self {
        self.batch_hint = batch_hint.max(1);
        self
    }

    pub fn subscribe(&self) -> ScrubberEventReceiver {
        self.events.subscribe()
    }

    pub fn event_bus(&self) -> ScrubberEventBus {
        self.events.clone()
    }

    fn now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    /// One planning pass over a feed.
    pub async fn plan(&self, feed: &ScrubFeed) -> Result<PlanReport> {
        self.events
            .emit(ScrubberEvent::PlanStarted { feed: feed.clone() });
        let result = self.plan_inner(feed).await;
        match &result {
            Ok(report) => self.events.emit(ScrubberEvent::PlanCompleted {
                report: report.clone(),
            }),
            Err(err) => self.events.emit(ScrubberEvent::Error {
                error: err.to_string(),
            }),
        }
        result
    }

    /// Plan over an arbitrary batch of records pushed from anywhere (a sync-event bridge,
    /// a UI selection, another process) — the actor-style entry point. Runs the same
    /// dedupe → analyze → policy → enqueue pipeline as feed-driven planning, but makes no
    /// planning-coverage claims: a pushed batch says nothing about a *time range* being
    /// fully analyzed, so coverage-driven passes may later re-visit the same subjects
    /// (the dismissed/open-intent filters absorb the overlap).
    pub async fn plan_records(&self, records: &[ScrobbleRecord]) -> Result<PlanReport> {
        let result = self.plan_records_inner(records).await;
        match &result {
            Ok(report) => self.events.emit(ScrubberEvent::PlanCompleted {
                report: report.clone(),
            }),
            Err(err) => self.events.emit(ScrubberEvent::Error {
                error: err.to_string(),
            }),
        }
        result
    }

    async fn plan_records_inner(&self, records: &[ScrobbleRecord]) -> Result<PlanReport> {
        let mut context = self.load_context().await?;
        let batch = FeedBatch {
            records: records.to_vec(),
            coverage_claim: None,
        };
        let mut report = PlanReport::default();
        self.plan_batch(&batch, &mut context, &mut report, false)
            .await?;
        self.save_context(&context).await?;
        Ok(report)
    }

    async fn plan_inner(&self, feed: &ScrubFeed) -> Result<PlanReport> {
        let mut context = self.load_context().await?;

        // Pin synced intervals, then compare their current records with durable analysis revisions.
        let incremental_work = match feed {
            ScrubFeed::Incremental { window } => Some(self.incremental_work(window.clone()).await?),
            _ => None,
        };
        let batches = feed
            .batches(
                self.store.as_ref(),
                incremental_work.as_ref(),
                self.batch_hint,
            )
            .await?;

        let mut report = PlanReport::default();
        for batch in batches {
            self.plan_batch(
                &batch,
                &mut context,
                &mut report,
                matches!(feed, ScrubFeed::Incremental { .. }),
            )
            .await?;
        }
        // Spot feeds (one artist, album, ids) stay within what was asked for.
        if matches!(feed, ScrubFeed::Incremental { .. }) {
            self.drain_change_feeds(&mut context, &mut report).await?;
        }
        self.save_context(&context).await?;
        Ok(report)
    }

    async fn save_context(&self, context: &PlanContext) -> Result<()> {
        if !self.policy.dry_run {
            for (name, coverage) in &context.coverages {
                self.state.save_provider_coverage(name, coverage).await?;
            }
        }
        Ok(())
    }

    /// Re-plan keys that providers' change feeds report since their saved cursor, with
    /// every provider and no coverage claim. The advanced cursor is saved with the rest of
    /// the pass's checkpoints, after its suggestions are enqueued.
    ///
    /// No subject is lost: a feed reports every key with an event after the cursor in one
    /// snapshot and returns that snapshot's head as the next cursor. Record fingerprints
    /// don't change when only a provider's evidence does, so a key resolved after its
    /// records were analyzed is re-planned here. Overlap is absorbed by `occupants`.
    async fn drain_change_feeds(
        &self,
        context: &mut PlanContext,
        report: &mut PlanReport,
    ) -> Result<()> {
        for Corrector { provider, .. } in &self.correctors {
            let name = provider.provider_name().to_string();
            let cursor = context.coverages[&name].change_cursor.unwrap_or(0);
            let Some(changes) = provider.changes_since(cursor) else {
                continue;
            };
            // Chunk whole keys so a subject is never split across batches.
            let mut chunk: Vec<ScrobbleRecord> = Vec::new();
            for key in &changes.keys {
                chunk.extend(key.live_records(self.store.as_ref()).await?);
                if chunk.len() >= self.batch_hint {
                    let batch = FeedBatch {
                        records: std::mem::take(&mut chunk),
                        coverage_claim: None,
                    };
                    self.plan_batch(&batch, context, report, false).await?;
                }
            }
            if !chunk.is_empty() {
                let batch = FeedBatch {
                    records: chunk,
                    coverage_claim: None,
                };
                self.plan_batch(&batch, context, report, false).await?;
            }
            context
                .coverages
                .get_mut(&name)
                .expect("loaded for all providers")
                .change_cursor = Some(changes.next_cursor);
        }
        Ok(())
    }

    /// Everything a planning pass consults besides the records themselves.
    async fn load_context(&self) -> Result<PlanContext> {
        if self.correctors.is_empty() {
            return Err(ScrubberError::Provider {
                provider: "planner".into(),
                message: "no providers configured".into(),
            });
        }

        // Load provider coverage, applying the rules-hash invalidation rule.
        let rules = self.state.load_rules().await?;
        let current_rules_hash = rules_hash(&rules);
        let rule_snapshots = self
            .correctors
            .iter()
            .filter_map(|Corrector { provider, .. }| {
                provider
                    .rules_snapshot(&rules)
                    .map(|snapshot| (provider.provider_name().to_string(), snapshot))
            })
            .collect();
        let mut coverages: HashMap<String, ProviderCoverage> = HashMap::new();
        for Corrector { provider, .. } in &self.correctors {
            let name = provider.provider_name().to_string();
            let mut coverage = self.state.load_provider_coverage(&name).await?;
            if coverage.analysis_version != 1
                || (name == RULES_PROVIDER
                    && coverage.rules_hash.as_deref() != Some(&current_rules_hash))
            {
                if !coverage.coverage.is_empty() {
                    log::info!("planner: rule set changed; resetting '{name}' planning coverage");
                }
                coverage = ProviderCoverage {
                    coverage: CoverageMap::new(),
                    rules_hash: (name == RULES_PROVIDER).then(|| current_rules_hash.clone()),
                    analysis_version: 1,
                    change_cursor: coverage.change_cursor,
                    generation: coverage.generation,
                    ..ProviderCoverage::default()
                };
            }
            let generation = provider.generation();
            if coverage.generation != generation {
                if coverage.generation.is_some() {
                    log::info!("planner: '{name}' configuration changed; re-planning");
                }
                coverage = ProviderCoverage {
                    rules_hash: coverage.rules_hash,
                    analysis_version: 1,
                    generation,
                    ..ProviderCoverage::default()
                };
            }
            coverages.insert(name, coverage);
        }

        let queue = self.state.load_queue().await?;
        let mut rejected = HashSet::new();
        let mut occupants = HashMap::new();
        let mut owned: HashMap<ScrobbleKey, BTreeMap<EditField, usize>> = HashMap::new();
        for intent in &queue {
            match &intent.state {
                IntentState::Rejected { .. } => {
                    rejected.insert((intent.subject.clone(), edit_target(&intent.proposed)));
                }
                IntentState::AwaitingApproval => {
                    occupants.insert(
                        intent.subject.clone(),
                        Occupant::Supersedable {
                            id: intent.id,
                            edit: intent.proposed.clone(),
                            sources: intent.sources.clone(),
                            rationale: intent.rationale.clone(),
                        },
                    );
                }
                IntentState::Ready | IntentState::InProgress => {
                    occupants.insert(intent.subject.clone(), Occupant::Locked);
                }
                IntentState::Applied => {
                    let fields = owned
                        .entry(resulting_key(&intent.subject, &intent.proposed))
                        .or_default();
                    for (field, provider) in &intent.sources {
                        if let Some(rank) = self.rank_of(provider) {
                            fields
                                .entry(*field)
                                .and_modify(|best| *best = (*best).min(rank))
                                .or_insert(rank);
                        }
                    }
                }
                IntentState::Abandoned { .. } => {}
            }
        }
        let open_intents: Vec<_> = queue
            .into_iter()
            .filter(|intent| intent.state.is_open())
            .collect();
        let dismissed = self.state.load_dismissed().await?;
        let pending_rules: Vec<_> = self
            .state
            .load_pending_rules()
            .await?
            .into_iter()
            .filter(|rule| rule.state == crate::queue::PendingRuleState::Open)
            .collect();

        Ok(PlanContext {
            rule_snapshots,
            coverages,
            open_intents,
            pending_rules,
            dismissed,
            occupants,
            rejected,
            owned,
        })
    }

    /// Scan current records within synced time; per-provider fingerprints select work.
    async fn incremental_work(&self, window: Option<std::ops::Range<u64>>) -> Result<CoverageMap> {
        let sync_coverage = self.store.load_coverage().await?;
        let mut work_segments = Vec::new();
        for segment in sync_coverage.segments() {
            let mut range = segment.range();
            if let Some(window) = &window {
                range.start = range.start.max(window.start);
                range.end = range.end.min(window.end);
            }
            if range.start >= range.end {
                continue;
            }
            work_segments.push(Segment::new(range.start, range.end, 0));
        }
        Ok(CoverageMap::from_segments(work_segments))
    }

    async fn plan_batch(
        &self,
        batch: &FeedBatch,
        context: &mut PlanContext,
        report: &mut PlanReport,
        skip_covered: bool,
    ) -> Result<()> {
        // Dedup to subjects, keeping instance stats for representative tracks.
        let subjects: Vec<(Subject, Vec<ScrobbleId>)> = group_by_subject(&batch.records)
            .into_iter()
            .filter(|(subject, _)| {
                !context.dismissed.contains(subject)
                    && !matches!(context.occupants.get(subject), Some(Occupant::Locked))
            })
            .collect();
        self.events.emit(ScrubberEvent::SubjectsFound {
            count: subjects.len(),
            batch_range: batch.coverage_claim.clone(),
        });
        report.subjects_seen += subjects.len() as u64;

        let fingerprints: HashMap<_, _> = batch
            .records
            .iter()
            .map(|record| {
                let json = serde_json::to_vec(record)?;
                Ok((record.id.clone(), format!("{:x}", Sha256::digest(json))))
            })
            .collect::<Result<_>>()?;

        let mut proposals: BTreeMap<usize, Vec<Proposal>> = BTreeMap::new();
        let mut claims: Vec<(String, Vec<usize>)> = Vec::new();
        for (rank, corrector) in self.correctors.iter().enumerate() {
            let name = corrector.provider.provider_name().to_string();
            let provider = context
                .rule_snapshots
                .get(&name)
                .unwrap_or(&corrector.provider);
            let coverage = &context.coverages[&name];

            let selected: Vec<usize> = subjects
                .iter()
                .enumerate()
                .filter(|(_, (_, ids))| {
                    !skip_covered
                        || ids
                            .iter()
                            .any(|id| coverage.analyzed.get(id) != fingerprints.get(id))
                })
                .map(|(index, _)| index)
                .collect();
            let tracks: Vec<_> = selected
                .iter()
                .map(|&index| {
                    let (subject, ids) = &subjects[index];
                    subject
                        .representative_track(ids.len() as u32, ids.iter().map(|id| id.uts()).max())
                })
                .collect();

            if !tracks.is_empty() {
                let results = match provider
                    .analyze_tracks(
                        &tracks,
                        Some(&context.open_intents),
                        Some(&context.pending_rules),
                    )
                    .await
                {
                    Ok(results) => results,
                    Err(err) => {
                        // Provider failure: skip it for this batch and WITHHOLD its
                        // coverage claim so the range is retried next plan.
                        log::warn!("provider '{name}' failed on batch: {err}");
                        self.events.emit(ScrubberEvent::Error {
                            error: format!("provider '{name}': {err}"),
                        });
                        continue;
                    }
                };

                for (track_idx, suggestions) in results {
                    let Some(&index) = selected.get(track_idx) else {
                        log::warn!("provider '{name}' returned out-of-range index {track_idx}");
                        continue;
                    };
                    let subject = &subjects[index].0;
                    self.events.emit(ScrubberEvent::SubjectAnalyzed {
                        subject: subject.clone(),
                        suggestions: suggestions.len(),
                    });
                    for suggestion in suggestions {
                        report.suggestions += 1;
                        match suggestion.suggestion {
                            ScrubActionSuggestion::NoAction => {}
                            ScrubActionSuggestion::Edit(edit) => {
                                proposals.entry(index).or_default().push(Proposal {
                                    provider: suggestion.provider_name,
                                    rank,
                                    edit: *edit,
                                    requires_confirmation: corrector
                                        .ceiling
                                        .requires_confirmation(suggestion.requires_confirmation),
                                    rationale: suggestion.rationale,
                                });
                            }
                            ScrubActionSuggestion::ProposeRule { rule, motivation } => {
                                self.record_rule(
                                    subject,
                                    *rule,
                                    motivation,
                                    suggestion.provider_name,
                                    report,
                                )
                                .await?;
                            }
                        }
                    }
                }
            }
            claims.push((name, selected));
        }

        for (index, fresh) in proposals {
            self.record_merge(&subjects[index].0, fresh, context, report)
                .await?;
        }

        // Durably enqueued: claim the range for each corrector that analyzed it.
        // Dry runs claim nothing — they must leave the next real plan all its work.
        if self.policy.dry_run {
            return Ok(());
        }
        for (name, selected) in claims {
            let coverage = context
                .coverages
                .get_mut(&name)
                .expect("loaded for all providers");
            for &index in &selected {
                for id in &subjects[index].1 {
                    if let Some(fingerprint) = fingerprints.get(id) {
                        coverage.analyzed.insert(id.clone(), fingerprint.clone());
                    }
                }
            }
            if let Some(claim) = &batch.coverage_claim {
                let changes =
                    coverage
                        .coverage
                        .insert(Segment::new(claim.start, claim.end, Self::now()));
                if !changes.is_empty() {
                    self.events.emit(ScrubberEvent::CoverageAdvanced {
                        provider: name,
                        range: claim.clone(),
                    });
                }
            }
        }
        Ok(())
    }

    /// Merge one subject's fresh proposals with the intent awaiting approval on it, if any,
    /// and queue the result unless it matches that intent.
    async fn record_merge(
        &self,
        subject: &Subject,
        mut proposals: Vec<Proposal>,
        context: &mut PlanContext,
        report: &mut PlanReport,
    ) -> Result<()> {
        let superseded = match context.occupants.get(subject) {
            Some(Occupant::Locked) => return Ok(()),
            Some(Occupant::Supersedable {
                id,
                edit,
                sources,
                rationale,
            }) => {
                proposals.extend(self.open_proposals(edit, sources, rationale));
                Some((*id, changes(edit)))
            }
            None => None,
        };
        let unowned = BTreeMap::new();
        let owned = context.owned.get(&subject.key()).unwrap_or(&unowned);
        let Some(merged) = merge(&proposals, owned) else {
            return Ok(());
        };
        if superseded
            .as_ref()
            .is_some_and(|(_, open)| *open == changes(&merged.edit))
        {
            return Ok(());
        }
        if context
            .rejected
            .contains(&(subject.clone(), edit_target(&merged.edit)))
        {
            return Ok(());
        }
        let requires_approval = match self.policy.decide_edit(merged.requires_confirmation) {
            EditDecision::Report => {
                report.reported_only += 1;
                self.events.emit(ScrubberEvent::SuggestionReported {
                    subject: subject.clone(),
                    provider: merged.provider.clone(),
                    summary: summarize_edit(&merged.edit),
                });
                return Ok(());
            }
            EditDecision::Queue { requires_approval } => requires_approval,
        };

        let id = Uuid::new_v4();
        let mut events = Vec::new();
        if let Some((old, _)) = superseded {
            events.push(QueueEvent {
                id: old,
                at: Self::now(),
                kind: QueueEventKind::Abandoned {
                    reason: SUPERSEDED.into(),
                },
            });
        }
        events.push(QueueEvent {
            id,
            at: Self::now(),
            kind: QueueEventKind::Created {
                subject: subject.clone(),
                proposed: Box::new(merged.edit.clone()),
                provider: merged.provider.clone(),
                requires_approval,
                rationale: merged.rationale.clone(),
                sources: merged.sources.clone(),
            },
        });
        self.state.append_queue_events(&events).await?;

        if let Some((old, _)) = superseded {
            report.superseded += 1;
            self.events.emit(ScrubberEvent::IntentCompleted {
                id: old,
                state: IntentState::Abandoned {
                    reason: SUPERSEDED.into(),
                },
            });
        }
        let state = if requires_approval {
            report.queued_awaiting_approval += 1;
            context.occupants.insert(
                subject.clone(),
                Occupant::Supersedable {
                    id,
                    edit: Box::new(merged.edit),
                    sources: merged.sources,
                    rationale: merged.rationale,
                },
            );
            IntentState::AwaitingApproval
        } else {
            report.queued_ready += 1;
            context.occupants.insert(subject.clone(), Occupant::Locked);
            IntentState::Ready
        };
        self.events.emit(ScrubberEvent::IntentQueued {
            id,
            subject: subject.clone(),
            provider: merged.provider,
            state,
        });
        Ok(())
    }

    /// An intent awaiting approval, as one proposal per corrector that set its fields.
    /// They rank after fresh proposals of the same corrector, which are more current.
    /// Fields from correctors no longer configured are dropped.
    fn open_proposals(
        &self,
        edit: &ScrobbleEdit,
        sources: &BTreeMap<EditField, String>,
        rationale: &Option<String>,
    ) -> Vec<Proposal> {
        let mut by_provider: BTreeMap<&str, BTreeSet<EditField>> = BTreeMap::new();
        for (field, provider) in sources {
            by_provider.entry(provider).or_default().insert(*field);
        }
        let mut proposals: Vec<Proposal> = by_provider
            .into_iter()
            .filter_map(|(provider, fields)| {
                Some(Proposal {
                    provider: provider.to_string(),
                    rank: self.rank_of(provider)?,
                    edit: restrict(edit, &fields),
                    requires_confirmation: true,
                    rationale: None,
                })
            })
            .collect();
        proposals.sort_by_key(|proposal| proposal.rank);
        if let Some(lead) = proposals.first_mut() {
            lead.rationale = rationale.clone();
        }
        proposals
    }

    async fn record_rule(
        &self,
        subject: &Subject,
        rule: RewriteRule,
        motivation: String,
        provider: String,
        report: &mut PlanReport,
    ) -> Result<()> {
        if self.policy.dry_run {
            report.reported_only += 1;
            self.events.emit(ScrubberEvent::SuggestionReported {
                subject: subject.clone(),
                provider,
                summary: format!(
                    "propose rule '{}': {motivation}",
                    rule.name.as_deref().unwrap_or("unnamed")
                ),
            });
            return Ok(());
        }
        let id = Uuid::new_v4();
        let mut events = vec![crate::queue::RuleEvent {
            id,
            at: Self::now(),
            kind: crate::queue::RuleEventKind::Created {
                rule: Box::new(rule.clone()),
                motivation,
                provider: provider.clone(),
                example: Some(subject.clone()),
            },
        }];
        if self.policy.auto_approve_rules {
            events.push(crate::queue::RuleEvent {
                id,
                at: Self::now(),
                kind: crate::queue::RuleEventKind::Approved,
            });
            let mut rules = self.state.load_rules().await?;
            rules.push(rule);
            self.state.save_rules(&rules).await?;
            // Note: the rules-hash coverage reset takes effect on the NEXT plan;
            // this pass continues under the rule set it started with.
        }
        self.state.append_rule_events(&events).await?;
        report.rules_proposed += 1;
        self.events
            .emit(ScrubberEvent::PendingRuleCreated { id, provider });
        Ok(())
    }
}

/// The reason recorded on an intent replaced by a merge that differs from it.
pub const SUPERSEDED: &str = "superseded";

fn summarize_edit(edit: &ScrobbleEdit) -> String {
    let mut parts = Vec::new();
    if let (Some(from), Some(to)) = (&edit.track_name_original, &edit.track_name) {
        if from != to {
            parts.push(format!("track: '{from}' → '{to}'"));
        }
    }
    if edit.artist_name_original != edit.artist_name {
        parts.push(format!(
            "artist: '{}' → '{}'",
            edit.artist_name_original, edit.artist_name
        ));
    }
    if edit.album_name_original != edit.album_name {
        parts.push(format!(
            "album: {:?} → {:?}",
            edit.album_name_original, edit.album_name
        ));
    }
    if edit.album_artist_name_original != edit.album_artist_name {
        parts.push(format!(
            "album artist: {:?} → {:?}",
            edit.album_artist_name_original, edit.album_artist_name
        ));
    }
    if parts.is_empty() {
        "no changes".to_string()
    } else {
        parts.join("; ")
    }
}
