use super::*;

#[tokio::test]
async fn events_reach_subscribers() {
    let bus = SyncEventBus::new();
    let mut rx = bus.subscribe();
    bus.emit(SyncEvent::SyncStarted {
        mode: SyncMode::ExtendToPresent,
    });
    assert_eq!(
        rx.recv().await.unwrap(),
        SyncEvent::SyncStarted {
            mode: SyncMode::ExtendToPresent
        }
    );
}

#[test]
fn emit_without_subscribers_is_fine() {
    let bus = SyncEventBus::new();
    bus.emit(SyncEvent::SyncResumed);
}

#[test]
fn events_serialize() {
    let event = SyncEvent::SyncPaused {
        reason: PauseReason::RateLimited {
            until_estimate: Some(1_700_000_000),
        },
    };
    let json = serde_json::to_string(&event).unwrap();
    let back: SyncEvent = serde_json::from_str(&json).unwrap();
    assert_eq!(back, event);
}
