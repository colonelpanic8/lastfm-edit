use super::*;

#[test]
fn cancellation_and_reset_survive_without_subscribers() {
    let cancel = CancellationState::new();
    cancel.cancel();
    assert!(cancel.is_cancelled());
    assert!(*cancel.subscribe().borrow());
    cancel.reset();
    assert!(!cancel.is_cancelled());
    let receiver = cancel.subscribe();
    drop(receiver);
    cancel.cancel();
    assert!(*cancel.subscribe().borrow());
}

#[tokio::test]
async fn cancellation_wakes_an_active_sleep() {
    let cancel = CancellationState::new();
    let sleep = sleep_with_cancel(cancel.subscribe(), Duration::from_secs(3600));
    let (result, ()) = tokio::join!(sleep, async {
        tokio::task::yield_now().await;
        cancel.cancel();
    });
    assert!(
        matches!(result, Err(LastFmError::Io(err)) if err.kind() == std::io::ErrorKind::Interrupted)
    );
}

#[tokio::test]
async fn dropping_sender_does_not_cancel_the_sleep() {
    let cancel = CancellationState::new();
    let receiver = cancel.subscribe();
    drop(cancel);
    assert!(tokio::time::timeout(
        Duration::from_millis(10),
        sleep_with_cancel(receiver, Duration::from_secs(3600)),
    )
    .await
    .is_err());
}
