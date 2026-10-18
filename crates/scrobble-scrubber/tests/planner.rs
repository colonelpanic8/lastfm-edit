//! Planner + feed behavior against in-memory store/state: chunking and claims, subject
//! dedup, the policy matrix, coverage-driven incrementality, provider-failure claim
//! withholding, rules-hash invalidation, and merging ranked correctors.

use async_trait::async_trait;
use lastfm_edit::Track;
use scrobble_scrubber::provider::{
    ActionProviderError, ChangeBatch, ScrubActionProvider, SuggestionWithContext,
};
use scrobble_scrubber::queue::{EditIntent, PendingRule};
use scrobble_scrubber::{
    Ceiling, DismissedEntry, EditField, IntentState, MemoryScrubberState, Planner, Policy,
    QueueEvent, QueueEventKind, RewriteRule, RewriteRulesScrubActionProvider, ScrobbleKey,
    ScrubFeed, ScrubberState, SdRule, Subject,
};
use scrobble_store::{
    CoverageMap, MemoryStorage, Provenanced, RecordSource, ScrobbleId, ScrobbleRecord, Segment,
    Storage,
};
use std::collections::BTreeMap;
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

    // A rescan still avoids duplicate intents for occupied subjects.
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
            active: true,
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
        .plan(&ScrubFeed::Incremental { window: None })
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
        .plan(&ScrubFeed::Incremental { window: None })
        .await
        .unwrap();
    assert_eq!(provider.call_count(), 1);
    assert_eq!(other.call_count(), 1);

    // Same rules → both covered, no calls.
    let planner = Planner::new(store.clone(), state.clone())
        .with_provider(provider.clone())
        .with_provider(other.clone());
    planner
        .plan(&ScrubFeed::Incremental { window: None })
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
        .plan(&ScrubFeed::Incremental { window: None })
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

#[tokio::test]
async fn plan_store_does_not_claim_future_scrobbles() {
    let (store, state) = seeded(&[record(100, "Queen", "Clean")]).await;
    let mut coverage = CoverageMap::new();
    coverage.insert(Segment::new(0, 1000, 1));
    store.save_coverage(&coverage).await.unwrap();
    state.save_rules(&[remaster_rule()]).await.unwrap();
    let planner = Planner::new(store.clone(), state.clone()).with_provider(
        RewriteRulesScrubActionProvider::from_rules(vec![remaster_rule()]),
    );
    planner
        .plan(&ScrubFeed::StoreRange { range: None })
        .await
        .unwrap();
    store
        .append_scrobbles(&[record(1500, "Queen", "Song - Remastered 2011")])
        .await
        .unwrap();
    coverage.insert(Segment::new(1000, 2000, 2));
    store.save_coverage(&coverage).await.unwrap();
    planner
        .plan(&ScrubFeed::Incremental { window: None })
        .await
        .unwrap();
    assert_eq!(
        state.load_queue().await.unwrap().len(),
        1,
        "store pass preclaimed all future time"
    );
}

#[tokio::test]
async fn whole_store_rescans_but_only_claims_synced_intervals() {
    let (store, state) = seeded(&[
        record(100, "A", "clean"),
        record(200, "A", "hole"),
        record(300, "A", "clean"),
    ])
    .await;
    store
        .save_coverage(&CoverageMap::from_segments(vec![
            Segment::new(50, 150, 1),
            Segment::new(250, 350, 1),
        ]))
        .await
        .unwrap();
    let provider = ScriptedProvider::new("test", "never");
    let planner = Planner::new(store, state.clone()).with_provider(provider.clone());
    planner
        .plan(&ScrubFeed::StoreRange { range: None })
        .await
        .unwrap();
    assert!(provider.analyzed_names().contains(&"hole".into()));
    let coverage = state.load_provider_coverage("test").await.unwrap().coverage;
    assert!(coverage.covers(50..150));
    assert!(coverage.covers(250..350));
    assert!(!coverage.covers(150..250));
    assert!(!coverage.covers(350..351));
    let calls = provider.call_count();
    planner
        .plan(&ScrubFeed::StoreRange { range: None })
        .await
        .unwrap();
    assert!(
        provider.call_count() > calls,
        "explicit store pass rescans covered data"
    );
}

