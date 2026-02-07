//! HTTP client with connection pooling, rate limiting, and retry logic.

use crate::error::{RepositoryError, Result};
use backon::{ExponentialBuilder, Retryable};
use dashmap::DashMap;
use governor::{
    Quota, RateLimiter,
    clock::DefaultClock,
    state::{InMemoryState, NotKeyed},
};
use parking_lot::RwLock;
use reqwest::{Client, Response, StatusCode, header};
use std::num::NonZeroU32;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tracing::{debug, warn};
use url::Url;

/// HTTP client configuration.
#[derive(Debug, Clone)]
pub struct HttpClientConfig {
    /// Request timeout.
    pub timeout: Duration,
    /// Connection timeout.
    pub connect_timeout: Duration,
    /// Maximum retries.
    pub max_retries: usize,
    /// Initial retry delay.
    pub retry_delay: Duration,
    /// Maximum retry delay.
    pub max_retry_delay: Duration,
    /// Requests per second per host.
    pub rate_limit_per_host: u32,
    /// Maximum concurrent requests per host.
    pub max_concurrent_per_host: usize,
    /// User agent string.
    pub user_agent: String,
}

impl Default for HttpClientConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            connect_timeout: Duration::from_secs(10),
            max_retries: 3,
            retry_delay: Duration::from_millis(100),
            max_retry_delay: Duration::from_secs(10),
            rate_limit_per_host: 20, // 20 req/s per host
            max_concurrent_per_host: 10,
            user_agent: format!(
                "Libretto/{} (mailto=libretto@example.com)",
                env!("CARGO_PKG_VERSION")
            ),
        }
    }
}

/// `ETag` and Last-Modified cache entry.
#[derive(Debug, Clone)]
pub struct CacheMetadata {
    /// `ETag` header value.
    pub etag: Option<String>,
    /// Last-Modified header value.
    pub last_modified: Option<String>,
    /// When the metadata was cached.
    pub cached_at: Instant,
}

/// HTTP client statistics.
#[derive(Debug, Default)]
pub struct HttpClientStats {
    /// Total requests made.
    pub requests: AtomicU64,
    /// Successful requests (2xx).
    pub successes: AtomicU64,
    /// Client errors (4xx).
    pub client_errors: AtomicU64,
    /// Server errors (5xx).
    pub server_errors: AtomicU64,
    /// Retries attempted.
    pub retries: AtomicU64,
    /// Rate limit hits.
    pub rate_limited: AtomicU64,
    /// Cache hits (304 Not Modified).
    pub cache_hits: AtomicU64,
    /// Total bytes received.
    pub bytes_received: AtomicU64,
    /// Total time spent on requests.
    total_request_time_ms: AtomicU64,
}

impl HttpClientStats {
    /// Create new stats tracker.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Get average request time in milliseconds.
    #[must_use]
    pub fn avg_request_time_ms(&self) -> f64 {
        let total = self.total_request_time_ms.load(Ordering::Relaxed);
        let count = self.requests.load(Ordering::Relaxed);
        if count == 0 {
            0.0
        } else {
            total as f64 / count as f64
        }
    }

    /// Get success rate as a percentage.
    #[must_use]
    pub fn success_rate(&self) -> f64 {
        let total = self.requests.load(Ordering::Relaxed);
        let success = self.successes.load(Ordering::Relaxed);
        if total == 0 {
            100.0
        } else {
            (success as f64 / total as f64) * 100.0
        }
    }

    fn record_request(&self, duration: Duration) {
        self.requests.fetch_add(1, Ordering::Relaxed);
        self.total_request_time_ms
            .fetch_add(duration.as_millis() as u64, Ordering::Relaxed);
    }
}

/// HTTP response with metadata.
#[derive(Debug)]
pub struct HttpResponse {
    /// Response body bytes.
    pub body: bytes::Bytes,
    /// HTTP status code.
    pub status: StatusCode,
    /// `ETag` header if present.
    pub etag: Option<String>,
    /// Last-Modified header if present.
    pub last_modified: Option<String>,
    /// Cache-Control max-age if present.
    pub max_age: Option<Duration>,
    /// Whether this was a cache hit (304).
    pub was_cached: bool,
}

