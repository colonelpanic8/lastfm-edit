use super::*;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

#[tokio::test]
async fn test_successful_operation() {
    let config = RetryConfig {
        max_retries: 3,
        base_delay: 1,
        max_delay: 60,
        enabled: true,
    };

    let result = retry_operation(config, "test", || async { Ok::<i32, LastFmError>(42) }).await;

    assert!(result.is_ok());
    let retry_result = result.unwrap();
    assert_eq!(retry_result.result, 42);
    assert_eq!(retry_result.attempts_made, 0);
    assert_eq!(retry_result.total_retry_time, 0);
}

#[tokio::test]
async fn test_retry_on_rate_limit() {
    let config = RetryConfig {
        max_retries: 2,
        base_delay: 1,
        max_delay: 60,
        enabled: true,
    };

    let call_count = Arc::new(AtomicU32::new(0));
    let call_count_clone = call_count.clone();

    let result = retry_operation(config, "test", move || {
        let count = call_count_clone.fetch_add(1, Ordering::SeqCst);
        async move {
            if count < 2 {
                Err(LastFmError::RateLimit { retry_after: 1 })
            } else {
                Ok::<i32, LastFmError>(42)
            }
        }
    })
    .await;

    assert!(result.is_ok());
    let retry_result = result.unwrap();
    assert_eq!(retry_result.result, 42);
    assert_eq!(retry_result.attempts_made, 2);
    assert!(retry_result.total_retry_time >= 2); // At least 2 seconds of delay
}

#[tokio::test]
async fn test_max_retries_exceeded() {
    let config = RetryConfig {
        max_retries: 1,
        base_delay: 1,
        max_delay: 60,
        enabled: true,
    };

    let result = retry_operation(config, "test", || async {
        Err::<i32, LastFmError>(LastFmError::RateLimit { retry_after: 1 })
    })
    .await;

    assert!(result.is_err());
    match result.unwrap_err() {
        LastFmError::RateLimit { .. } => {} // Expected
        other => panic!("Expected rate limit error, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_retries_disabled() {
    let config = RetryConfig::disabled();

    let result = retry_operation(config, "test", || async {
        Err::<i32, LastFmError>(LastFmError::RateLimit { retry_after: 1 })
    })
    .await;

    assert!(result.is_err());
    match result.unwrap_err() {
        LastFmError::RateLimit { .. } => {} // Expected - should fail immediately
        other => panic!("Expected rate limit error, got: {other:?}"),
    }
}
