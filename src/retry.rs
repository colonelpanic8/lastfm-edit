use crate::{LastFmError, Result};
use std::future::Future;

/// Configuration for rate limit detection behavior
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    /// Whether to detect rate limits by HTTP status codes (429, 403)
    pub detect_by_status: bool,
    /// Whether to detect rate limits by response body patterns
    pub detect_by_patterns: bool,
    /// Patterns to look for in response bodies (used when detect_by_patterns is true)
    pub patterns: Vec<String>,
    /// Additional custom patterns to look for in response bodies
    pub custom_patterns: Vec<String>,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            detect_by_status: true,
            detect_by_patterns: true,
            patterns: vec![
                "you've tried to log in too many times".to_string(),
                "you're requesting too many pages".to_string(),
                "slow down".to_string(),
                "too fast".to_string(),
                "rate limit".to_string(),
                "throttled".to_string(),
                "temporarily blocked".to_string(),
                "temporarily restricted".to_string(),
                "captcha".to_string(),
                "verify you're human".to_string(),
                "prove you're not a robot".to_string(),
                "security check".to_string(),
                "service temporarily unavailable".to_string(),
                "quota exceeded".to_string(),
                "limit exceeded".to_string(),
                "daily limit".to_string(),
            ],
            custom_patterns: vec![],
        }
    }
}

impl RateLimitConfig {
    /// Create config with all detection disabled
    pub fn disabled() -> Self {
        Self {
            detect_by_status: false,
            detect_by_patterns: false,
            patterns: vec![],
            custom_patterns: vec![],
        }
    }

    /// Create config with only status code detection
    pub fn status_only() -> Self {
        Self {
            detect_by_status: true,
            detect_by_patterns: false,
            patterns: vec![],
            custom_patterns: vec![],
        }
    }

    /// Create config with only default pattern detection
    pub fn patterns_only() -> Self {
        Self {
            detect_by_status: false,
            detect_by_patterns: true,
            ..Default::default()
        }
    }

    /// Create config with custom patterns only (no default patterns)
    pub fn custom_patterns_only(patterns: Vec<String>) -> Self {
        Self {
            detect_by_status: false,
            detect_by_patterns: false,
            patterns: vec![],
            custom_patterns: patterns,
        }
    }

    /// Create config with both default and custom patterns
    pub fn with_custom_patterns(mut self, patterns: Vec<String>) -> Self {
        self.custom_patterns = patterns;
        self
    }

    /// Create config with custom patterns (replaces built-in patterns)
    pub fn with_patterns(mut self, patterns: Vec<String>) -> Self {
        self.patterns = patterns;
        self
    }
}

/// Unified configuration for retry behavior and rate limiting
#[derive(Debug, Clone, Default)]
pub struct ClientConfig {
    /// Retry configuration
    pub retry: RetryConfig,
    /// Rate limit detection configuration
    pub rate_limit: RateLimitConfig,
}

impl ClientConfig {
    /// Create a new config with default settings
    pub fn new() -> Self {
        Self::default()
    }

    /// Create config with retries disabled
    pub fn with_retries_disabled() -> Self {
        Self {
            retry: RetryConfig::disabled(),
            rate_limit: RateLimitConfig::default(),
        }
    }

    /// Create config with rate limit detection disabled
    pub fn with_rate_limiting_disabled() -> Self {
        Self {
            retry: RetryConfig::default(),
            rate_limit: RateLimitConfig::disabled(),
        }
    }

    /// Create config with both retries and rate limiting disabled
    pub fn minimal() -> Self {
        Self {
            retry: RetryConfig::disabled(),
            rate_limit: RateLimitConfig::disabled(),
        }
    }

    /// Set custom retry configuration
    pub fn with_retry_config(mut self, retry_config: RetryConfig) -> Self {
        self.retry = retry_config;
        self
    }

    /// Set custom rate limit configuration
    pub fn with_rate_limit_config(mut self, rate_limit_config: RateLimitConfig) -> Self {
        self.rate_limit = rate_limit_config;
        self
    }

    /// Set custom retry count
    pub fn with_max_retries(mut self, max_retries: u32) -> Self {
        self.retry.max_retries = max_retries;
        self.retry.enabled = max_retries > 0;
        self
    }

    /// Set custom retry delays
    pub fn with_retry_delays(mut self, base_delay: u64, max_delay: u64) -> Self {
        self.retry.base_delay = base_delay;
        self.retry.max_delay = max_delay;
        self
    }

    /// Add custom rate limit patterns
    pub fn with_custom_rate_limit_patterns(mut self, patterns: Vec<String>) -> Self {
        self.rate_limit.custom_patterns = patterns;
        self
    }

    /// Enable/disable HTTP status code rate limit detection
    pub fn with_status_detection(mut self, enabled: bool) -> Self {
        self.rate_limit.detect_by_status = enabled;
        self
    }

    /// Enable/disable response pattern rate limit detection
    pub fn with_pattern_detection(mut self, enabled: bool) -> Self {
        self.rate_limit.detect_by_patterns = enabled;
        self
    }
}

/// Configuration for retry behavior
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts (set to 0 to disable retries)
    pub max_retries: u32,
    /// Base delay for exponential backoff (in seconds)
    pub base_delay: u64,
    /// Maximum delay cap (in seconds)
    pub max_delay: u64,
    /// Whether retries are enabled at all
    pub enabled: bool,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay: 5,
            max_delay: 300, // 5 minutes
            enabled: true,
        }
    }
}

impl RetryConfig {
    /// Create a config with retries disabled
    pub fn disabled() -> Self {
        Self {
            max_retries: 0,
            base_delay: 5,
            max_delay: 300,
            enabled: false,
        }
    }

    /// Create a config with custom retry count
    pub fn with_retries(max_retries: u32) -> Self {
        Self {
            max_retries,
            enabled: max_retries > 0,
            ..Default::default()
        }
    }

    /// Create a config with custom delays
    pub fn with_delays(base_delay: u64, max_delay: u64) -> Self {
        Self {
            base_delay,
            max_delay,
            ..Default::default()
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
