//! FsScrubberState round-trips: every state kind survives a fresh handle, torn JSONL
//! tails are tolerated, and provider coverage stores its rules hash.

use scrobble_scrubber::queue::{QueueEvent, QueueEventKind};
use scrobble_scrubber::rewrite::create_no_op_edit;
use scrobble_scrubber::{
    reinstate_intent, reject_intent, rules_hash, DismissedEntry, FsScrubberState, IntentState,
    MemoryScrubberState, ProviderCoverage, RewriteRule, RuleEvent, RuleEventKind, ScrubberState,
    SdRule, Subject,
};
use scrobble_store::{CoverageMap, Segment};
use uuid::Uuid;

fn subject() -> Subject {
    Subject {
        artist: "A".into(),
        track: "x".into(),
        album: Some("Album".into()),
        album_artist: None,
    }
}

#[tokio::test]
async fn full_round_trip_through_fresh_handles() {
    let dir = tempfile::tempdir().unwrap();
    let state = FsScrubberState::open(dir.path()).unwrap();

    // Rules.
    let rules = vec![RewriteRule::new()
        .with_name("strip remaster")
        .with_track_name(SdRule::new(" - Remaster", ""))];
    state.save_rules(&rules).await.unwrap();

    // Queue.
    let intent_id = Uuid::new_v4();
    let track = subject().representative_track(1, Some(100));
    state
        .append_queue_events(&[
            QueueEvent {
                id: intent_id,
                at: 1,
                kind: QueueEventKind::Created {
                    subject: subject(),
                    proposed: Box::new(create_no_op_edit(&track)),
                    provider: "rewrite_rules".into(),
                    requires_approval: true,
                    rationale: None,
                },
            },
            QueueEvent {
                id: intent_id,
                at: 2,
                kind: QueueEventKind::Approved,
            },
        ])
        .await
        .unwrap();

    // Pending rules.
    let rule_id = Uuid::new_v4();
    state
        .append_rule_events(&[RuleEvent {
            id: rule_id,
            at: 3,
            kind: RuleEventKind::Created {
                rule: Box::new(RewriteRule::new().with_name("proposed")),
                motivation: "seen repeatedly".into(),
                provider: "openai".into(),
                example: Some(subject()),
            },
        }])
        .await
        .unwrap();

    // Dismissals.
    state
        .append_dismissed(&[DismissedEntry {
            subject: subject(),
            at: 4,
            reason: "manual".into(),
            active: true,
        }])
        .await
        .unwrap();

    // Coverage with rules hash.
    let mut coverage = CoverageMap::new();
    coverage.insert(Segment::new(10, 20, 5));
    let provider_coverage = ProviderCoverage {
        coverage,
        rules_hash: Some(rules_hash(&rules)),
        ..ProviderCoverage::default()
    };
    state
        .save_provider_coverage("rewrite_rules", &provider_coverage)
        .await
        .unwrap();

    // Fresh handle sees identical state.
    let reopened = FsScrubberState::open(dir.path()).unwrap();
    assert_eq!(reopened.load_rules().await.unwrap(), rules);

    let queue = reopened.load_queue().await.unwrap();
    assert_eq!(queue.len(), 1);
    assert_eq!(queue[0].id, intent_id);
    assert_eq!(queue[0].state, IntentState::Ready); // approved
    assert_eq!(queue[0].subject, subject());

    let pending = reopened.load_pending_rules().await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].provider, "openai");

    let dismissed = reopened.load_dismissed().await.unwrap();
    assert!(dismissed.contains(&subject()));

    let loaded_coverage = reopened
        .load_provider_coverage("rewrite_rules")
        .await
        .unwrap();
    assert_eq!(loaded_coverage, provider_coverage);
    // Unknown provider: empty default.
    let empty = reopened.load_provider_coverage("openai").await.unwrap();
    assert!(empty.coverage.is_empty());
    assert!(empty.rules_hash.is_none());

    // The state dir carries the merge=union gitattributes like the store.
    assert!(dir.path().join(".gitattributes").exists());
}

