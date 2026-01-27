//! Configuration types for the downloader.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

/// Download configuration with all options.
#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)]
pub struct DownloadConfig {
    /// Connection timeout for initial TCP connection.
    pub connect_timeout: Duration,
    /// Read timeout for data transfer.
    pub read_timeout: Duration,
    /// Total request timeout.
    pub total_timeout: Duration,
    /// Number of retry attempts.
    pub max_retries: u32,
    /// Base delay for exponential backoff.
    pub retry_base_delay: Duration,
    /// Maximum delay between retries.
    pub retry_max_delay: Duration,
    /// Maximum concurrent downloads.
    pub max_concurrent: usize,
    /// Maximum concurrent connections per host.
    pub max_connections_per_host: usize,
    /// Enable HTTP/2 multiplexing.
    pub http2_multiplexing: bool,
    /// Enable HTTP/2 adaptive window sizing.
    pub http2_adaptive_window: bool,
    /// Initial HTTP/2 stream window size.
    pub http2_initial_stream_window: u32,
    /// Initial HTTP/2 connection window size.
    pub http2_initial_connection_window: u32,
    /// Enable keep-alive connections.
    pub keep_alive: bool,
    /// Keep-alive idle timeout.
    pub keep_alive_timeout: Duration,
    /// Show progress bars.
    pub show_progress: bool,
    /// Verify checksums.
    pub verify_checksum: bool,
    /// Enable resume for interrupted downloads.
    pub resume_downloads: bool,
    /// Threshold for using memory-mapped files (bytes).
    pub mmap_threshold: u64,
    /// Bandwidth limit in bytes per second (None = unlimited).
    pub bandwidth_limit: Option<u64>,
    /// Mirror URLs for fallback.
    pub mirrors: Vec<String>,
    /// Proxy URL (overrides env vars).
    pub proxy: Option<String>,
    /// Path to auth.json for authentication.
    pub auth_config_path: Option<PathBuf>,
    /// User agent string.
    pub user_agent: String,
    /// Accept encoding header.
    pub accept_encoding: String,
    /// Buffer size for streaming operations.
    pub buffer_size: usize,
    /// Chunk size for large file downloads.
    pub chunk_size: usize,
}

impl Default for DownloadConfig {
    fn default() -> Self {
        // Adaptive concurrency based on CPU cores
        let cpu_cores = std::thread::available_parallelism().map_or(4, std::num::NonZeroUsize::get);
        let max_concurrent = (cpu_cores * 4).clamp(8, 100);

        Self {
            connect_timeout: Duration::from_secs(10),
            read_timeout: Duration::from_secs(30),
            total_timeout: Duration::from_secs(300),
            max_retries: 3,
            retry_base_delay: Duration::from_millis(500),
            retry_max_delay: Duration::from_secs(30),
            max_concurrent,
            max_connections_per_host: 6,
            http2_multiplexing: true,
            http2_adaptive_window: true,
            http2_initial_stream_window: 2 * 1024 * 1024, // 2MB
            http2_initial_connection_window: 4 * 1024 * 1024, // 4MB
            keep_alive: true,
            keep_alive_timeout: Duration::from_secs(90),
            show_progress: true,
            verify_checksum: true,
            resume_downloads: true,
            mmap_threshold: 100 * 1024 * 1024, // 100MB
            bandwidth_limit: None,
            mirrors: Vec::new(),
            proxy: None,
            auth_config_path: None,
            user_agent: format!("libretto/{}", env!("CARGO_PKG_VERSION")),
            accept_encoding: "gzip, deflate, br, zstd".to_string(),
            buffer_size: 128 * 1024,     // 128KB
            chunk_size: 8 * 1024 * 1024, // 8MB chunks for parallel download
        }
    }
}

impl DownloadConfig {
    /// Create a new config builder.
    #[must_use]
    pub fn builder() -> DownloadConfigBuilder {
        DownloadConfigBuilder::default()
    }

    /// Get adaptive concurrency based on available system resources.
    #[must_use]
    pub fn adaptive_concurrency() -> usize {
        let cpu_cores = std::thread::available_parallelism().map_or(4, std::num::NonZeroUsize::get);
        (cpu_cores * 4).clamp(8, 100)
    }
}

/// Builder for `DownloadConfig`.
#[derive(Debug, Default)]
pub struct DownloadConfigBuilder {
    config: DownloadConfig,
}

