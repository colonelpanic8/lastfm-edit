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
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[tokio::test]
    async fn cancellation_during_sync_prevents_later_phases_and_reenable_is_independent() {
        let run = RunGeneration::new();
        let reached_plan = AtomicBool::new(false);
        let cycle = async {
            run.run(std::future::pending::<Result<()>>()).await?;
            reached_plan.store(true, Ordering::Relaxed);
            Ok::<_, ScrubberError>(())
        };
        let (result, ()) = tokio::join!(cycle, async {
            tokio::task::yield_now().await;
            run.cancel();
        });
        assert!(matches!(result, Err(ScrubberError::Cancelled)));
        assert!(!reached_plan.load(Ordering::Relaxed));
        assert!(run
            .run::<()>(async { panic!("cancelled generation polled a phase") })
            .await
            .is_err());
        assert!(RunGeneration::new().run(async { Ok(()) }).await.is_ok());
    }
}
