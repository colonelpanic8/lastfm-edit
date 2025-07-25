use crate::{LastFmError, Result};
use std::future::Future;

/// Configuration for retry behavior
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts
    pub max_retries: u32,
    /// Base delay for exponential backoff (in seconds)
    pub base_delay: u64,
    /// Maximum delay cap (in seconds)
    pub max_delay: u64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay: 5,
            max_delay: 300, // 5 minutes
        }
    }
}

/// Result of a retry operation with context
#[derive(Debug)]
pub struct RetryResult<T> {
    /// The successful result
    pub result: T,
    /// Number of retry attempts made
    pub attempts_made: u32,
    /// Total time spent retrying (in seconds)
    pub total_retry_time: u64,
}

/// Execute an async operation with retry logic for rate limiting
///
/// This function handles the common pattern of retrying operations that may fail
/// due to rate limiting, with exponential backoff and configurable limits.
///
/// # Arguments
/// * `config` - Retry configuration
/// * `operation_name` - Name of the operation for logging
/// * `operation` - Async function that returns a Result
/// * `on_rate_limit` - Callback for rate limit events (delay in seconds)
///
/// # Returns
/// A `RetryResult` containing the successful result and retry statistics
pub async fn retry_with_backoff<T, F, Fut, OnRateLimit>(
    config: RetryConfig,
    operation_name: &str,
    mut operation: F,
    mut on_rate_limit: OnRateLimit,
) -> Result<RetryResult<T>>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
    OnRateLimit: FnMut(u64, &str),
{
    let mut retries = 0;
    let mut total_retry_time = 0;

    loop {
        match operation().await {
            Ok(result) => {
                return Ok(RetryResult {
                    result,
                    attempts_made: retries,
                    total_retry_time,
                });
            }
            Err(LastFmError::RateLimit { retry_after }) => {
                if retries >= config.max_retries {
                    log::warn!(
                        "Max retries ({}) exceeded for {} operation",
                        config.max_retries,
                        operation_name
                    );
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
                on_rate_limit(delay, operation_name);

                tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
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
    retry_with_backoff(config, operation_name, operation, |delay, op_name| {
        log::debug!("Rate limited during {op_name}: waiting {delay} seconds");
    })
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
}
