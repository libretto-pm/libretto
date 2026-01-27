//! Ultra-fast parallel package downloader for Libretto.
//!
//! This crate provides high-performance package downloading with:
//!
//! - **HTTP/2 multiplexing** for efficient connection reuse
//! - **Adaptive concurrency** based on CPU cores (up to 100 concurrent downloads)
//! - **Resumable downloads** using Range headers
//! - **Memory-mapped files** for large downloads (>100MB)
//! - **Multi-algorithm checksums** (SHA-256, SHA-1, SIMD-accelerated BLAKE3)
//! - **Streaming decompression** for ZIP, tar.gz, tar.bz2, tar.xz, tar.zst
//! - **Progress tracking** with multi-progress bars
//! - **Retry with exponential backoff** and mirror fallback
//! - **Bandwidth throttling** using token bucket algorithm
//! - **VCS support** for Git, SVN, and Mercurial
//! - **Authentication** from auth.json (HTTP Basic, Bearer, OAuth)
//!
//! # Performance
//!
//! Designed to saturate 1Gbps connections with 100 concurrent downloads.
//!
//! # Example
//!
//! ```no_run
//! use libretto_downloader::{ParallelDownloader, DownloadSource, Source, ArchiveType};
//! use std::path::PathBuf;
//! use url::Url;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create downloader with default settings
//! let downloader = ParallelDownloader::with_defaults()?;
//!
//! // Define packages to download
//! let sources = vec![
//!     DownloadSource::new(
//!         "symfony/console",
//!         "6.4.0",
//!         Source::dist_with_type(
//!             Url::parse("https://example.com/console-6.4.0.zip")?,
//!             ArchiveType::Zip,
//!         ),
//!         PathBuf::from("vendor/symfony/console"),
//!     ),
//! ];
//!
//! // Download all packages concurrently
//! let results = downloader.download_all(sources).await;
//!
//! for result in results {
//!     match result {
//!         Ok(r) => println!("Downloaded {} ({} bytes)", r.name, r.size),
//!         Err(e) => eprintln!("Failed: {}", e),
//!     }
//! }
//!
//! // Print statistics
//! let stats = downloader.stats();
//! println!("Success: {}, Failed: {}", stats.successful, stats.failed);
//! # Ok(())
//! # }
//! ```
//!
//! # Configuration
//!
//! ```no_run
//! use libretto_downloader::{ParallelDownloader, DownloadConfig};
//! use std::time::Duration;
//!
//! # fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let config = DownloadConfig::builder()
//!     .max_concurrent(50)
//!     .bandwidth_limit(Some(10 * 1024 * 1024)) // 10 MB/s
//!     .connect_timeout(Duration::from_secs(5))
//!     .max_retries(5)
//!     .show_progress(true)
//!     .verify_checksum(true)
//!     .build();
//!
//! let downloader = ParallelDownloader::new(config, None)?;
//! # Ok(())
//! # }
//! ```

#![deny(clippy::all)]
#![allow(clippy::module_name_repetitions)]

pub mod checksum;
mod client;
pub mod config;
mod error;
mod extract;
mod parallel;
mod progress;
mod retry;
mod source;
mod stream;
pub mod throttle;
mod vcs;

// Re-export main types
pub use checksum::{
    blake3_file, bytes_to_hex, hex_to_bytes, sha1_file, sha256_file, verify_file,
    ComputedChecksums, MultiHasher,
};
pub use client::HttpClient;
pub use config::{
    AuthConfig, BearerAuth, BitbucketOAuth, ChecksumType, DownloadConfig, DownloadConfigBuilder,
    ExpectedChecksum, GitLabOAuth, HttpBasicAuth,
};
pub use error::{DownloadError, Result};
pub use extract::{extract, extract_with_strip, ExtractOptions, ExtractionResult, Extractor};
pub use parallel::{DownloadStats, ParallelDownloader, ParallelDownloaderBuilder, StatsSnapshot};
pub use progress::{DownloadProgress, ProgressStats, ProgressTracker};
pub use retry::{with_mirrors, with_retry, CircuitBreaker, RetryConfig};
pub use source::{ArchiveType, DownloadResult, DownloadSource, Source, SourceType, VcsRef};
pub use stream::{DownloadState, DownloadedFile, StreamDownloader};
pub use throttle::BandwidthThrottler;
pub use vcs::{copy_path, GitHandler, GitResult, HgHandler, HgResult, SvnHandler, SvnResult};

