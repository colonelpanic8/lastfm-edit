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