#[tokio::test]
async fn rule_change_uses_new_rules_in_existing_planner() {
    let (store, state) = seeded(&[record(100, "Queen", "Song - Remastered 2011")]).await;
    let planner = Planner::new(store, state.clone())
        .with_provider(RewriteRulesScrubActionProvider::from_rules(vec![]));
    let feed = ScrubFeed::Incremental { window: None };
    planner.plan(&feed).await.unwrap();
    state.save_rules(&[remaster_rule()]).await.unwrap();
    planner.plan(&feed).await.unwrap();
    assert_eq!(
        state.load_queue().await.unwrap().len(),
        1,
        "new rules hash was covered using old provider rules"
    );
}

#[tokio::test]
async fn rule_changes_during_a_pass_apply_only_to_the_next_pass() {
    struct ChangeRules(Arc<MemoryScrubberState>);
    #[async_trait]
    impl ScrubActionProvider for ChangeRules {
        type Error = ActionProviderError;
        fn provider_name(&self) -> &str {
            "change_rules"
        }
        async fn analyze_tracks(
            &self,
            _tracks: &[Track],
            _open: Option<&[EditIntent]>,
            _pending: Option<&[PendingRule]>,
        ) -> Result<Vec<(usize, Vec<SuggestionWithContext>)>, ActionProviderError> {
            self.0.save_rules(&[remaster_rule()]).await.unwrap();
            Ok(vec![])
        }
    }
    let (store, state) = seeded(&[
        record(100, "Queen", "First - Remastered 2011"),
        record(200, "Queen", "Second - Remastered 2011"),
    ])
    .await;
    let planner = Planner::new(store, state.clone())
        .with_provider(ChangeRules(state.clone()))
        .with_provider(RewriteRulesScrubActionProvider::from_rules(vec![]))
        .with_batch_hint(1);
    let feed = ScrubFeed::Incremental { window: None };
    planner.plan(&feed).await.unwrap();
    assert!(state.load_queue().await.unwrap().is_empty());
    assert_eq!(
        state
            .load_provider_coverage("rewrite_rules")
            .await
            .unwrap()
            .rules_hash,
        Some(scrobble_scrubber::rules_hash(&[]))
    );
    planner.plan(&feed).await.unwrap();
    assert_eq!(state.load_queue().await.unwrap().len(), 2);
}

