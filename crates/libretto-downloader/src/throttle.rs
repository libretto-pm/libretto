//! Bandwidth throttling for downloads.
//!
//! Provides rate limiting using a token bucket algorithm.

use governor::{
    clock::DefaultClock,
    middleware::NoOpMiddleware,
    state::{InMemoryState, NotKeyed},
    Quota, RateLimiter,
};
use std::num::NonZeroU32;
use std::sync::Arc;

/// Bandwidth throttler using token bucket algorithm.
#[derive(Clone)]
pub struct BandwidthThrottler {
    limiter: Option<Arc<RateLimiter<NotKeyed, InMemoryState, DefaultClock, NoOpMiddleware>>>,
    chunk_size: u32,
}

impl std::fmt::Debug for BandwidthThrottler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BandwidthThrottler")
            .field("enabled", &self.limiter.is_some())
            .field("chunk_size", &self.chunk_size)
            .finish()
    }
}

impl BandwidthThrottler {
    /// Create a new throttler with the given bandwidth limit in bytes per second.
    ///
    /// If `bytes_per_second` is `None`, no throttling is applied.
    #[must_use]
    pub fn new(bytes_per_second: Option<u64>) -> Self {
        let limiter = bytes_per_second.and_then(|bps| {
            if bps == 0 {
                return None;
            }

            // Use 1KB chunks for granular control
            let chunk_size = 1024u32;
            let chunks_per_second = (bps / u64::from(chunk_size)).max(1);

            // Clamp to u32::MAX for the quota - safe because we clamped above
            #[allow(clippy::cast_possible_truncation)]
            let cps = chunks_per_second.min(u64::from(u32::MAX)) as u32;

            NonZeroU32::new(cps).map(|nz| {
                let quota = Quota::per_second(nz);
                Arc::new(RateLimiter::direct(quota))
            })
        });

        Self {
            limiter,
            chunk_size: 1024,
        }
    }

    /// Create an unlimited throttler (no rate limiting).
    #[must_use]
    pub const fn unlimited() -> Self {
        Self {
            limiter: None,
            chunk_size: 1024,
        }
    }

    /// Check if throttling is enabled.
    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.limiter.is_some()
    }

    /// Wait for permission to transfer `bytes` amount of data.
    ///
    /// This method blocks (asynchronously) until enough tokens are available.
    pub async fn acquire(&self, bytes: usize) {
        if let Some(ref limiter) = self.limiter {
            // Calculate how many chunks we need permission for
            // Saturate to u32::MAX for very large transfers
            let bytes_u32 = u32::try_from(bytes).unwrap_or(u32::MAX);
            let chunks = (bytes_u32 / self.chunk_size).max(1);

            // Acquire permission for each chunk
            for _ in 0..chunks {
                limiter.until_ready().await;
            }
        }
    }

    /// Try to acquire permission without waiting.
    ///
    /// Returns `true` if permission was granted immediately.
    #[must_use]
    pub fn try_acquire(&self, bytes: usize) -> bool {
        if let Some(ref limiter) = self.limiter {
            let bytes_u32 = u32::try_from(bytes).unwrap_or(u32::MAX);
            let chunks = (bytes_u32 / self.chunk_size).max(1);
            for _ in 0..chunks {
                if limiter.check().is_err() {
                    return false;
                }
            }
        }
        true
    }
}

impl Default for BandwidthThrottler {
    fn default() -> Self {
        Self::unlimited()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlimited_throttler() {
        let throttler = BandwidthThrottler::unlimited();
        assert!(!throttler.is_enabled());
        assert!(throttler.try_acquire(1_000_000));
    }

    #[test]
    fn limited_throttler() {
        let throttler = BandwidthThrottler::new(Some(1024 * 1024)); // 1MB/s
        assert!(throttler.is_enabled());
    }

    #[test]
    fn zero_limit_is_unlimited() {
        let throttler = BandwidthThrottler::new(Some(0));
        assert!(!throttler.is_enabled());
    }

    #[tokio::test]
    async fn acquire_works() {
        let throttler = BandwidthThrottler::new(Some(1024 * 1024)); // 1MB/s
        throttler.acquire(1024).await;
        // Should complete without issue
    }
}
