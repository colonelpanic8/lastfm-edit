use crate::types::LastFmError;
use crate::Result;
use std::time::Duration;
use tokio::sync::watch;

/// Cooperative cancellation support for long-running operations.
///
/// This is intentionally simple:
/// - `cancel()` flips a boolean and wakes sleepers.
/// - `reset()` clears the flag so future operations can run again.
/// - Long sleeps select on either the timer or cancellation.
#[derive(Clone, Debug)]
pub struct CancellationState {
    tx: watch::Sender<bool>,
}

impl Default for CancellationState {
    fn default() -> Self {
        Self::new()
    }
}

impl CancellationState {
    pub fn new() -> Self {
        let (tx, _rx) = watch::channel(false);
        Self { tx }
    }

    pub fn cancel(&self) {
        self.tx.send_replace(true);
    }

    pub fn reset(&self) {
        self.tx.send_replace(false);
    }

    pub fn is_cancelled(&self) -> bool {
        *self.tx.borrow()
    }

    pub fn subscribe(&self) -> watch::Receiver<bool> {
        self.tx.subscribe()
    }
}

fn cancelled_error() -> LastFmError {
    LastFmError::Io(std::io::Error::new(
        std::io::ErrorKind::Interrupted,
        "cancelled",
    ))
}

pub async fn sleep_with_cancel(
    mut cancel_rx: watch::Receiver<bool>,
    duration: Duration,
) -> Result<()> {
    if *cancel_rx.borrow() {
        return Err(cancelled_error());
    }

    let sleeper = tokio::time::sleep(duration);
    tokio::pin!(sleeper);
    tokio::select! {
        _ = &mut sleeper => Ok(()),
        _ = async {
            loop {
                if cancel_rx.changed().await.is_err() {
                    // Sender dropped; treat as non-cancelable.
                    std::future::pending::<()>().await;
                }
                if *cancel_rx.borrow() {
                    break;
                }
            }
        } => Err(cancelled_error()),
    }
}

#[cfg(test)]
mod tests {
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
}
