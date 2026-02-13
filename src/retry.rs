use crate::types::{LastFmError, RetryConfig, RetryResult};
use crate::Result;
use std::future::Future;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::cancel;

/// Execute an async operation with retry logic for rate limiting
///
/// This function handles the common pattern of retrying operations that may fail
/// due to rate limiting, with exponential backoff and configurable limits.
///
/// # Arguments
/// * `config` - Retry configuration
/// * `operation_name` - Name of the operation for logging
/// * `operation` - Async function that returns a Result
/// * `on_rate_limit` - Callback for rate limit events (delay in seconds, timestamp)
/// * `on_rate_limit_end` - Optional callback for when rate limiting ends (total duration in seconds)
///
/// # Returns
/// A `RetryResult` containing the successful result and retry statistics
pub async fn retry_with_backoff<T, F, Fut, OnRateLimit, OnRateLimitEnd>(
    config: RetryConfig,
    operation_name: &str,
    operation: F,
    on_rate_limit: OnRateLimit,
    on_rate_limit_end: OnRateLimitEnd,
) -> Result<RetryResult<T>>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
    OnRateLimit: FnMut(u64, u64, &str),
    OnRateLimitEnd: FnMut(u64, &str),
{
    retry_with_backoff_cancelable(
        config,
        operation_name,
        operation,
        on_rate_limit,
        on_rate_limit_end,
        None,
    )
    .await
}

/// Like [`retry_with_backoff`], but allows callers to cooperatively cancel during backoff sleeps.
///
/// Cancellation returns `LastFmError::Io(ErrorKind::Interrupted)` so downstream crates do not need
/// to handle a new `LastFmError` variant.
pub async fn retry_with_backoff_cancelable<T, F, Fut, OnRateLimit, OnRateLimitEnd>(
    config: RetryConfig,
    operation_name: &str,
    mut operation: F,
    mut on_rate_limit: OnRateLimit,
    mut on_rate_limit_end: OnRateLimitEnd,
    cancel_rx: Option<tokio::sync::watch::Receiver<bool>>,
) -> Result<RetryResult<T>>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
    OnRateLimit: FnMut(u64, u64, &str),
    OnRateLimitEnd: FnMut(u64, &str),
{
    let mut retries = 0;
    let mut total_retry_time = 0;
    let mut rate_limit_start_time: Option<Instant> = None;

    loop {
        match operation().await {
            Ok(result) => {
                // If we had rate limiting and now succeeded, emit rate limit end event
                if let Some(start_time) = rate_limit_start_time {
                    let total_duration = start_time.elapsed().as_secs();
                    on_rate_limit_end(total_duration, operation_name);
                }

                return Ok(RetryResult {
                    result,
                    attempts_made: retries,
                    total_retry_time,
                });
            }
            Err(LastFmError::RateLimit { retry_after }) => {
                // Track when rate limiting first occurs
                if rate_limit_start_time.is_none() {
                    rate_limit_start_time = Some(Instant::now());
                }

                if !config.enabled || retries >= config.max_retries {
                    if !config.enabled {
                        log::debug!("Retries disabled for {operation_name} operation");
                    } else {
                        log::warn!(
                            "Max retries ({}) exceeded for {operation_name} operation",
                            config.max_retries
                        );
                    }
                    return Err(LastFmError::RateLimit { retry_after });
                }

                // Calculate delay with exponential backoff
                let base_backoff = config.base_delay * 2_u64.pow(retries);
                let delay = std::cmp::min(
                    std::cmp::min(retry_after + base_backoff, config.max_delay),
                    retry_after + (retries as u64 * 30), // Legacy backoff for compatibility
                );

                log::info!(
                    "{} rate limited. Waiting {} seconds before retry {} of {}",
                    operation_name,
                    delay,
                    retries + 1,
                    config.max_retries
                );

                // Notify caller about rate limit
                let timestamp = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                on_rate_limit(delay, timestamp, operation_name);

                if let Some(rx) = cancel_rx.clone() {
                    cancel::sleep_with_cancel(rx, std::time::Duration::from_secs(delay)).await?;
                } else {
                    tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
                }
                retries += 1;
                total_retry_time += delay;
            }
            Err(other_error) => {
                return Err(other_error);
            }
        }
    }
}

/// Simplified retry function for operations that don't need custom rate limit handling
pub async fn retry_operation<T, F, Fut>(
    config: RetryConfig,
    operation_name: &str,
    operation: F,
) -> Result<RetryResult<T>>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    retry_with_backoff(
        config,
        operation_name,
        operation,
        |delay, timestamp, op_name| {
            log::debug!(
                "Rate limited during {op_name}: waiting {delay} seconds (at timestamp {timestamp})"
            );
        },
        |duration, op_name| {
            log::debug!("Rate limiting ended for {op_name} after {duration} seconds");
        },
    )
    .await
}

#[cfg(test)]
mod tests {
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
}