#[tokio::test]
async fn restart_finds_late_arrivals_and_metadata_updates_without_events() {
    use scrobble_scrubber::FsScrubberState;
    use scrobble_store::FsStorage;
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(FsStorage::open(dir.path()).unwrap());
    let state = Arc::new(FsScrubberState::open(dir.path().join("scrubber")).unwrap());
    store
        .append_scrobbles(&[record(100, "Queen", "Clean")])
        .await
        .unwrap();
    store
        .save_coverage(&CoverageMap::from_segments(vec![Segment::new(0, 1000, 1)]))
        .await
        .unwrap();
    let album_rule = RewriteRule::new().with_album_name(SdRule::new("^(.*) Dirty$", "$1"));
    state
        .save_rules(&[remaster_rule(), album_rule])
        .await
        .unwrap();
    // Legacy coverage must not pre-cover future records or suppress the initial analysis.
    state
        .save_provider_coverage(
            "rewrite_rules",
            &scrobble_scrubber::ProviderCoverage {
                coverage: CoverageMap::from_segments(vec![Segment::new(0, u64::MAX, 1)]),
                rules_hash: Some(scrobble_scrubber::rules_hash(
                    &state.load_rules().await.unwrap(),
                )),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let planner = Planner::new(store.clone(), state.clone())
        .with_provider(RewriteRulesScrubActionProvider::from_rules(vec![]));
    let feed = ScrubFeed::Incremental { window: None };
    planner.plan(&feed).await.unwrap();
    let coverage = state.load_provider_coverage("rewrite_rules").await.unwrap();
    assert_eq!(coverage.analyzed.len(), 1);
    assert!(!coverage.coverage.contains(1000));
    let mut updated = record(100, "Queen", "Clean");
    updated.album = Some("Album Dirty".into());
    updated.fetched_at = 2;
    let stats = store
        .append_scrobbles(&[updated, record(200, "Queen", "Late - Remastered 2011")])
        .await
        .unwrap();
    assert_eq!((stats.new, stats.updated), (1, 1));
    drop(planner);
    drop(store);
    drop(state);
    let store = Arc::new(FsStorage::open(dir.path()).unwrap());
    let state = Arc::new(FsScrubberState::open(dir.path().join("scrubber")).unwrap());
    let planner = Planner::new(store, state.clone())
        .with_provider(RewriteRulesScrubActionProvider::from_rules(vec![]));
    planner.plan(&feed).await.unwrap();
    let queue = state.load_queue().await.unwrap();
    assert_eq!(queue.len(), 2);
    assert!(
        queue.iter().any(
            |i| i.proposed.album_name.as_deref() == Some("Album") && i.subject.track == "Clean"
        ),
        "{queue:?}"
    );
    assert!(queue
        .iter()
        .any(|i| i.proposed.track_name.as_deref() == Some("Late")));
    assert_eq!(planner.plan(&feed).await.unwrap().queued_ready, 0);
}

#[tokio::test]
async fn records_deferred_by_an_open_intent_are_revisited_after_it_closes() {
    let (store, state) = seeded(&[record(100, "A", "Song Dirty")]).await;
    let provider = ScriptedProvider::new("scripted", "Dirty");
    let planner = Planner::new(store.clone(), state.clone()).with_provider(provider);
    let feed = ScrubFeed::Incremental { window: None };
    planner.plan(&feed).await.unwrap();
    store
        .append_scrobbles(&[record(200, "A", "Song Dirty")])
        .await
        .unwrap();
    planner.plan(&feed).await.unwrap();
    let queue = state.load_queue().await.unwrap();
    assert_eq!(queue.len(), 1);
    state
        .append_queue_events(&[scrobble_scrubber::QueueEvent {
            id: queue[0].id,
            at: u64::MAX,
            kind: scrobble_scrubber::QueueEventKind::Completed,
        }])
        .await
        .unwrap();
    planner.plan(&feed).await.unwrap();
    assert_eq!(state.load_queue().await.unwrap().len(), 2);
}

// =====================================================================================
// Change feeds
// =====================================================================================

/// A provider whose answers depend on external evidence that arrives later: it renames a
/// track only once its key has been "resolved", and reports each resolution through its
/// change feed (seq = position in the resolution list).
struct LateEvidenceProvider {
    resolved: Mutex<Vec<(ScrobbleKey, String)>>,
    analyzed: Mutex<Vec<String>>,
    generation: Mutex<Option<String>>,
}

impl LateEvidenceProvider {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            resolved: Mutex::new(Vec::new()),
            analyzed: Mutex::new(Vec::new()),
            generation: Mutex::new(None),
        })
    }

    fn resolve(&self, key: ScrobbleKey, title: &str) {
        self.resolved.lock().unwrap().push((key, title.to_string()));
    }

    fn analyzed(&self) -> Vec<String> {
        self.analyzed.lock().unwrap().clone()
    }
}

#[async_trait]
impl ScrubActionProvider for LateEvidenceProvider {
    type Error = ActionProviderError;