impl DownloadConfigBuilder {
    /// Set connection timeout.
    #[must_use]
    pub const fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.config.connect_timeout = timeout;
        self
    }

    /// Set read timeout.
    #[must_use]
    pub const fn read_timeout(mut self, timeout: Duration) -> Self {
        self.config.read_timeout = timeout;
        self
    }

    /// Set total request timeout.
    #[must_use]
    pub const fn total_timeout(mut self, timeout: Duration) -> Self {
        self.config.total_timeout = timeout;
        self
    }

    /// Set maximum retries.
    #[must_use]
    pub const fn max_retries(mut self, retries: u32) -> Self {
        self.config.max_retries = retries;
        self
    }

    /// Set maximum concurrent downloads.
    #[must_use]
    pub const fn max_concurrent(mut self, concurrent: usize) -> Self {
        self.config.max_concurrent = concurrent;
        self
    }

    /// Set maximum connections per host.
    #[must_use]
    pub const fn max_connections_per_host(mut self, connections: usize) -> Self {
        self.config.max_connections_per_host = connections;
        self
    }

    /// Enable or disable HTTP/2 multiplexing.
    #[must_use]
    pub const fn http2_multiplexing(mut self, enabled: bool) -> Self {
        self.config.http2_multiplexing = enabled;
        self
    }

    /// Enable or disable progress bars.
    #[must_use]
    pub const fn show_progress(mut self, show: bool) -> Self {
        self.config.show_progress = show;
        self
    }

    /// Enable or disable checksum verification.
    #[must_use]
    pub const fn verify_checksum(mut self, verify: bool) -> Self {
        self.config.verify_checksum = verify;
        self
    }

    /// Enable or disable download resumption.
    #[must_use]
    pub const fn resume_downloads(mut self, resume: bool) -> Self {
        self.config.resume_downloads = resume;
        self
    }

    /// Set bandwidth limit in bytes per second.
    #[must_use]
    pub const fn bandwidth_limit(mut self, limit: Option<u64>) -> Self {
        self.config.bandwidth_limit = limit;
        self
    }

    /// Set mirror URLs.
    #[must_use]
    pub fn mirrors(mut self, mirrors: Vec<String>) -> Self {
        self.config.mirrors = mirrors;
        self
    }

    /// Set proxy URL.
    #[must_use]
    pub fn proxy(mut self, proxy: Option<String>) -> Self {
        self.config.proxy = proxy;
        self
    }

    /// Set auth config path.
    #[must_use]
    pub fn auth_config_path(mut self, path: Option<PathBuf>) -> Self {
        self.config.auth_config_path = path;
        self
    }

    /// Set memory-map threshold.
    #[must_use]
    pub const fn mmap_threshold(mut self, threshold: u64) -> Self {
        self.config.mmap_threshold = threshold;
        self
    }

    /// Build the configuration.
    #[must_use]
    pub fn build(self) -> DownloadConfig {
        self.config
    }
}

/// Authentication configuration from auth.json.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuthConfig {
    /// HTTP Basic auth credentials by domain.
    #[serde(default, rename = "http-basic")]
    pub http_basic: std::collections::HashMap<String, HttpBasicAuth>,
    /// Bearer tokens by domain.
    #[serde(default)]
    pub bearer: std::collections::HashMap<String, BearerAuth>,
    /// GitHub OAuth tokens.
    #[serde(default, rename = "github-oauth")]
    pub github_oauth: std::collections::HashMap<String, String>,
    /// GitLab tokens.
    #[serde(default, rename = "gitlab-token")]
    pub gitlab_token: std::collections::HashMap<String, String>,
    /// GitLab OAuth tokens.
    #[serde(default, rename = "gitlab-oauth")]
    pub gitlab_oauth: std::collections::HashMap<String, GitLabOAuth>,
    /// Bitbucket OAuth tokens.
    #[serde(default, rename = "bitbucket-oauth")]
    pub bitbucket_oauth: std::collections::HashMap<String, BitbucketOAuth>,
}

/// HTTP Basic authentication credentials.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpBasicAuth {
    /// Username.
    pub username: String,
    /// Password.
    pub password: String,
}

/// Bearer token authentication.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BearerAuth {
    /// Bearer token.
    pub token: String,
}

/// GitLab OAuth credentials.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitLabOAuth {
    /// OAuth token.
    pub token: String,
}

/// Bitbucket OAuth credentials.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BitbucketOAuth {
    /// Consumer key.
    pub consumer_key: String,
    /// Consumer secret.
    pub consumer_secret: String,
}

impl AuthConfig {
    /// Load auth config from file.
    ///
    /// # Errors
    /// Returns error if file cannot be read or parsed.
    pub fn load(path: &std::path::Path) -> Result<Self, std::io::Error> {
        let content = std::fs::read_to_string(path)?;
        sonic_rs::from_str(&content)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }

    /// Get HTTP Basic auth for a domain.
    #[must_use]
    pub fn get_http_basic(&self, domain: &str) -> Option<&HttpBasicAuth> {
        self.http_basic.get(domain)
    }

