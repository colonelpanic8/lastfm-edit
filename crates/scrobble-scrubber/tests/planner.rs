//! Planner + feed behavior against in-memory store/state: chunking and claims, subject
//! dedup, the policy matrix, coverage-driven incrementality, provider-failure claim
//! withholding, and rules-hash invalidation.

use async_trait::async_trait;
use lastfm_edit::Track;
use scrobble_scrubber::provider::{
    ActionProviderError, ScrubActionProvider, SuggestionWithContext,
};
use scrobble_scrubber::queue::{EditIntent, PendingRule};
use scrobble_scrubber::{
    DismissedEntry, IntentState, MemoryScrubberState, Planner, Policy, RewriteRule,
    RewriteRulesScrubActionProvider, ScrubFeed, ScrubberState, SdRule, Subject,
};
use scrobble_store::{
    CoverageMap, MemoryStorage, Provenanced, RecordSource, ScrobbleId, ScrobbleRecord, Segment,
    Storage,
};
use std::sync::{Arc, Mutex};

fn record(uts: u64, artist: &str, track: &str) -> ScrobbleRecord {
    ScrobbleRecord {
        id: ScrobbleId::new(uts, artist, track),
        uts,
        artist: artist.to_string(),
        track: track.to_string(),
        album: Some("Album".to_string()),
        album_artist: Provenanced::Unknown,
        source: RecordSource::Api,
        fetched_at: 1,
        deleted: false,
        v: 1,
    }
}

fn remaster_rule() -> RewriteRule {
    RewriteRule::new()
        .with_name("strip remaster")
        .with_track_name(SdRule::new(r"^(?P<base>.+) - Remastered \d+$", "${base}"))
}

async fn seeded(records: &[ScrobbleRecord]) -> (Arc<MemoryStorage>, Arc<MemoryScrubberState>) {
    let store = Arc::new(MemoryStorage::new());
    store.append_scrobbles(records).await.unwrap();
    // Everything appended counts as synced.
    let mut sync = CoverageMap::new();
    sync.insert(Segment::new(0, u64::MAX, 1));
    store.save_coverage(&sync).await.unwrap();
    (store, Arc::new(MemoryScrubberState::new()))
}

/// Scripted provider: suggests a fixed track rename for tracks matching a needle, and
/// records every batch of track names it is asked to analyze.
struct ScriptedProvider {
    name: &'static str,
    needle: &'static str,
    requires_confirmation: bool,
    fail: bool,
    calls: Mutex<Vec<Vec<String>>>,
}

impl ScriptedProvider {
    fn new(name: &'static str, needle: &'static str) -> Arc<Self> {
        Arc::new(Self {
            name,
            needle,
            requires_confirmation: false,
            fail: false,
            calls: Mutex::new(Vec::new()),
        })
    }

    fn call_count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }

    fn analyzed_names(&self) -> Vec<String> {
        self.calls.lock().unwrap().concat()
    }
}

#[async_trait]
impl ScrubActionProvider for ScriptedProvider {
    type Error = ActionProviderError;

    async fn analyze_tracks(
        &self,
        tracks: &[Track],
        _open_intents: Option<&[EditIntent]>,
        _pending_rules: Option<&[PendingRule]>,
    ) -> Result<Vec<(usize, Vec<SuggestionWithContext>)>, Self::Error> {
        self.calls
            .lock()
            .unwrap()
            .push(tracks.iter().map(|t| t.name.clone()).collect());
        if self.fail {
            return Err(ActionProviderError("scripted failure".into()));
        }
        let mut results = Vec::new();
        for (idx, track) in tracks.iter().enumerate() {
            if track.name.contains(self.needle) {
                let mut edit = scrobble_scrubber::create_no_op_edit(track);
                edit.track_name = Some(track.name.replace(self.needle, "").trim().to_string());
                results.push((
                    idx,
                    vec![SuggestionWithContext::edit_with_confirmation(
                        edit,
                        self.requires_confirmation,
                        self.name.to_string(),
                    )],
                ));
            }
        }
        Ok(results)
    }

    fn provider_name(&self) -> &str {
        self.name
    }
}

