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
        let _ = self.tx.send(true);
    }

    pub fn reset(&self) {
        let _ = self.tx.send(false);
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
                    break;
                }
                if *cancel_rx.borrow() {
                    break;
                }
            }
        } => Err(cancelled_error()),
    }
}