    async fn analyze_tracks(
        &self,
        tracks: &[Track],
        _open_intents: Option<&[EditIntent]>,
        _pending_rules: Option<&[PendingRule]>,
    ) -> Result<Vec<(usize, Vec<SuggestionWithContext>)>, Self::Error> {
        let resolved = self.resolved.lock().unwrap();
        let mut results = Vec::new();
        for (idx, track) in tracks.iter().enumerate() {
            self.analyzed.lock().unwrap().push(track.name.clone());
            let key = ScrobbleKey {
                artist: track.artist.clone(),
                track: track.name.clone(),
                album: track.album.clone(),
            };
            if let Some((_, title)) = resolved.iter().rev().find(|(k, _)| *k == key) {
                let mut edit = scrobble_scrubber::create_no_op_edit(track);
                edit.track_name = Some(title.clone());
                results.push((
                    idx,
                    vec![SuggestionWithContext::edit_with_confirmation(
                        edit,
                        false,
                        "late".to_string(),
                    )],
                ));
            }
        }
        Ok(results)
    }

    fn provider_name(&self) -> &str {
        "late"
    }

    fn generation(&self) -> Option<String> {
        self.generation.lock().unwrap().clone()
    }

    fn changes_since(&self, cursor: u64) -> Option<ChangeBatch> {
        let resolved = self.resolved.lock().unwrap();
        Some(ChangeBatch {
            keys: resolved
                .iter()
                .skip(cursor as usize)
                .map(|(key, _)| key.clone())
                .collect(),
            next_cursor: resolved.len() as u64,
        })
    }
}

#[tokio::test]
async fn keys_resolved_after_their_range_was_planned_are_planned_once_via_the_change_feed() {
    let dirty = "Wild Horses - 2009 Re-Mastered Digital Version";
    let (store, state) = seeded(&[
        record(100, "The Rolling Stones", dirty),
        record(200, "The Rolling Stones", dirty),
        record(300, "The Rolling Stones", "Brown Sugar"),
    ])
    .await;
    let provider = LateEvidenceProvider::new();
    let planner = Planner::new(store, state.clone()).with_provider(provider.clone());
    let incremental = ScrubFeed::Incremental { window: None };

    planner.plan(&incremental).await.unwrap();
    assert!(state.load_queue().await.unwrap().is_empty());
    assert!(state
        .load_provider_coverage("late")
        .await
        .unwrap()
        .coverage
        .covers(100..301));

    provider.resolve(
        ScrobbleKey {
            artist: "The Rolling Stones".into(),
            track: dirty.into(),
            album: Some("Album".into()),
        },
        "Wild Horses",
    );
    let report = planner.plan(&incremental).await.unwrap();
    assert_eq!(report.queued_ready, 1);
    let queue = state.load_queue().await.unwrap();
    assert_eq!(queue.len(), 1);
    assert_eq!(queue[0].subject.track, dirty);
    assert_eq!(queue[0].proposed.track_name.as_deref(), Some("Wild Horses"));
    assert_eq!(
        state
            .load_provider_coverage("late")
            .await
            .unwrap()
            .change_cursor,
        Some(1)
    );
    assert_eq!(provider.analyzed(), vec!["Brown Sugar", dirty, dirty]);

    let report = planner.plan(&incremental).await.unwrap();
    assert_eq!(report.suggestions, 0);
    assert_eq!(state.load_queue().await.unwrap().len(), 1);
    assert_eq!(
        provider.analyzed().len(),
        3,
        "a drained cursor replans nothing"
    );
}

