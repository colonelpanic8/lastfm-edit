//! Cancellation scope for one enabled continuous run. Cancelled runs cannot be reset.

use crate::{Result, ScrubberError};
use lastfm_edit::CancellationState;

#[derive(Clone, Debug, Default)]
pub struct RunGeneration {
    cancel: CancellationState,
}

impl RunGeneration {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// Run one phase, cancelling it if this generation is disabled.
    pub async fn run<T>(&self, phase: impl std::future::Future<Output = Result<T>>) -> Result<T> {
        let mut changed = self.cancel.subscribe();
        if *changed.borrow() {
            return Err(ScrubberError::Cancelled);
        }
        tokio::select! {
            biased;
            _ = changed.changed() => Err(ScrubberError::Cancelled),
            result = phase => result,
        }
    }
}

#[cfg(test)]
#[path = "run_tests.rs"]
mod tests;