type HostRateLimiter = RateLimiter<NotKeyed, InMemoryState, DefaultClock>;

/// High-performance HTTP client with per-host rate limiting and retry logic.
#[derive(Debug)]
pub struct HttpClient {
    /// Underlying reqwest client.
    client: Client,
    /// Configuration.
    config: HttpClientConfig,
    /// Per-host rate limiters.
    rate_limiters: DashMap<String, Arc<HostRateLimiter>>,
    /// ETag/Last-Modified cache.
    cache_metadata: DashMap<String, CacheMetadata>,
    /// Statistics.
    stats: Arc<HttpClientStats>,
    /// Default headers.
    default_headers: RwLock<header::HeaderMap>,
}

impl HttpClient {
    /// Create a new HTTP client with default configuration.
    ///
    /// # Errors
    /// Returns error if client cannot be created.
    pub fn new() -> Result<Self> {
        Self::with_config(HttpClientConfig::default())
    }

    /// Create a new HTTP client with custom configuration.
    ///
    /// # Errors
    /// Returns error if client cannot be created.
    pub fn with_config(config: HttpClientConfig) -> Result<Self> {
        let mut headers = header::HeaderMap::new();
        headers.insert(
            header::USER_AGENT,
            config
                .user_agent
                .parse()
                .map_err(|_| RepositoryError::InvalidConfig {
                    message: "Invalid user agent".into(),
                })?,
        );
        headers.insert(
            header::ACCEPT,
            "application/json"
                .parse()
                .map_err(|_| RepositoryError::InvalidConfig {
                    message: "Invalid accept header".into(),
                })?,
        );
        headers.insert(
            header::ACCEPT_ENCODING,
            "gzip, br, deflate, zstd"
                .parse()
                .map_err(|_| RepositoryError::InvalidConfig {
                    message: "Invalid accept-encoding header".into(),
                })?,
        );

        let client = Client::builder()
            .timeout(config.timeout)
            .connect_timeout(config.connect_timeout)
            .pool_max_idle_per_host(config.max_concurrent_per_host)
            .pool_idle_timeout(Duration::from_secs(90))
            .gzip(true)
            .brotli(true)
            .deflate(true)
            .zstd(true)
            .http2_prior_knowledge()
            .default_headers(headers.clone())
            .build()
            .map_err(|e| RepositoryError::InvalidConfig {
                message: format!("Failed to create HTTP client: {e}"),
            })?;

        Ok(Self {
            client,
            config,
            rate_limiters: DashMap::new(),
            cache_metadata: DashMap::new(),
            stats: Arc::new(HttpClientStats::new()),
            default_headers: RwLock::new(headers),
        })
    }

    /// Set authentication for a specific host.
    pub fn set_auth(&self, host: &str, auth: AuthType) {
        let mut headers = self.default_headers.write();
        match auth {
            AuthType::Bearer(token) => {
                if let Ok(value) = format!("Bearer {token}").parse() {
                    headers.insert(header::AUTHORIZATION, value);
                }
            }
            AuthType::Basic { username, password } => {
                let credentials = base64_encode(&format!("{username}:{password}"));
                if let Ok(value) = format!("Basic {credentials}").parse() {
                    headers.insert(header::AUTHORIZATION, value);
                }
            }
        }
        debug!(host = %host, "authentication configured");
    }

    /// Get the rate limiter for a host, creating one if needed.
    fn get_rate_limiter(&self, host: &str) -> Arc<HostRateLimiter> {
        self.rate_limiters
            .entry(host.to_string())
            .or_insert_with(|| {
                let quota = Quota::per_second(
                    NonZeroU32::new(self.config.rate_limit_per_host).unwrap_or(NonZeroU32::MIN),
                );
                Arc::new(RateLimiter::direct(quota))
            })
            .clone()
    }

    /// Perform a GET request with retry and rate limiting.
    ///
    /// # Errors
    /// Returns error if request fails after all retries.
    pub async fn get(&self, url: &Url) -> Result<HttpResponse> {
        self.get_with_cache(url, None).await
    }