    /// Get bearer token for a domain.
    #[must_use]
    pub fn get_bearer(&self, domain: &str) -> Option<&str> {
        self.bearer.get(domain).map(|b| b.token.as_str())
    }

    /// Get GitHub OAuth token for a domain.
    #[must_use]
    pub fn get_github_oauth(&self, domain: &str) -> Option<&str> {
        self.github_oauth.get(domain).map(String::as_str)
    }

    /// Get GitLab token for a domain.
    #[must_use]
    pub fn get_gitlab_token(&self, domain: &str) -> Option<&str> {
        self.gitlab_token.get(domain).map(String::as_str)
    }
}

/// Checksum type for integrity verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChecksumType {
    /// SHA-256 hash.
    Sha256,
    /// SHA-1 hash.
    Sha1,
    /// BLAKE3 hash (fastest, SIMD-accelerated).
    Blake3,
}

impl ChecksumType {
    /// Parse checksum type from string.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "sha256" | "sha-256" => Some(Self::Sha256),
            "sha1" | "sha-1" => Some(Self::Sha1),
            "blake3" => Some(Self::Blake3),
            _ => None,
        }
    }

    /// Get expected hex length for this checksum type.
    #[must_use]
    pub const fn hex_length(&self) -> usize {
        match self {
            Self::Sha256 | Self::Blake3 => 64,
            Self::Sha1 => 40,
        }
    }

    /// Detect checksum type from hex string length.
    #[must_use]
    pub const fn detect_from_len(len: usize) -> Option<Self> {
        match len {
            40 => Some(Self::Sha1),
            64 => Some(Self::Sha256), // Could be BLAKE3, but SHA256 is more common
            _ => None,
        }
    }

    /// Detect checksum type from hex string length.
    #[must_use]
    pub fn detect(hex: &str) -> Option<Self> {
        Self::detect_from_len(hex.len())
    }
}

/// Expected checksum for verification.
#[derive(Debug, Clone)]
pub struct ExpectedChecksum {
    /// Checksum type.
    pub checksum_type: ChecksumType,
    /// Hex-encoded checksum value.
    pub value: String,
}

impl ExpectedChecksum {
    /// Create a new expected checksum.
    #[must_use]
    pub fn new(checksum_type: ChecksumType, value: impl Into<String>) -> Self {
        Self {
            checksum_type,
            value: value.into(),
        }
    }

    /// Create from a hex string, auto-detecting the type.
    #[must_use]
    pub fn from_hex(hex: impl Into<String>) -> Option<Self> {
        let value = hex.into();
        let checksum_type = ChecksumType::detect(&value)?;
        Some(Self {
            checksum_type,
            value,
        })
    }

    /// Create a SHA-256 checksum.
    #[must_use]
    pub fn sha256(value: impl Into<String>) -> Self {
        Self::new(ChecksumType::Sha256, value)
    }

    /// Create a SHA-1 checksum.
    #[must_use]
    pub fn sha1(value: impl Into<String>) -> Self {
        Self::new(ChecksumType::Sha1, value)
    }

    /// Create a BLAKE3 checksum.
    #[must_use]
    pub fn blake3(value: impl Into<String>) -> Self {
        Self::new(ChecksumType::Blake3, value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config() {
        let config = DownloadConfig::default();
        assert!(config.max_concurrent >= 8);
        assert!(config.http2_multiplexing);
        assert!(config.verify_checksum);
    }

    #[test]
    fn config_builder() {
        let config = DownloadConfig::builder()
            .max_concurrent(50)
            .bandwidth_limit(Some(1_000_000))
            .show_progress(false)
            .build();

        assert_eq!(config.max_concurrent, 50);
        assert_eq!(config.bandwidth_limit, Some(1_000_000));
        assert!(!config.show_progress);
    }

    #[test]
    fn checksum_type_detection() {
        assert_eq!(
            ChecksumType::detect("a".repeat(40).as_str()),
            Some(ChecksumType::Sha1)
        );
        assert_eq!(
            ChecksumType::detect("a".repeat(64).as_str()),
            Some(ChecksumType::Sha256)
        );
        assert_eq!(ChecksumType::detect("abc"), None);
    }

    #[test]
    fn expected_checksum_from_hex() {
        let sha1 = ExpectedChecksum::from_hex("a".repeat(40));
        assert!(sha1.is_some());
        assert_eq!(sha1.unwrap().checksum_type, ChecksumType::Sha1);

        let sha256 = ExpectedChecksum::from_hex("b".repeat(64));
        assert!(sha256.is_some());
        assert_eq!(sha256.unwrap().checksum_type, ChecksumType::Sha256);
    }
}
