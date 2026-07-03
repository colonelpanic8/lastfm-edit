//! The planner: conceives edits without ever touching last.fm.
//!
//! Resolves a [`ScrubFeed`] against the store, dedupes records into
//! [`Subject`]s, runs the provider stack over representative tracks, and records the
//! outcomes as durable *edit intents* in the queue (or pending rule proposals). All
//! network the planner may cause belongs to providers (MusicBrainz/LLM); by construction
//! it holds no lastfm-edit client at all.
//!
//! ## Coverage
//!
//! Each provider has its own planning [`CoverageMap`]: a covered instant means "every
//! scrobble at that instant has been analyzed by this provider under the current rules
//! generation". Claims are inserted only after a batch's suggestions are durably
//! enqueued, so a crash before that point simply replans the batch. The rules provider's
//! coverage is invalidated automatically when the active rule set changes (tracked by
//! [`rules_hash`]); other providers' coverage only resets explicitly.

use crate::error::{Result, ScrubberError};
use crate::events::{PlanReport, ScrubberEvent, ScrubberEventBus, ScrubberEventReceiver};
use crate::feed::{FeedBatch, ScrubFeed};
use crate::policy::{EditDecision, Policy};
use crate::provider::{BoxedProvider, ScrubActionSuggestion};
use crate::queue::{QueueEvent, QueueEventKind};
use crate::state::{rules_hash, ProviderCoverage, ScrubberState};
use crate::subject::{group_by_subject, Subject};
use scrobble_store::{CoverageMap, ScrobbleId, Segment, Storage};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use uuid::Uuid;

/// The provider name whose coverage is keyed to the active rule set.
pub const RULES_PROVIDER: &str = "rewrite_rules";

pub struct Planner {
    store: Arc<dyn Storage>,
    state: Arc<dyn ScrubberState>,
    providers: Vec<BoxedProvider>,
    policy: Policy,
    events: ScrubberEventBus,
    batch_hint: usize,
}

impl Planner {
    pub fn new(store: Arc<dyn Storage>, state: Arc<dyn ScrubberState>) -> Self {
        Self {
            store,
            state,
            providers: Vec::new(),
            policy: Policy::default(),
            events: ScrubberEventBus::new(),
            batch_hint: 50,
        }
    }

    pub fn with_provider<P>(mut self, provider: P) -> Self
    where
        P: crate::provider::ScrubActionProvider + 'static,
        P::Error: Into<crate::provider::ActionProviderError>,
    {
        self.providers.push(crate::provider::boxed(provider));
        self
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

    async fn plan_inner(&self, feed: &ScrubFeed) -> Result<PlanReport> {
        if self.providers.is_empty() {
            return Err(ScrubberError::Provider {
                provider: "planner".into(),
                message: "no providers configured".into(),
            });
        }

        // Load provider coverage, applying the rules-hash invalidation rule.
        let current_rules_hash = rules_hash(&self.state.load_rules().await?);
        let mut coverages: HashMap<String, ProviderCoverage> = HashMap::new();
        for provider in &self.providers {
            let name = provider.provider_name().to_string();
            let mut coverage = self.state.load_provider_coverage(&name).await?;
            if name == RULES_PROVIDER && coverage.rules_hash.as_deref() != Some(&current_rules_hash)
            {
                if !coverage.coverage.is_empty() {
                    log::info!("planner: rule set changed; resetting '{name}' planning coverage");
                }
                coverage = ProviderCoverage {
                    coverage: CoverageMap::new(),
                    rules_hash: Some(current_rules_hash.clone()),
                };
            }
            coverages.insert(name, coverage);
        }

        // Context for providers and dedup.
        let queue = self.state.load_queue().await?;
        let open_intents: Vec<_> = queue
            .into_iter()
            .filter(|intent| intent.state.is_open())
            .collect();
        let mut occupied_subjects: HashSet<Subject> = open_intents
            .iter()
            .map(|intent| intent.subject.clone())
            .collect();
        let dismissed = self.state.load_dismissed().await?;
        let pending_rules: Vec<_> = self
            .state
            .load_pending_rules()
            .await?
            .into_iter()
            .filter(|rule| rule.state == crate::queue::PendingRuleState::Open)
            .collect();

        // Resolve the feed. For Incremental: work = union over providers of their gaps,
        // restricted to synced coverage (and the optional window).
        let incremental_work = match feed {
            ScrubFeed::Incremental { window } => {
                Some(self.incremental_work(&coverages, window.clone()).await?)
            }
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
                &mut coverages,
                &open_intents,
                &pending_rules,
                &dismissed,
                &mut occupied_subjects,
                &mut report,
            )
            .await?;
        }
        Ok(report)
    }