#[tokio::test]
async fn reinstate_reopens_intent_and_lifts_dismissal() {
    let state = MemoryScrubberState::new();
    let intent_id = Uuid::new_v4();
    let track = subject().representative_track(1, Some(100));
    state
        .append_queue_events(&[QueueEvent {
            id: intent_id,
            at: 1,
            kind: QueueEventKind::Created {
                subject: subject(),
                proposed: Box::new(create_no_op_edit(&track)),
                provider: "rewrite_rules".into(),
                requires_approval: true,
                rationale: None,
            },
        }])
        .await
        .unwrap();

    // Rejecting with dismiss=true dismisses the subject.
    reject_intent(&state, intent_id, true).await.unwrap();
    assert_eq!(
        state.load_queue().await.unwrap()[0].state,
        IntentState::Rejected { dismissed: true }
    );
    assert!(state.load_dismissed().await.unwrap().contains(&subject()));

    // Reinstating reopens the intent and lifts the dismissal.
    reinstate_intent(&state, intent_id).await.unwrap();
    assert_eq!(
        state.load_queue().await.unwrap()[0].state,
        IntentState::AwaitingApproval
    );
    assert!(!state.load_dismissed().await.unwrap().contains(&subject()));
}

#[tokio::test]
async fn torn_queue_tail_is_tolerated() {
    let dir = tempfile::tempdir().unwrap();
    let state = FsScrubberState::open(dir.path()).unwrap();
    let track = subject().representative_track(1, Some(100));
    state
        .append_queue_events(&[QueueEvent {
            id: Uuid::new_v4(),
            at: 1,
            kind: QueueEventKind::Created {
                subject: subject(),
                proposed: Box::new(create_no_op_edit(&track)),
                provider: "rewrite_rules".into(),
                requires_approval: false,
                rationale: None,
            },
        }])
        .await
        .unwrap();

    // Simulate a crash mid-append.
    let path = dir.path().join("queue.jsonl");
    let mut content = std::fs::read_to_string(&path).unwrap();
    content.push_str("{\"id\":\"trunc");
    std::fs::write(&path, content).unwrap();

    let queue = FsScrubberState::open(dir.path())
        .unwrap()
        .load_queue()
        .await
        .unwrap();
    assert_eq!(queue.len(), 1);
    state
        .append_queue_events(&[QueueEvent {
            id: queue[0].id,
            at: 2,
            kind: QueueEventKind::Completed,
        }])
        .await
        .unwrap();
    let reopened = FsScrubberState::open(dir.path()).unwrap();
    assert_eq!(
        reopened.load_queue().await.unwrap()[0].state,
        IntentState::Applied
    );
}

#[tokio::test]
async fn resolution_log_cut_inside_a_multibyte_character_keeps_earlier_records() {
    use scrobble_scrubber::identity::{ResolutionEvent, ResolutionEventKind};
    use scrobble_scrubber::subject::ScrobbleKey;

    let dir = tempfile::tempdir().unwrap();
    let state = FsScrubberState::open(dir.path()).unwrap();
    let not_found = |track: &str| ResolutionEvent {
        key: ScrobbleKey {
            artist: "Béla Fleck and the Flecktones".into(),
            track: track.into(),
            album: Some("Flight of the Cosmic Hippo".into()),
        },
        at: 1,
        resolver: 1,
        kind: ResolutionEventKind::NotFound {
            queries: Vec::new(),
        },
    };
    state
        .append_resolution_events(&[not_found("Blu-Bop"), not_found("Star of the County Down")])
        .await
        .unwrap();

    // Simulate a crash mid-append that cut "é" after its first byte.
    let path = dir.path().join("resolutions.jsonl");
    let mut content = std::fs::read(&path).unwrap();
    let torn = serde_json::to_vec(&not_found("Turtle Rock")).unwrap();
    let cut = torn.windows(2).position(|w| w == "é".as_bytes()).unwrap() + 1;
    content.extend_from_slice(&torn[..cut]);
    std::fs::write(&path, content).unwrap();

    let reopened = FsScrubberState::open(dir.path()).unwrap();
    let tracks = |events: Vec<ResolutionEvent>| -> Vec<String> {
        events.into_iter().map(|event| event.key.track).collect()
    };
    assert_eq!(
        tracks(reopened.load_resolution_events().await.unwrap()),
        vec!["Blu-Bop", "Star of the County Down"]
    );

    reopened
        .append_resolution_events(&[not_found("Turtle Rock")])
        .await
        .unwrap();
    assert_eq!(
        tracks(reopened.load_resolution_events().await.unwrap()),
        vec!["Blu-Bop", "Star of the County Down", "Turtle Rock"]
    );
}