#[tokio::test]
async fn dry_run_drains_the_change_feed_without_persisting_its_cursor() {
    let dirty = "Cocaine - Remastered";
    let (store, state) = seeded(&[record(100, "Eric Clapton", dirty)]).await;
    let provider = LateEvidenceProvider::new();
    provider.resolve(
        ScrobbleKey {
            artist: "Eric Clapton".into(),
            track: dirty.into(),
            album: Some("Album".into()),
        },
        "Cocaine",
    );
    let planner = Planner::new(store, state.clone())
        .with_provider(provider.clone())
        .with_policy(Policy {
            dry_run: true,
            ..Policy::default()
        });

    let spot = planner.plan(&ScrubFeed::Ids(Vec::new())).await.unwrap();
    assert_eq!(
        spot.reported_only, 0,
        "spot feeds don't drain the change feed"
    );

    let report = planner
        .plan(&ScrubFeed::Incremental { window: Some(0..1) })
        .await
        .unwrap();
    assert_eq!(report.reported_only, 1);
    assert!(state.load_queue().await.unwrap().is_empty());
    assert_eq!(
        state
            .load_provider_coverage("late")
            .await
            .unwrap()
            .change_cursor,
        None
    );
}

#[tokio::test]
async fn a_rejected_edit_is_not_requeued_when_its_key_changes_again() {
    let dirty = "Cocaine - Remastered";
    let key = ScrobbleKey {
        artist: "Eric Clapton".into(),
        track: dirty.into(),
        album: Some("Album".into()),
    };
    let (store, state) = seeded(&[record(100, "Eric Clapton", dirty)]).await;
    let provider = LateEvidenceProvider::new();
    let planner = Planner::new(store, state.clone()).with_provider(provider.clone());
    let incremental = ScrubFeed::Incremental { window: None };

    provider.resolve(key.clone(), "Cocaine");
    planner.plan(&incremental).await.unwrap();
    let id = state.load_queue().await.unwrap()[0].id;
    scrobble_scrubber::reject_intent(state.as_ref(), id, false)
        .await
        .unwrap();

    provider.resolve(key.clone(), "Cocaine");
    let report = planner.plan(&incremental).await.unwrap();
    assert_eq!(report.queued_ready + report.queued_awaiting_approval, 0);
    assert_eq!(state.load_queue().await.unwrap().len(), 1);

    provider.resolve(key, "Cocaine (Live)");
    let report = planner.plan(&incremental).await.unwrap();
    assert_eq!(report.queued_ready, 1, "a different edit is still proposed");
}

#[tokio::test]
async fn a_configuration_change_replans_the_whole_change_feed() {
    let dirty = "Cocaine - Remastered";
    let (store, state) = seeded(&[record(100, "Eric Clapton", dirty)]).await;
    let provider = LateEvidenceProvider::new();
    let planner = Planner::new(store, state.clone()).with_provider(provider.clone());
    let incremental = ScrubFeed::Incremental { window: None };
    provider.resolve(
        ScrobbleKey {
            artist: "Eric Clapton".into(),
            track: dirty.into(),
            album: Some("Album".into()),
        },
        "Cocaine",
    );
    planner.plan(&incremental).await.unwrap();
    let id = state.load_queue().await.unwrap()[0].id;
    scrobble_scrubber::reject_intent(state.as_ref(), id, false)
        .await
        .unwrap();
    let analyzed = provider.analyzed().len();

    planner.plan(&incremental).await.unwrap();
    assert_eq!(provider.analyzed().len(), analyzed, "nothing new to plan");

    *provider.generation.lock().unwrap() = Some("compilations=off".into());
    planner.plan(&incremental).await.unwrap();
    assert!(
        provider.analyzed().len() > analyzed,
        "the new configuration re-analyzes resolved keys"
    );
    let coverage = state.load_provider_coverage("late").await.unwrap();
    assert_eq!(coverage.generation.as_deref(), Some("compilations=off"));
    assert_eq!(coverage.change_cursor, Some(1));
}

// =====================================================================================
// Correctors
// =====================================================================================

const MB: &str = "musicbrainz_identity";
const RULES: &str = "rewrite_rules";

fn record_on(uts: u64, artist: &str, track: &str, album: &str) -> ScrobbleRecord {
    ScrobbleRecord {
        album: Some(album.to_string()),
        ..record(uts, artist, track)
    }
}

