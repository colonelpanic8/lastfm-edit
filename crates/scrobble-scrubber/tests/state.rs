//! FsScrubberState round-trips: every state kind survives a fresh handle, torn JSONL
//! tails are tolerated, and provider coverage stores its rules hash.

use scrobble_scrubber::queue::{QueueEvent, QueueEventKind};
use scrobble_scrubber::rewrite::create_no_op_edit;
use scrobble_scrubber::{
    rules_hash, DismissedEntry, FsScrubberState, IntentState, ProviderCoverage, RewriteRule,
    RuleEvent, RuleEventKind, ScrubberState, SdRule, Subject,
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
        }])
        .await
        .unwrap();

    // Coverage with rules hash.
    let mut coverage = CoverageMap::new();
    coverage.insert(Segment::new(10, 20, 5));
    let provider_coverage = ProviderCoverage {
        coverage,
        rules_hash: Some(rules_hash(&rules)),
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
}
