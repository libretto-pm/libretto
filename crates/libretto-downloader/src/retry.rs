//! Retry logic with exponential backoff and mirror fallback.
//!
//! Provides robust retry strategies for network operations.

use crate::error::{DownloadError, Result};
use backon::{ExponentialBuilder, Retryable};
use std::future::Future;
use std::time::Duration;
use tracing::{debug, warn};

/// Retry configuration.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts.
    pub max_retries: u32,
    /// Base delay for exponential backoff.
    pub base_delay: Duration,
    /// Maximum delay between retries.
    pub max_delay: Duration,
    /// Jitter factor (0.0 to 1.0).
    pub jitter: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(30),
            jitter: 0.1,
        }
    }
}

impl RetryConfig {
    /// Create a new retry configuration.
    #[must_use]
    pub fn new(max_retries: u32) -> Self {
        Self {
            max_retries,
            ..Default::default()
        }
    }

    /// Build a backoff strategy.
    fn build_backoff(&self) -> ExponentialBuilder {
        ExponentialBuilder::default()
            .with_min_delay(self.base_delay)
            .with_max_delay(self.max_delay)
            .with_max_times(self.max_retries as usize)
            .with_jitter()
    }
}

/// Execute a fallible operation with retry and exponential backoff.
///
/// # Errors
/// Returns the last error after all retries are exhausted.
pub async fn with_retry<F, Fut, T>(config: &RetryConfig, operation: F) -> Result<T>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let backoff = config.build_backoff();

    let result = operation
        .retry(backoff)
        .when(|e: &DownloadError| {
            let should_retry = e.is_retryable();
            if should_retry {
                debug!(error = %e, "retrying after error");
            }
            should_retry
        })
        .notify(|e: &DownloadError, dur: Duration| {
            warn!(error = %e, delay = ?dur, "operation failed, retrying");
        })
        .await;

    result
}

/// Execute an operation with fallback to mirror URLs.
///
/// Tries the primary URL first, then falls back to mirrors on failure.
///
/// # Errors
/// Returns `AllMirrorsFailed` error if all URLs fail.
pub async fn with_mirrors<F, Fut, T>(
    primary: &str,
    mirrors: &[String],
    config: &RetryConfig,
    operation: F,
) -> Result<T>
where
    F: Fn(&str) -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let all_urls: Vec<&str> = std::iter::once(primary)
        .chain(mirrors.iter().map(|s| s.as_str()))
        .collect();

    let mut errors = Vec::new();

    for (i, url) in all_urls.iter().enumerate() {
        debug!(url, attempt = i + 1, total = all_urls.len(), "trying URL");

        match with_retry(config, || operation(url)).await {
            Ok(result) => return Ok(result),
            Err(e) => {
                warn!(url, error = %e, "URL failed");
                errors.push(format!("{url}: {e}"));
            }
        }
    }

    Err(DownloadError::AllMirrorsFailed {
        package: primary.to_string(),
        errors,
    })
}

/// Circuit breaker for preventing cascading failures.
#[derive(Debug)]
pub struct CircuitBreaker {
    /// Number of consecutive failures.
    failures: std::sync::atomic::AtomicU32,
    /// Failure threshold before opening circuit.
    threshold: u32,
    /// Time to wait before trying again after circuit opens.
    reset_timeout: Duration,
    /// When the circuit was last opened.
    last_failure: parking_lot::Mutex<Option<std::time::Instant>>,
}

impl CircuitBreaker {
    /// Create a new circuit breaker.
    #[must_use]
    pub fn new(threshold: u32, reset_timeout: Duration) -> Self {
        Self {
            failures: std::sync::atomic::AtomicU32::new(0),
            threshold,
            reset_timeout,
            last_failure: parking_lot::Mutex::new(None),
        }
    }

    /// Check if the circuit is open (failing).
    #[must_use]
    pub fn is_open(&self) -> bool {
        let failures = self.failures.load(std::sync::atomic::Ordering::Relaxed);
        if failures < self.threshold {
            return false;
        }

        // Check if we should try again
        let last = self.last_failure.lock();
        if let Some(instant) = *last {
            if instant.elapsed() < self.reset_timeout {
                return true;
            }
        }

        false
    }

    /// Record a successful operation.
    pub fn record_success(&self) {
        self.failures.store(0, std::sync::atomic::Ordering::Relaxed);
        *self.last_failure.lock() = None;
    }

    /// Record a failed operation.
    pub fn record_failure(&self) {
        let prev = self
            .failures
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if prev + 1 >= self.threshold {
            *self.last_failure.lock() = Some(std::time::Instant::now());
        }
    }

    /// Execute an operation with circuit breaker protection.
    ///
    /// # Errors
    /// Returns error if circuit is open or operation fails.
    pub async fn execute<F, Fut, T>(&self, operation: F) -> Result<T>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        if self.is_open() {
            return Err(DownloadError::network("circuit breaker open"));
        }

        match operation().await {
            Ok(result) => {
                self.record_success();
                Ok(result)
            }
            Err(e) => {
                self.record_failure();
                Err(e)
            }
        }
    }
}

impl Default for CircuitBreaker {
    fn default() -> Self {
        Self::new(5, Duration::from_secs(30))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[test]
    fn retry_config_default() {
        let config = RetryConfig::default();
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.base_delay, Duration::from_millis(500));
    }

    #[tokio::test]
    async fn retry_success() {
        let config = RetryConfig::new(3);
        let result: Result<i32> = with_retry(&config, || async { Ok(42) }).await;
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn retry_eventual_success() {
        let config = RetryConfig {
            max_retries: 3,
            base_delay: Duration::from_millis(10),
            max_delay: Duration::from_millis(100),
            jitter: 0.0,
        };

        let attempts = AtomicU32::new(0);

        let result: Result<i32> = with_retry(&config, || {
            let attempt = attempts.fetch_add(1, Ordering::Relaxed);
            async move {
                if attempt < 2 {
                    Err(DownloadError::Connection("test".into()))
                } else {
                    Ok(42)
                }
            }
        })
        .await;

        assert_eq!(result.unwrap(), 42);
        assert_eq!(attempts.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn circuit_breaker_closed() {
        let cb = CircuitBreaker::new(3, Duration::from_secs(10));
        assert!(!cb.is_open());

        cb.record_failure();
        assert!(!cb.is_open());

        cb.record_failure();
        assert!(!cb.is_open());

        cb.record_failure();
        assert!(cb.is_open());
    }

    #[test]
    fn circuit_breaker_reset() {
        let cb = CircuitBreaker::new(3, Duration::from_secs(10));

        for _ in 0..3 {
            cb.record_failure();
        }
        assert!(cb.is_open());

        cb.record_success();
        assert!(!cb.is_open());
    }
}