// =====================================================================================
// Feed resolution
// =====================================================================================

#[tokio::test]
async fn store_range_claims_tile_and_same_second_groups_stay_together() {
    let (store, _) = seeded(&[
        record(100, "A", "t1"),
        record(200, "A", "t2"),
        record(200, "B", "t3"), // same second as t2
        record(300, "A", "t4"),
    ])
    .await;
    let feed = ScrubFeed::StoreRange {
        range: Some(50..400),
    };
    let batches = feed.batches(store.as_ref(), None, 2).await.unwrap();

    // Batch 1 must extend past the hint to keep the 200-second group together.
    assert_eq!(batches[0].records.len(), 3);
    assert_eq!(batches[0].coverage_claim, Some(50..201));
    assert_eq!(batches[1].records.len(), 1);
    assert_eq!(batches[1].coverage_claim, Some(201..400));
}

#[tokio::test]
async fn empty_range_still_claims() {
    let (store, _) = seeded(&[]).await;
    let feed = ScrubFeed::StoreRange {
        range: Some(10..20),
    };
    let batches = feed.batches(store.as_ref(), None, 5).await.unwrap();
    assert_eq!(batches.len(), 1);
    assert!(batches[0].records.is_empty());
    assert_eq!(batches[0].coverage_claim, Some(10..20));
}

#[tokio::test]
async fn spot_feeds_never_claim_and_ids_skip_tombstones() {
    let dead = {
        let mut r = record(400, "A", "gone");
        r.deleted = true;
        r
    };
    let (store, _) = seeded(&[record(100, "A", "t1"), record(200, "B", "t2"), dead.clone()]).await;

    let artist = ScrubFeed::Artist {
        name: "A".into(),
        range: None,
    };
    let batches = artist.batches(store.as_ref(), None, 10).await.unwrap();
    assert_eq!(batches.len(), 1);
    assert!(batches[0].coverage_claim.is_none());
    assert_eq!(batches[0].records.len(), 1);

    let ids = ScrubFeed::Ids(vec![
        record(100, "A", "t1").id,
        dead.id,
        ScrobbleId::new(999, "Z", "missing"),
    ]);
    let batches = ids.batches(store.as_ref(), None, 10).await.unwrap();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].records.len(), 1);
    assert!(batches[0].coverage_claim.is_none());
}

// =====================================================================================
// Planner
// =====================================================================================

#[tokio::test]
async fn dedupes_instances_queues_one_ready_intent_and_is_idempotent() {
    let (store, state) = seeded(&[
        record(100, "Queen", "You And I - Remastered 2011"),
        record(200, "Queen", "You And I - Remastered 2011"),
        record(300, "Queen", "You And I - Remastered 2011"),
        record(400, "Queen", "Clean Song"),
    ])
    .await;
    state.save_rules(&[remaster_rule()]).await.unwrap();
    let rules = state.load_rules().await.unwrap();

    let planner = Planner::new(store.clone(), state.clone())
        .with_provider(RewriteRulesScrubActionProvider::from_rules(rules.clone()));
    let report = planner
        .plan(&ScrubFeed::StoreRange { range: None })
        .await
        .unwrap();

    assert_eq!(report.subjects_seen, 2); // deduped
    assert_eq!(report.queued_ready, 1);
    assert_eq!(report.queued_awaiting_approval, 0);

    let queue = state.load_queue().await.unwrap();
    assert_eq!(queue.len(), 1);
    assert_eq!(queue[0].state, IntentState::Ready);
    assert_eq!(queue[0].proposed.track_name.as_deref(), Some("You And I"));

    // Second plan: coverage skips analysis entirely, no duplicate intents.
    let planner2 = Planner::new(store, state.clone())
        .with_provider(RewriteRulesScrubActionProvider::from_rules(rules));
    let report2 = planner2
        .plan(&ScrubFeed::StoreRange { range: None })
        .await
        .unwrap();
    assert_eq!(report2.queued_ready, 0);
    assert_eq!(state.load_queue().await.unwrap().len(), 1);
}