fn key(artist: &str, track: &str, album: &str) -> ScrobbleKey {
    ScrobbleKey {
        artist: artist.into(),
        track: track.into(),
        album: Some(album.into()),
    }
}

type Sets = Vec<(EditField, &'static str)>;

/// Sets fixed fields on the tracks of the keys it knows. Keys added later are reported
/// through its change feed, like identity resolutions.
struct FixedProvider {
    name: &'static str,
    requires_confirmation: bool,
    edits: Mutex<Vec<(ScrobbleKey, Sets)>>,
}

impl FixedProvider {
    fn new(name: &'static str, edits: Vec<(ScrobbleKey, Sets)>) -> Arc<Self> {
        Arc::new(Self {
            name,
            requires_confirmation: false,
            edits: Mutex::new(edits),
        })
    }

    fn add(&self, key: ScrobbleKey, sets: Sets) {
        self.edits.lock().unwrap().push((key, sets));
    }
}

#[async_trait]
impl ScrubActionProvider for FixedProvider {
    type Error = ActionProviderError;

    async fn analyze_tracks(
        &self,
        tracks: &[Track],
        _open_intents: Option<&[EditIntent]>,
        _pending_rules: Option<&[PendingRule]>,
    ) -> Result<Vec<(usize, Vec<SuggestionWithContext>)>, Self::Error> {
        let edits = self.edits.lock().unwrap();
        let mut results = Vec::new();
        for (idx, track) in tracks.iter().enumerate() {
            let track_key = ScrobbleKey {
                artist: track.artist.clone(),
                track: track.name.clone(),
                album: track.album.clone(),
            };
            let Some((_, sets)) = edits.iter().rev().find(|(key, _)| *key == track_key) else {
                continue;
            };
            let mut edit = scrobble_scrubber::create_no_op_edit(track);
            for (field, value) in sets {
                let value = value.to_string();
                match field {
                    EditField::Track => edit.track_name = Some(value),
                    EditField::Artist => edit.artist_name = value,
                    EditField::Album => edit.album_name = Some(value),
                    EditField::AlbumArtist => edit.album_artist_name = Some(value),
                }
            }
            results.push((
                idx,
                vec![SuggestionWithContext::edit_with_confirmation(
                    edit,
                    self.requires_confirmation,
                    self.name.to_string(),
                )],
            ));
        }
        Ok(results)
    }

    fn provider_name(&self) -> &str {
        self.name
    }

    fn changes_since(&self, cursor: u64) -> Option<ChangeBatch> {
        let edits = self.edits.lock().unwrap();
        Some(ChangeBatch {
            keys: edits
                .iter()
                .skip(cursor as usize)
                .map(|(key, _)| key.clone())
                .collect(),
            next_cursor: edits.len() as u64,
        })
    }
}

fn sources(entries: &[(EditField, &str)]) -> BTreeMap<EditField, String> {
    entries
        .iter()
        .map(|(field, provider)| (*field, provider.to_string()))
        .collect()
}

#[tokio::test]
async fn corrector_order_decides_each_conflicting_field() {
    let cocaine = key("Eric Clapton", "Cocaine", "The Cream of Clapton");
    let mut got = Vec::new();
    for order in [[MB, RULES], [RULES, MB]] {
        let (store, state) = seeded(&[record_on(
            100,
            "Eric Clapton",
            "Cocaine",
            "The Cream of Clapton",
        )])
        .await;
        let mb = FixedProvider::new(
            MB,
            vec![(cocaine.clone(), vec![(EditField::Album, "Slowhand")])],
        );
        let rules = FixedProvider::new(
            RULES,
            vec![(
                cocaine.clone(),
                vec![
                    (EditField::Album, "The Cream Of Clapton"),
                    (EditField::AlbumArtist, "Eric Clapton"),
                ],
            )],
        );
        let planner = order
            .iter()
            .fold(Planner::new(store, state.clone()), |planner, name| {
                if *name == MB {
                    planner.with_provider(mb.clone())
                } else {
                    planner.with_provider(rules.clone())
                }
            });
        planner
            .plan(&ScrubFeed::StoreRange { range: None })
            .await
            .unwrap();
        let queue = state.load_queue().await.unwrap();
        let [intent] = queue.as_slice() else {
            panic!("one intent: {queue:?}");
        };
        got.push((
            intent.proposed.album_name.clone().unwrap(),
            intent.proposed.album_artist_name.clone(),
            intent.provider.clone(),
            intent.sources.clone(),
        ));
    }
    assert_eq!(
        got,
        vec![
            (
                "Slowhand".to_string(),
                None,
                MB.to_string(),
                sources(&[(EditField::Album, MB)]),
            ),
            (
                "The Cream Of Clapton".to_string(),
                Some("Eric Clapton".to_string()),
                RULES.to_string(),
                sources(&[(EditField::Album, RULES), (EditField::AlbumArtist, RULES)]),
            ),
        ]
    );
}

#[tokio::test]
async fn a_merge_supersedes_an_intent_awaiting_approval_but_never_a_ready_one() {
    let horses = "Wild Horses - 2009 Re-Mastered Digital Version";
    let album = "Sticky Fingers (Super Deluxe)";
    let strip = |requires_confirmation| {
        RewriteRule::new()
            .with_name("strip digital remaster")
            .with_track_name(SdRule::new(
                r"^(?P<base>.+) - \d{4} Re-Mastered Digital Version$",
                "${base}",
            ))
            .with_confirmation_required(requires_confirmation)
    };
    let incremental = ScrubFeed::Incremental { window: None };

    let mut got = Vec::new();
    for requires_confirmation in [true, false] {
        let (store, state) = seeded(&[record_on(100, "The Rolling Stones", horses, album)]).await;
        state
            .save_rules(&[strip(requires_confirmation)])
            .await
            .unwrap();
        let mb = FixedProvider::new(MB, Vec::new());
        let planner = Planner::new(store, state.clone())
            .with_provider(mb.clone())
            .with_provider(RewriteRulesScrubActionProvider::from_rules(Vec::new()));
        planner.plan(&incremental).await.unwrap();

        mb.add(
            key("The Rolling Stones", horses, album),
            vec![(EditField::Album, "Sticky Fingers")],
        );
        let report = planner.plan(&incremental).await.unwrap();
        // A full rescan re-proposes the same merge, which changes nothing.
        let rescan = planner
            .plan(&ScrubFeed::StoreRange { range: None })
            .await
            .unwrap();
        let queue: Vec<_> = state
            .load_queue()
            .await
            .unwrap()
            .into_iter()
            .map(|intent| {
                (
                    intent.state,
                    intent.proposed.track_name.unwrap(),
                    intent.proposed.album_name.unwrap(),
                    intent.provider,
                    intent.sources,
                )
            })
            .collect();
        got.push((
            report.superseded,
            report.queued_awaiting_approval + report.queued_ready,
            rescan.queued_awaiting_approval + rescan.queued_ready,
            queue,
        ));
    }

    assert_eq!(
        got,
        vec![
            (
                1,
                1,
                0,
                vec![
                    (
                        IntentState::Abandoned {
                            reason: "superseded".into()
                        },
                        "Wild Horses".to_string(),
                        album.to_string(),
                        RULES.to_string(),
                        sources(&[(EditField::Track, RULES)]),
                    ),
                    (
                        IntentState::AwaitingApproval,
                        "Wild Horses".to_string(),
                        "Sticky Fingers".to_string(),
                        MB.to_string(),
                        sources(&[(EditField::Track, RULES), (EditField::Album, MB)]),
                    ),
                ],
            ),
            (
                0,
                0,
                0,
                vec![(
                    IntentState::Ready,
                    "Wild Horses".to_string(),
                    album.to_string(),
                    RULES.to_string(),
                    sources(&[(EditField::Track, RULES)]),
                )],
            ),
        ]
    );
}

#[tokio::test]
async fn a_review_ceiling_holds_every_proposal_for_approval() {
    let cocaine = key("Eric Clapton", "Cocaine", "The Cream of Clapton");
    let mut got = Vec::new();
    for (ceiling, requires_confirmation) in [
        (Ceiling::Auto, false),
        (Ceiling::Review, false),
        (Ceiling::Auto, true),
    ] {
        let (store, state) = seeded(&[record_on(
            100,
            "Eric Clapton",
            "Cocaine",
            "The Cream of Clapton",
        )])
        .await;
        let mb = Arc::new(FixedProvider {
            name: MB,
            requires_confirmation,
            edits: Mutex::new(vec![(
                cocaine.clone(),
                vec![(EditField::Album, "Slowhand")],
            )]),
        });
        Planner::new(store, state.clone())
            .with_corrector(mb, ceiling)
            .plan(&ScrubFeed::StoreRange { range: None })
            .await
            .unwrap();
        got.push(state.load_queue().await.unwrap()[0].state.clone());
    }
    assert_eq!(
        got,
        vec![
            IntentState::Ready,
            IntentState::AwaitingApproval,
            IntentState::AwaitingApproval,
        ]
    );
}

#[tokio::test]
async fn a_lower_priority_corrector_cannot_undo_a_field_a_higher_one_applied() {
    let cocaine = "Cocaine - 2012 Remaster";
    let (store, state) = seeded(&[
        record_on(100, "Eric Clapton", cocaine, "Slowhand"),
        record_on(200, "Eric Clapton", "Wonderful Tonight", "Slowhand"),
    ])
    .await;
    for (at, track) in [(1, cocaine), (3, "Wonderful Tonight")] {
        let subject = Subject {
            artist: "Eric Clapton".into(),
            track: track.into(),
            album: Some("The Cream of Clapton".into()),
            album_artist: None,
        };
        let mut proposed =
            scrobble_scrubber::create_no_op_edit(&subject.representative_track(1, Some(at)));
        proposed.album_name = Some("Slowhand".into());
        let id = uuid::Uuid::from_u128(at.into());
        state
            .append_queue_events(&[
                QueueEvent {
                    id,
                    at,
                    kind: QueueEventKind::Created {
                        subject,
                        proposed: Box::new(proposed),
                        provider: MB.into(),
                        requires_approval: false,
                        rationale: None,
                        sources: sources(&[(EditField::Album, MB)]),
                    },
                },
                QueueEvent {
                    id,
                    at: at + 1,
                    kind: QueueEventKind::Completed,
                },
            ])
            .await
            .unwrap();
    }
    let rules = FixedProvider::new(
        RULES,
        vec![
            (
                key("Eric Clapton", cocaine, "Slowhand"),
                vec![
                    (EditField::Track, "Cocaine"),
                    (EditField::Album, "The Cream of Clapton"),
                ],
            ),
            (
                key("Eric Clapton", "Wonderful Tonight", "Slowhand"),
                vec![(EditField::Album, "The Cream of Clapton")],
            ),
        ],
    );
    Planner::new(store, state.clone())
        .with_provider(FixedProvider::new(MB, Vec::new()))
        .with_provider(rules)
        .plan(&ScrubFeed::StoreRange { range: None })
        .await
        .unwrap();

    let queued: Vec<_> = state
        .load_queue()
        .await
        .unwrap()
        .into_iter()
        .filter(|intent| intent.state != IntentState::Applied)
        .map(|intent| {
            (
                intent.subject.track,
                intent.proposed.track_name.unwrap(),
                intent.proposed.album_name.unwrap(),
                intent.sources,
            )
        })
        .collect();
    assert_eq!(
        queued,
        vec![(
            cocaine.to_string(),
            "Cocaine".to_string(),
            "Slowhand".to_string(),
            sources(&[(EditField::Track, RULES)]),
        )]
    );
}