    /// Perform a GET request with conditional caching support.
    ///
    /// # Errors
    /// Returns error if request fails after all retries.
    pub async fn get_with_cache(&self, url: &Url, cache_key: Option<&str>) -> Result<HttpResponse> {
        let host = url.host_str().ok_or_else(|| RepositoryError::InvalidUrl {
            url: url.to_string(),
            message: "No host in URL".into(),
        })?;

        // Wait for rate limiter
        let limiter = self.get_rate_limiter(host);
        limiter.until_ready().await;

        let url_str = url.to_string();
        let cache_key = cache_key.unwrap_or(&url_str);

        // Build request with conditional headers
        let mut request = self.client.get(url.clone());

        if let Some(metadata) = self.cache_metadata.get(cache_key) {
            if let Some(ref etag) = metadata.etag {
                request = request.header(header::IF_NONE_MATCH, etag.as_str());
            }
            if let Some(ref last_modified) = metadata.last_modified {
                request = request.header(header::IF_MODIFIED_SINCE, last_modified.as_str());
            }
        }

        // Execute with retry
        let config = &self.config;
        let stats = &self.stats;

        let response = (|| async {
            let start = Instant::now();
            let result = request
                .try_clone()
                .ok_or_else(|| RepositoryError::Network {
                    url: url_str.clone(),
                    message: "Failed to clone request".into(),
                    status: None,
                })?
                .send()
                .await;

            let elapsed = start.elapsed();
            stats.record_request(elapsed);

            match result {
                Ok(resp) => {
                    let status = resp.status();

                    if status == StatusCode::NOT_MODIFIED {
                        stats.cache_hits.fetch_add(1, Ordering::Relaxed);
                        stats.successes.fetch_add(1, Ordering::Relaxed);
                        return Ok(HttpResponse {
                            body: bytes::Bytes::new(),
                            status,
                            etag: None,
                            last_modified: None,
                            max_age: None,
                            was_cached: true,
                        });
                    }

                    if status.is_success() {
                        stats.successes.fetch_add(1, Ordering::Relaxed);
                        self.process_response(resp, cache_key).await
                    } else if status == StatusCode::TOO_MANY_REQUESTS {
                        stats.rate_limited.fetch_add(1, Ordering::Relaxed);
                        let retry_after = resp
                            .headers()
                            .get(header::RETRY_AFTER)
                            .and_then(|v| v.to_str().ok())
                            .and_then(|s| s.parse().ok());
                        Err(RepositoryError::RateLimited {
                            url: url_str.clone(),
                            retry_after,
                        })
                    } else if status == StatusCode::UNAUTHORIZED {
                        stats.client_errors.fetch_add(1, Ordering::Relaxed);
                        Err(RepositoryError::AuthRequired {
                            url: url_str.clone(),
                        })
                    } else if status == StatusCode::FORBIDDEN {
                        stats.client_errors.fetch_add(1, Ordering::Relaxed);
                        Err(RepositoryError::AuthFailed {
                            url: url_str.clone(),
                            message: "Access forbidden".into(),
                        })
                    } else if status == StatusCode::NOT_FOUND {
                        stats.client_errors.fetch_add(1, Ordering::Relaxed);
                        Err(RepositoryError::Network {
                            url: url_str.clone(),
                            message: "Not found".into(),
                            status: Some(404),
                        })
                    } else if status.is_client_error() {
                        stats.client_errors.fetch_add(1, Ordering::Relaxed);
                        Err(RepositoryError::Network {
                            url: url_str.clone(),
                            message: format!("Client error: {status}"),
                            status: Some(status.as_u16()),
                        })
                    } else if status.is_server_error() {
                        stats.server_errors.fetch_add(1, Ordering::Relaxed);
                        // Retry server errors
                        Err(RepositoryError::Network {
                            url: url_str.clone(),
                            message: format!("Server error: {status}"),
                            status: Some(status.as_u16()),
                        })
                    } else {
                        Err(RepositoryError::Network {
                            url: url_str.clone(),
                            message: format!("Unexpected status: {status}"),
                            status: Some(status.as_u16()),
                        })
                    }
                }
                Err(e) => {
                    if e.is_timeout() {
                        Err(RepositoryError::Timeout {
                            url: url_str.clone(),
                            timeout_secs: config.timeout.as_secs(),
                        })
                    } else if e.is_connect() {
                        Err(RepositoryError::Unavailable {
                            url: url_str.clone(),
                            message: "Connection failed".into(),
                        })
                    } else {
                        Err(RepositoryError::Network {
                            url: url_str.clone(),
                            message: e.to_string(),
                            status: None,
                        })
                    }
                }
            }
        })
        .retry(
            ExponentialBuilder::default()
                .with_min_delay(config.retry_delay)
                .with_max_delay(config.max_retry_delay)
                .with_max_times(config.max_retries),
        )
        .when(|e| {
            // Retry on server errors and rate limits
            matches!(
                e,
                RepositoryError::Network { status: Some(code), .. } if *code >= 500
            ) || matches!(e, RepositoryError::RateLimited { .. })
                || matches!(e, RepositoryError::Timeout { .. })
                || matches!(e, RepositoryError::Unavailable { .. })
        })
        .notify(|err, dur| {
            stats.retries.fetch_add(1, Ordering::Relaxed);
            warn!(error = %err, retry_in = ?dur, "retrying request");
        })
        .await?;

        Ok(response)
    }