/// Simple single-file downloader for backwards compatibility.
///
/// For parallel downloads, use [`ParallelDownloader`] instead.
#[derive(Debug)]
pub struct Downloader {
    inner: ParallelDownloader,
}

impl Downloader {
    /// Create a new downloader with default options.
    ///
    /// # Errors
    /// Returns error if HTTP client cannot be created.
    pub fn with_defaults() -> Result<Self> {
        let config = DownloadConfig::builder()
            .max_concurrent(1)
            .show_progress(true)
            .build();
        Ok(Self {
            inner: ParallelDownloader::new(config, None)?,
        })
    }

    /// Create a new downloader with custom options.
    ///
    /// # Errors
    /// Returns error if HTTP client cannot be created.
    pub fn new(options: &DownloadOptions) -> Result<Self> {
        let config = DownloadConfig::builder()
            .max_concurrent(1)
            .connect_timeout(options.connect_timeout)
            .read_timeout(options.read_timeout)
            .max_retries(options.retries)
            .show_progress(options.show_progress)
            .verify_checksum(options.verify_checksum)
            .build();
        Ok(Self {
            inner: ParallelDownloader::new(config, None)?,
        })
    }

    /// Download a URL to a file.
    ///
    /// # Errors
    /// Returns error if download fails.
    pub async fn download(
        &self,
        url: &url::Url,
        dest: &std::path::Path,
    ) -> Result<SimpleDownloadResult> {
        let progress = ProgressTracker::new(self.inner.config().show_progress);
        let dl_progress = progress.start_download("single", url.as_str(), None);

        let throttler = BandwidthThrottler::new(self.inner.config().bandwidth_limit);
        let stream_dl = StreamDownloader::new(
            HttpClient::new((*self.inner.config()).clone(), None)?,
            throttler,
        );

        let result = stream_dl.download(url, dest, &[], &dl_progress).await?;

        dl_progress.complete(result.size);
        progress.finish();

        Ok(SimpleDownloadResult {
            path: result.path,
            hash: libretto_core::ContentHash::from_bytes(&result.checksums.blake3),
            size: result.size,
        })
    }

    /// Download to bytes in memory.
    ///
    /// # Errors
    /// Returns error if download fails.
    pub async fn download_bytes(&self, url: &url::Url) -> Result<bytes::Bytes> {
        let client = HttpClient::new((*self.inner.config()).clone(), None)?;
        let response = client.get(url).await?;
        response
            .bytes()
            .await
            .map_err(|e| DownloadError::network(e.to_string()))
    }

    /// Verify a file's checksum.
    ///
    /// # Errors
    /// Returns error if checksums don't match.
    pub fn verify(&self, path: &std::path::Path, expected: &str, name: &str) -> Result<()> {
        if let Some(checksum) = ExpectedChecksum::from_hex(expected) {
            verify_file(path, &checksum, name)?;
        }
        Ok(())
    }
}

/// Download options for backwards compatibility.
#[derive(Debug, Clone)]
pub struct DownloadOptions {
    /// Connection timeout.
    pub connect_timeout: std::time::Duration,
    /// Read timeout.
    pub read_timeout: std::time::Duration,
    /// Number of retries.
    pub retries: u32,
    /// Show progress bar.
    pub show_progress: bool,
    /// Verify checksum.
    pub verify_checksum: bool,
}

impl Default for DownloadOptions {
    fn default() -> Self {
        Self {
            connect_timeout: std::time::Duration::from_secs(10),
            read_timeout: std::time::Duration::from_secs(60),
            retries: 3,
            show_progress: true,
            verify_checksum: true,
        }
    }
}

/// Simple download result for backwards compatibility.
#[derive(Debug)]
pub struct SimpleDownloadResult {
    /// Path to downloaded file.
    pub path: std::path::PathBuf,
    /// Content hash (BLAKE3).
    pub hash: libretto_core::ContentHash,
    /// Size in bytes.
    pub size: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_options_default() {
        let opts = DownloadOptions::default();
        assert_eq!(opts.retries, 3);
        assert!(opts.show_progress);
        assert!(opts.verify_checksum);
    }

    #[tokio::test]
    async fn downloader_creation() {
        let downloader = Downloader::with_defaults();
        assert!(downloader.is_ok());
    }

    #[tokio::test]
    async fn parallel_downloader_creation() {
        let downloader = ParallelDownloader::with_defaults();
        assert!(downloader.is_ok());
    }

    #[test]
    fn config_adaptive_concurrency() {
        let concurrency = DownloadConfig::adaptive_concurrency();
        assert!(concurrency >= 8);
        assert!(concurrency <= 100);
    }
}