#[tokio::test]
async fn policy_matrix_dry_run_and_confirmation() {
    let (store, state) = seeded(&[record(100, "A", "Song Dirty")]).await;

    // Dry run: reported, nothing queued, and coverage does NOT advance (a later real
    // plan must still see this work).
    let provider = ScriptedProvider::new("scripted", "Dirty");
    let planner = Planner::new(store.clone(), state.clone())
        .with_provider(provider.clone())
        .with_policy(Policy {
            dry_run: true,
            ..Policy::default()
        });
    let report = planner
        .plan(&ScrubFeed::StoreRange { range: None })
        .await
        .unwrap();
    assert_eq!(report.reported_only, 1);
    assert!(state.load_queue().await.unwrap().is_empty());
    assert!(state
        .load_provider_coverage("scripted")
        .await
        .unwrap()
        .coverage
        .is_empty());

    // Force-confirmation: queued as AwaitingApproval.
    let planner = Planner::new(store, state.clone())
        .with_provider(provider)
        .with_policy(Policy {
            require_confirmation_all: true,
            ..Policy::default()
        });
    let report = planner
        .plan(&ScrubFeed::StoreRange { range: None })
        .await
        .unwrap();
    assert_eq!(report.queued_awaiting_approval, 1);
    let queue = state.load_queue().await.unwrap();
    assert_eq!(queue[0].state, IntentState::AwaitingApproval);
}

#[tokio::test]
async fn dismissed_and_occupied_subjects_are_filtered() {
    let (store, state) = seeded(&[
        record(100, "A", "Song Dirty"),
        record(200, "B", "Other Dirty"),
    ])
    .await;
    let dismissed_subject = Subject {
        artist: "A".into(),
        track: "Song Dirty".into(),
        album: Some("Album".into()),
        album_artist: None,
    };
    state
        .append_dismissed(&[DismissedEntry {
            subject: dismissed_subject,
            at: 1,
            reason: "manual".into(),
        }])
        .await
        .unwrap();

    let provider = ScriptedProvider::new("scripted", "Dirty");
    let planner = Planner::new(store.clone(), state.clone()).with_provider(provider.clone());
    planner
        .plan(&ScrubFeed::StoreRange { range: None })
        .await
        .unwrap();

    // Only B's subject reached the provider and got queued.
    assert_eq!(provider.analyzed_names(), vec!["Other Dirty"]);
    let queue = state.load_queue().await.unwrap();
    assert_eq!(queue.len(), 1);
    assert_eq!(queue[0].subject.artist, "B");

    // Re-plan with fresh coverage: B is now occupied by its open intent → filtered too.
    state
        .save_provider_coverage("scripted", &Default::default())
        .await
        .unwrap();
    let planner = Planner::new(store, state.clone()).with_provider(provider.clone());
    planner
        .plan(&ScrubFeed::StoreRange { range: None })
        .await
        .unwrap();
    assert_eq!(
        provider.call_count(),
        1,
        "no subjects left → provider not called"
    );
    assert_eq!(state.load_queue().await.unwrap().len(), 1);
}

#[tokio::test]
async fn incremental_respects_sync_and_planning_coverage() {
    let store = Arc::new(MemoryStorage::new());
    store
        .append_scrobbles(&[
            record(120, "A", "Early Dirty"),
            record(250, "B", "Late Dirty"),
        ])
        .await
        .unwrap();
    // Only [100, 200) is synced so far.
    let mut sync = CoverageMap::new();
    sync.insert(Segment::new(100, 200, 1));
    store.save_coverage(&sync).await.unwrap();
    let state = Arc::new(MemoryScrubberState::new());

    let provider = ScriptedProvider::new("scripted", "Dirty");
    let planner = Planner::new(store.clone(), state.clone()).with_provider(provider.clone());
    planner
        .plan(&ScrubFeed::Incremental { window: None })
        .await
        .unwrap();
    assert_eq!(
        provider.analyzed_names(),
        vec!["Early Dirty"],
        "unsynced time is not planned"
    );

    // Store syncs further; the next incremental pass plans only the new range.
    let mut sync = CoverageMap::new();
    sync.insert(Segment::new(100, 300, 2));
    store.save_coverage(&sync).await.unwrap();
    let planner = Planner::new(store, state.clone()).with_provider(provider.clone());
    planner
        .plan(&ScrubFeed::Incremental { window: None })
        .await
        .unwrap();
    assert_eq!(
        provider.analyzed_names(),
        vec!["Early Dirty", "Late Dirty"],
        "second pass analyzes only the newly synced range"
    );

    // Planning coverage now spans exactly the synced range.
    let coverage = state.load_provider_coverage("scripted").await.unwrap();
    assert!(coverage.coverage.covers(100..300));
    assert!(!coverage.coverage.contains(99));
}