    /// Work remaining for an incremental pass: within synced time (∩ window), any range
    /// at least one provider hasn't planned yet.
    async fn incremental_work(
        &self,
        coverages: &HashMap<String, ProviderCoverage>,
        window: Option<std::ops::Range<u64>>,
    ) -> Result<CoverageMap> {
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
            for coverage in coverages.values() {
                for gap in coverage.coverage.gaps(range.clone()) {
                    work_segments.push(Segment::new(gap.start, gap.end, 0));
                }
            }
        }
        Ok(CoverageMap::from_segments(work_segments))
    }

    #[allow(clippy::too_many_arguments)]
    async fn plan_batch(
        &self,
        batch: &FeedBatch,
        coverages: &mut HashMap<String, ProviderCoverage>,
        open_intents: &[crate::queue::EditIntent],
        pending_rules: &[crate::queue::PendingRule],
        dismissed: &HashSet<Subject>,
        occupied_subjects: &mut HashSet<Subject>,
        report: &mut PlanReport,
    ) -> Result<()> {
        // Dedup to subjects, keeping instance stats for representative tracks.
        let groups = group_by_subject(&batch.records);
        let mut subjects: Vec<(Subject, Vec<ScrobbleId>)> = Vec::new();
        for (subject, ids) in groups {
            if dismissed.contains(&subject) || occupied_subjects.contains(&subject) {
                continue;
            }
            subjects.push((subject, ids));
        }
        self.events.emit(ScrubberEvent::SubjectsFound {
            count: subjects.len(),
            batch_range: batch.coverage_claim.clone(),
        });
        report.subjects_seen += subjects.len() as u64;

        let tracks: Vec<lastfm_edit::Track> = subjects
            .iter()
            .map(|(subject, ids)| {
                let newest = ids.iter().map(|id| id.uts()).max();
                subject.representative_track(ids.len() as u32, newest)
            })
            .collect();

        for provider in &self.providers {
            let name = provider.provider_name().to_string();
            let coverage = coverages.get_mut(&name).expect("loaded for all providers");

            // Skip providers that already planned this whole range.
            if let Some(claim) = &batch.coverage_claim {
                if coverage.coverage.covers(claim.clone()) {
                    continue;
                }
            }

            if !tracks.is_empty() {
                let results = match provider
                    .analyze_tracks(&tracks, Some(open_intents), Some(pending_rules))
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
                    let Some((subject, _ids)) = subjects.get(track_idx) else {
                        log::warn!("provider '{name}' returned out-of-range index {track_idx}");
                        continue;
                    };
                    self.events.emit(ScrubberEvent::SubjectAnalyzed {
                        subject: subject.clone(),
                        suggestions: suggestions.len(),
                    });
                    for suggestion in suggestions {
                        report.suggestions += 1;
                        self.record_suggestion(subject, suggestion, occupied_subjects, report)
                            .await?;
                    }
                }
            }

            // Durably enqueued: claim the range for this provider and persist.
            // Dry runs claim nothing — they must leave the next real plan all its work.
            if self.policy.dry_run {
                continue;
            }
            if let Some(claim) = &batch.coverage_claim {
                let changes =
                    coverage
                        .coverage
                        .insert(Segment::new(claim.start, claim.end, Self::now()));
                if !changes.is_empty() {
                    self.state.save_provider_coverage(&name, coverage).await?;
                    self.events.emit(ScrubberEvent::CoverageAdvanced {
                        provider: name.clone(),
                        range: claim.clone(),
                    });
                }
            }
        }
        Ok(())
    }

    async fn record_suggestion(
        &self,
        subject: &Subject,
        suggestion: crate::provider::SuggestionWithContext,
        occupied_subjects: &mut HashSet<Subject>,
        report: &mut PlanReport,
    ) -> Result<()> {
        match suggestion.suggestion {
            ScrubActionSuggestion::NoAction => Ok(()),
            ScrubActionSuggestion::Edit(proposed) => {
                if occupied_subjects.contains(subject) {
                    return Ok(()); // another provider already queued this subject
                }
                match self.policy.decide_edit(suggestion.requires_confirmation) {
                    EditDecision::Report => {
                        report.reported_only += 1;
                        self.events.emit(ScrubberEvent::SuggestionReported {
                            subject: subject.clone(),
                            provider: suggestion.provider_name.clone(),
                            summary: summarize_edit(&proposed),
                        });
                        Ok(())
                    }
                    EditDecision::Queue { requires_approval } => {
                        let id = Uuid::new_v4();
                        let event = QueueEvent {
                            id,
                            at: Self::now(),
                            kind: QueueEventKind::Created {
                                subject: subject.clone(),
                                proposed,
                                provider: suggestion.provider_name.clone(),
                                requires_approval,
                            },
                        };
                        self.state.append_queue_events(&[event]).await?;
                        occupied_subjects.insert(subject.clone());
                        if requires_approval {
                            report.queued_awaiting_approval += 1;
                        } else {
                            report.queued_ready += 1;
                        }
                        self.events.emit(ScrubberEvent::IntentQueued {
                            id,
                            subject: subject.clone(),
                            provider: suggestion.provider_name,
                            state: if requires_approval {
                                crate::queue::IntentState::AwaitingApproval
                            } else {
                                crate::queue::IntentState::Ready
                            },
                        });
                        Ok(())
                    }
                }
            }
            ScrubActionSuggestion::ProposeRule { rule, motivation } => {
                if self.policy.dry_run {
                    report.reported_only += 1;
                    self.events.emit(ScrubberEvent::SuggestionReported {
                        subject: subject.clone(),
                        provider: suggestion.provider_name,
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
                        rule: rule.clone(),
                        motivation,
                        provider: suggestion.provider_name.clone(),
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
                    rules.push((*rule).clone());
                    self.state.save_rules(&rules).await?;
                    // Note: the rules-hash coverage reset takes effect on the NEXT plan;
                    // this pass continues under the rule set it started with.
                }
                self.state.append_rule_events(&events).await?;
                report.rules_proposed += 1;
                self.events.emit(ScrubberEvent::PendingRuleCreated {
                    id,
                    provider: suggestion.provider_name,
                });
                Ok(())
            }
        }
    }
}

fn summarize_edit(edit: &lastfm_edit::ScrobbleEdit) -> String {
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