    /// Process a successful response, extracting headers and body.
    async fn process_response(&self, response: Response, cache_key: &str) -> Result<HttpResponse> {
        let headers = response.headers();

        let etag = headers
            .get(header::ETAG)
            .and_then(|v| v.to_str().ok())
            .map(String::from);

        let last_modified = headers
            .get(header::LAST_MODIFIED)
            .and_then(|v| v.to_str().ok())
            .map(String::from);

        let max_age = headers
            .get(header::CACHE_CONTROL)
            .and_then(|v| v.to_str().ok())
            .and_then(parse_max_age);

        let status = response.status();

        // Store cache metadata
        if etag.is_some() || last_modified.is_some() {
            self.cache_metadata.insert(
                cache_key.to_string(),
                CacheMetadata {
                    etag: etag.clone(),
                    last_modified: last_modified.clone(),
                    cached_at: Instant::now(),
                },
            );
        }

        let body = response
            .bytes()
            .await
            .map_err(|e| RepositoryError::Network {
                url: cache_key.to_string(),
                message: format!("Failed to read body: {e}"),
                status: None,
            })?;

        self.stats
            .bytes_received
            .fetch_add(body.len() as u64, Ordering::Relaxed);

        Ok(HttpResponse {
            body,
            status,
            etag,
            last_modified,
            max_age,
            was_cached: false,
        })
    }

    /// Perform a POST request (fire-and-forget for notifications).
    pub async fn post_fire_and_forget(&self, url: &Url, body: &str) {
        let host = match url.host_str() {
            Some(h) => h,
            None => return,
        };

        let limiter = self.get_rate_limiter(host);
        limiter.until_ready().await;

        let client = self.client.clone();
        let url = url.clone();
        let body = body.to_string();

        // Spawn fire-and-forget task
        tokio::spawn(async move {
            let _ = client
                .post(url)
                .header(header::CONTENT_TYPE, "application/json")
                .body(body)
                .send()
                .await;
        });
    }

    /// Get client statistics.
    #[must_use]
    pub fn stats(&self) -> &HttpClientStats {
        &self.stats
    }

    /// Clear cache metadata for a specific URL.
    pub fn clear_cache_metadata(&self, cache_key: &str) {
        self.cache_metadata.remove(cache_key);
    }

    /// Clear all cache metadata.
    pub fn clear_all_cache_metadata(&self) {
        self.cache_metadata.clear();
    }

    /// Get cache metadata for a URL if present.
    #[must_use]
    pub fn get_cache_metadata(&self, cache_key: &str) -> Option<CacheMetadata> {
        self.cache_metadata.get(cache_key).map(|v| v.clone())
    }
}

/// Authentication type.
#[derive(Debug, Clone)]
pub enum AuthType {
    /// Bearer token authentication.
    Bearer(String),
    /// HTTP Basic authentication.
    Basic {
        /// Username.
        username: String,
        /// Password.
        password: String,
    },
}