#[tokio::test]
async fn provider_failure_withholds_only_that_providers_claim() {
    // Distinct subjects per provider, so one provider's queued intent doesn't occupy the
    // other's work.
    let (store, state) = seeded(&[
        record(100, "A", "Song Dirty"),
        record(200, "B", "Other Grimy"),
    ])
    .await;
    let good = ScriptedProvider::new("good", "Dirty");
    let bad = Arc::new(ScriptedProvider {
        name: "bad",
        needle: "Grimy",
        requires_confirmation: false,
        fail: true,
        calls: Mutex::new(Vec::new()),
    });

    let planner = Planner::new(store.clone(), state.clone())
        .with_provider(good.clone())
        .with_provider(bad.clone());
    planner
        .plan(&ScrubFeed::StoreRange { range: None })
        .await
        .unwrap();

    assert!(state
        .load_provider_coverage("good")
        .await
        .unwrap()
        .coverage
        .covers(0..u64::MAX));
    assert!(state
        .load_provider_coverage("bad")
        .await
        .unwrap()
        .coverage
        .is_empty());

    // Re-plan: good skips (covered), bad retries.
    let planner = Planner::new(store, state)
        .with_provider(good.clone())
        .with_provider(bad.clone());
    planner
        .plan(&ScrubFeed::StoreRange { range: None })
        .await
        .unwrap();
    assert_eq!(good.call_count(), 1);
    assert_eq!(bad.call_count(), 2);
}

#[tokio::test]
async fn rules_hash_change_resets_only_the_rules_provider_coverage() {
    let (store, state) = seeded(&[record(100, "A", "Song Dirty")]).await;
    state.save_rules(&[remaster_rule()]).await.unwrap();

    // A scripted provider masquerading as the rules provider (name-keyed reset).
    let provider = ScriptedProvider::new("rewrite_rules", "Dirty");
    let other = ScriptedProvider::new("other", "NOPE");

    let planner = Planner::new(store.clone(), state.clone())
        .with_provider(provider.clone())
        .with_provider(other.clone());
    planner
        .plan(&ScrubFeed::StoreRange { range: None })
        .await
        .unwrap();
    assert_eq!(provider.call_count(), 1);
    assert_eq!(other.call_count(), 1);

    // Same rules → both covered, no calls.
    let planner = Planner::new(store.clone(), state.clone())
        .with_provider(provider.clone())
        .with_provider(other.clone());
    planner
        .plan(&ScrubFeed::StoreRange { range: None })
        .await
        .unwrap();
    assert_eq!(provider.call_count(), 1);
    assert_eq!(other.call_count(), 1);

    // Rules change → only the rules provider replans. (Its subject is occupied by the
    // intent queued in pass one, so it sees an empty batch — but its coverage was reset
    // and re-claimed under the new hash.)
    state
        .save_rules(&[remaster_rule().with_name("renamed")])
        .await
        .unwrap();
    let planner = Planner::new(store, state.clone())
        .with_provider(provider.clone())
        .with_provider(other.clone());
    planner
        .plan(&ScrubFeed::StoreRange { range: None })
        .await
        .unwrap();
    assert_eq!(other.call_count(), 1, "non-rules provider stays covered");
    let coverage = state.load_provider_coverage("rewrite_rules").await.unwrap();
    assert!(coverage.coverage.covers(0..u64::MAX));
    assert_eq!(
        coverage.rules_hash,
        Some(scrobble_scrubber::rules_hash(
            &state.load_rules().await.unwrap()
        ))
    );
}