/// Parse max-age from Cache-Control header.
fn parse_max_age(cache_control: &str) -> Option<Duration> {
    for directive in cache_control.split(',') {
        let directive = directive.trim();
        if let Some(value) = directive.strip_prefix("max-age=")
            && let Ok(seconds) = value.trim().parse::<u64>()
        {
            return Some(Duration::from_secs(seconds));
        }
    }
    None
}

/// Simple base64 encoding for basic auth.
fn base64_encode(input: &str) -> String {
    use std::io::Write;
    let mut buf = Vec::new();
    {
        let mut encoder = base64_writer(&mut buf);
        let _ = encoder.write_all(input.as_bytes());
    }
    String::from_utf8(buf).unwrap_or_default()
}

fn base64_writer(output: &mut Vec<u8>) -> impl std::io::Write + '_ {
    struct Base64Writer<'a> {
        output: &'a mut Vec<u8>,
        buffer: [u8; 3],
        buffer_len: usize,
    }

    impl std::io::Write for Base64Writer<'_> {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            const ALPHABET: &[u8] =
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

            for &byte in buf {
                self.buffer[self.buffer_len] = byte;
                self.buffer_len += 1;

                if self.buffer_len == 3 {
                    let b0 = self.buffer[0];
                    let b1 = self.buffer[1];
                    let b2 = self.buffer[2];

                    self.output.push(ALPHABET[(b0 >> 2) as usize]);
                    self.output
                        .push(ALPHABET[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize]);
                    self.output
                        .push(ALPHABET[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize]);
                    self.output.push(ALPHABET[(b2 & 0x3f) as usize]);

                    self.buffer_len = 0;
                }
            }

            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            const ALPHABET: &[u8] =
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

            match self.buffer_len {
                1 => {
                    let b0 = self.buffer[0];
                    self.output.push(ALPHABET[(b0 >> 2) as usize]);
                    self.output.push(ALPHABET[((b0 & 0x03) << 4) as usize]);
                    self.output.push(b'=');
                    self.output.push(b'=');
                }
                2 => {
                    let b0 = self.buffer[0];
                    let b1 = self.buffer[1];
                    self.output.push(ALPHABET[(b0 >> 2) as usize]);
                    self.output
                        .push(ALPHABET[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize]);
                    self.output.push(ALPHABET[((b1 & 0x0f) << 2) as usize]);
                    self.output.push(b'=');
                }
                _ => {}
            }
            self.buffer_len = 0;
            Ok(())
        }
    }

    impl Drop for Base64Writer<'_> {
        fn drop(&mut self) {
            let _ = std::io::Write::flush(self);
        }
    }

    Base64Writer {
        output,
        buffer: [0; 3],
        buffer_len: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_max_age() {
        assert_eq!(
            parse_max_age("max-age=3600"),
            Some(Duration::from_secs(3600))
        );
        assert_eq!(
            parse_max_age("public, max-age=86400"),
            Some(Duration::from_secs(86400))
        );
        assert_eq!(
            parse_max_age("no-cache, max-age=0"),
            Some(Duration::from_secs(0))
        );
        assert_eq!(parse_max_age("no-cache"), None);
    }

    #[test]
    fn test_base64_encode() {
        assert_eq!(base64_encode("test:password"), "dGVzdDpwYXNzd29yZA==");
        assert_eq!(base64_encode("a"), "YQ==");
        assert_eq!(base64_encode("ab"), "YWI=");
        assert_eq!(base64_encode("abc"), "YWJj");
    }

    #[test]
    fn test_client_config_default() {
        let config = HttpClientConfig::default();
        assert_eq!(config.timeout, Duration::from_secs(30));
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.rate_limit_per_host, 20);
    }

    #[test]
    fn test_stats() {
        let stats = HttpClientStats::new();
        stats.requests.store(100, Ordering::Relaxed);
        stats.successes.store(95, Ordering::Relaxed);
        stats.total_request_time_ms.store(5000, Ordering::Relaxed);

        assert!((stats.avg_request_time_ms() - 50.0).abs() < f64::EPSILON);
        assert!((stats.success_rate() - 95.0).abs() < f64::EPSILON);
    }
}
