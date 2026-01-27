//! Ultra-fast parallel package downloader.
//!
//! Implements concurrent downloads with HTTP/2 multiplexing,
//! adaptive concurrency, mirror fallback, and progress tracking.

use crate::checksum::MultiHasher;
use crate::client::HttpClient;
use crate::config::{AuthConfig, DownloadConfig, ExpectedChecksum};
use crate::error::{DownloadError, Result};
use crate::extract::Extractor;
use crate::progress::{DownloadProgress, ProgressTracker};
use crate::retry::{with_retry, CircuitBreaker, RetryConfig};
use crate::source::{ArchiveType, DownloadResult, DownloadSource, Source, SourceType};
use crate::stream::StreamDownloader;
use crate::throttle::BandwidthThrottler;
use crate::vcs::{copy_path, GitHandler, HgHandler, SvnHandler};
use dashmap::DashMap;
use futures_util::stream::{self, StreamExt};
use std::path::Path;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::Semaphore;
use tracing::{error, info, warn};

/// Ultra-fast parallel downloader for packages.
///
/// Features:
/// - Adaptive concurrency based on CPU cores
/// - HTTP/2 multiplexing for efficient connection reuse
/// - Resumable downloads with Range headers
/// - Memory-mapped files for large downloads (>100MB)
/// - Multi-algorithm checksum verification (SHA-256, SHA-1, BLAKE3)
/// - Progress tracking with multi-progress bars
/// - Retry with exponential backoff
/// - Mirror fallback support
/// - Bandwidth throttling
/// - Authentication from auth.json
pub struct ParallelDownloader {
    /// Stream downloader.
    stream_downloader: StreamDownloader,
    /// Download configuration.
    config: Arc<DownloadConfig>,
    /// Retry configuration.
    retry_config: RetryConfig,
    /// Concurrency semaphore.
    semaphore: Arc<Semaphore>,
    /// Progress tracker.
    progress: ProgressTracker,
    /// Per-host circuit breakers.
    circuit_breakers: Arc<DashMap<String, CircuitBreaker>>,
    /// Download statistics.
    stats: Arc<DownloadStats>,
    /// Archive extractor.
    extractor: Extractor,
    /// Git handler.
    git_handler: GitHandler,
    /// SVN handler.
    svn_handler: SvnHandler,
    /// Hg handler.
    hg_handler: HgHandler,
}

impl std::fmt::Debug for ParallelDownloader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParallelDownloader")
            .field("max_concurrent", &self.config.max_concurrent)
            .field("http2", &self.config.http2_multiplexing)
            .field("progress_enabled", &self.progress.is_enabled())
            .finish()
    }
}

/// Download statistics.
#[derive(Debug, Default)]
pub struct DownloadStats {
    /// Total packages downloaded.
    pub total_packages: AtomicUsize,
    /// Successful downloads.
    pub successful: AtomicUsize,
    /// Failed downloads.
    pub failed: AtomicUsize,
    /// Total bytes downloaded.
    pub total_bytes: AtomicU64,
    /// Resumed downloads.
    pub resumed: AtomicUsize,
    /// Cache hits (already downloaded).
    pub cache_hits: AtomicUsize,
}

impl DownloadStats {
    /// Get a snapshot of current statistics.
    #[must_use]
    pub fn snapshot(&self) -> StatsSnapshot {
        StatsSnapshot {
            total_packages: self.total_packages.load(Ordering::Relaxed),
            successful: self.successful.load(Ordering::Relaxed),
            failed: self.failed.load(Ordering::Relaxed),
            total_bytes: self.total_bytes.load(Ordering::Relaxed),
            resumed: self.resumed.load(Ordering::Relaxed),
            cache_hits: self.cache_hits.load(Ordering::Relaxed),
        }
    }
}

/// Snapshot of download statistics.
#[derive(Debug, Clone)]
pub struct StatsSnapshot {
    /// Total packages.
    pub total_packages: usize,
    /// Successful downloads.
    pub successful: usize,
    /// Failed downloads.
    pub failed: usize,
    /// Total bytes.
    pub total_bytes: u64,
    /// Resumed downloads.
    pub resumed: usize,
    /// Cache hits.
    pub cache_hits: usize,
}

impl ParallelDownloader {
    /// Create a new parallel downloader with configuration.
    ///
    /// # Errors
    /// Returns error if HTTP client cannot be created.
    pub fn new(config: DownloadConfig, auth: Option<AuthConfig>) -> Result<Self> {
        let throttler = BandwidthThrottler::new(config.bandwidth_limit);
        let client = HttpClient::new(config.clone(), auth)?;
        let stream_downloader = StreamDownloader::new(client.clone(), throttler.clone());
        let semaphore = Arc::new(Semaphore::new(config.max_concurrent));
        let progress = ProgressTracker::new(config.show_progress);

        let retry_config = RetryConfig {
            max_retries: config.max_retries,
            base_delay: config.retry_base_delay,
            max_delay: config.retry_max_delay,
            jitter: 0.1,
        };

        Ok(Self {
            stream_downloader,
            config: Arc::new(config),
            retry_config,
            semaphore,
            progress,
            circuit_breakers: Arc::new(DashMap::new()),
            stats: Arc::new(DownloadStats::default()),
            extractor: Extractor::new(),
            git_handler: GitHandler::new(),
            svn_handler: SvnHandler::new(),
            hg_handler: HgHandler::new(),
        })
    }

    /// Create with default configuration.
    ///
    /// # Errors
    /// Returns error if downloader cannot be created.
    pub fn with_defaults() -> Result<Self> {
        Self::new(DownloadConfig::default(), None)
    }

    /// Create a builder for more control over configuration.
    #[must_use]
    pub fn builder() -> ParallelDownloaderBuilder {
        ParallelDownloaderBuilder::default()
    }

    /// Download multiple packages concurrently.
    ///
    /// # Errors
    /// Returns first error encountered, but continues downloading other packages.
    pub async fn download_all(
        &self,
        sources: Vec<DownloadSource>,
    ) -> Vec<std::result::Result<DownloadResult, DownloadError>> {
        let total = sources.len();
        self.stats.total_packages.store(total, Ordering::Relaxed);
        self.progress.set_total(total);

        info!(count = total, "starting parallel downloads");

        let results: Vec<_> = stream::iter(sources)
            .map(|source| self.download_source(source))
            .buffer_unordered(self.config.max_concurrent)
            .collect()
            .await;

        self.progress.finish();

        let stats = self.stats.snapshot();
        info!(
            successful = stats.successful,
            failed = stats.failed,
            bytes = stats.total_bytes,
            resumed = stats.resumed,
            "downloads complete"
        );

        results
    }

    /// Download a single package source.
    async fn download_source(
        &self,
        source: DownloadSource,
    ) -> std::result::Result<DownloadResult, DownloadError> {
        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|_| DownloadError::Cancelled)?;

        let id = source.id();
        let progress = self.progress.start_download(&id, &source.name, None);

        let result = self.download_source_inner(&source, &progress).await;

        match &result {
            Ok(r) => {
                self.stats.successful.fetch_add(1, Ordering::Relaxed);
                self.stats.total_bytes.fetch_add(r.size, Ordering::Relaxed);
                if r.resumed {
                    self.stats.resumed.fetch_add(1, Ordering::Relaxed);
                }
                progress.complete(r.size);
            }
            Err(e) => {
                self.stats.failed.fetch_add(1, Ordering::Relaxed);
                progress.fail(&e.to_string());
                error!(package = %source.name, error = %e, "download failed");
            }
        }

        result
    }

    /// Inner download logic with retry and mirror support.
    async fn download_source_inner(
        &self,
        source: &DownloadSource,
        progress: &DownloadProgress,
    ) -> Result<DownloadResult> {
        // Try each source in order (primary + fallbacks)
        let mut last_error = None;

        for src in source.all_sources() {
            match self.download_from_source(src, source, progress).await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    warn!(
                        package = %source.name,
                        source_type = ?src.source_type(),
                        error = %e,
                        "source failed, trying next"
                    );
                    last_error = Some(e);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| DownloadError::NotFound {
            url: source.name.clone(),
        }))
    }

    /// Download from a specific source.
    async fn download_from_source(
        &self,
        src: &Source,
        download_source: &DownloadSource,
        progress: &DownloadProgress,
    ) -> Result<DownloadResult> {
        match src {
            Source::Dist { url, archive_type } => {
                self.download_dist(
                    url,
                    *archive_type,
                    &download_source.dest,
                    &download_source.checksums,
                    &download_source.name,
                    &download_source.version,
                    progress,
                )
                .await
            }
            Source::Git { url, reference } => {
                self.download_git(
                    url,
                    reference,
                    &download_source.dest,
                    &download_source.name,
                    &download_source.version,
                )
                .await
            }
            Source::Svn { url, reference } => {
                self.download_svn(
                    url,
                    reference.as_deref(),
                    &download_source.dest,
                    &download_source.name,
                    &download_source.version,
                )
                .await
            }
            Source::Hg { url, reference } => {
                self.download_hg(
                    url,
                    reference.as_deref(),
                    &download_source.dest,
                    &download_source.name,
                    &download_source.version,
                )
                .await
            }
            Source::Path { path, symlink } => {
                self.download_path(
                    path,
                    *symlink,
                    &download_source.dest,
                    &download_source.name,
                    &download_source.version,
                )
                .await
            }
        }
    }

    /// Download a dist archive.
    #[allow(clippy::too_many_arguments)]
    async fn download_dist(
        &self,
        url: &url::Url,
        archive_type: ArchiveType,
        dest: &Path,
        checksums: &[ExpectedChecksum],
        name: &str,
        version: &str,
        progress: &DownloadProgress,
    ) -> Result<DownloadResult> {
        // Check circuit breaker for this host
        let host = url.host_str().unwrap_or("unknown").to_string();
        let cb = self.circuit_breakers.entry(host.clone()).or_default();

        if cb.is_open() {
            return Err(DownloadError::network(format!(
                "circuit breaker open for {host}"
            )));
        }

        // Create temp file for archive
        let archive_ext = archive_type.extension();
        let archive_path = dest.with_extension(format!("download{archive_ext}"));

        // Download with retry
        let download_result = with_retry(&self.retry_config, || {
            self.stream_downloader
                .download(url, &archive_path, checksums, progress)
        })
        .await;

        let downloaded = match download_result {
            Ok(d) => {
                cb.record_success();
                d
            }
            Err(e) => {
                cb.record_failure();
                return Err(e);
            }
        };

        // Verify checksums
        if !checksums.is_empty() && self.config.verify_checksum {
            downloaded.verify(checksums, name)?;
        }

        // Extract archive
        let extract_dest = dest;
        self.extractor
            .extract(&archive_path, extract_dest)
            .await
            .map_err(|e| {
                // Clean up archive on extraction failure
                let _ = std::fs::remove_file(&archive_path);
                e
            })?;

        // Remove archive after extraction
        let _ = std::fs::remove_file(&archive_path);

        Ok(DownloadResult {
            name: name.to_string(),
            version: version.to_string(),
            path: dest.to_path_buf(),
            size: downloaded.size,
            checksums: downloaded.checksums,
            source_type: SourceType::Dist,
            resumed: downloaded.resumed,
        })
    }

    /// Download from Git repository.
    async fn download_git(
        &self,
        url: &url::Url,
        reference: &crate::source::VcsRef,
        dest: &Path,
        name: &str,
        version: &str,
    ) -> Result<DownloadResult> {
        let libretto_ref = match reference {
            crate::source::VcsRef::Branch(b) => crate::source::VcsRef::Branch(b.clone()),
            crate::source::VcsRef::Tag(t) => crate::source::VcsRef::Tag(t.clone()),
            crate::source::VcsRef::Commit(c) => crate::source::VcsRef::Commit(c.clone()),
        };

        let result = with_retry(&self.retry_config, || {
            self.git_handler.clone(url, dest, &libretto_ref)
        })
        .await?;

        // Calculate size of cloned repo
        let size = dir_size(dest).await.unwrap_or(0);

        // Compute checksums (not applicable for VCS)
        let checksums = MultiHasher::new().finalize();

        Ok(DownloadResult {
            name: name.to_string(),
            version: version.to_string(),
            path: result.path,
            size,
            checksums,
            source_type: SourceType::Git,
            resumed: false,
        })
    }

    /// Download from SVN repository.
    async fn download_svn(
        &self,
        url: &url::Url,
        revision: Option<&str>,
        dest: &Path,
        name: &str,
        version: &str,
    ) -> Result<DownloadResult> {
        let result = with_retry(&self.retry_config, || {
            self.svn_handler.checkout(url, dest, revision)
        })
        .await?;

        let size = dir_size(dest).await.unwrap_or(0);
        let checksums = MultiHasher::new().finalize();

        Ok(DownloadResult {
            name: name.to_string(),
            version: version.to_string(),
            path: result.path,
            size,
            checksums,
            source_type: SourceType::Svn,
            resumed: false,
        })
    }

    /// Download from Mercurial repository.
    async fn download_hg(
        &self,
        url: &url::Url,
        revision: Option<&str>,
        dest: &Path,
        name: &str,
        version: &str,
    ) -> Result<DownloadResult> {
        let result = with_retry(&self.retry_config, || {
            self.hg_handler.clone(url, dest, revision)
        })
        .await?;

        let size = dir_size(dest).await.unwrap_or(0);
        let checksums = MultiHasher::new().finalize();

        Ok(DownloadResult {
            name: name.to_string(),
            version: version.to_string(),
            path: result.path,
            size,
            checksums,
            source_type: SourceType::Hg,
            resumed: false,
        })
    }

    /// Copy from local path.
    async fn download_path(
        &self,
        src: &Path,
        symlink: bool,
        dest: &Path,
        name: &str,
        version: &str,
    ) -> Result<DownloadResult> {
        copy_path(src, dest, symlink).await?;

        let size = dir_size(dest).await.unwrap_or(0);
        let checksums = MultiHasher::new().finalize();

        Ok(DownloadResult {
            name: name.to_string(),
            version: version.to_string(),
            path: dest.to_path_buf(),
            size,
            checksums,
            source_type: SourceType::Path,
            resumed: false,
        })
    }

    /// Get download statistics.
    #[must_use]
    pub fn stats(&self) -> StatsSnapshot {
        self.stats.snapshot()
    }

    /// Get the configuration.
    #[must_use]
    pub fn config(&self) -> &DownloadConfig {
        &self.config
    }
}

/// Builder for `ParallelDownloader`.
#[derive(Debug, Default)]
pub struct ParallelDownloaderBuilder {
    config: DownloadConfig,
    auth: Option<AuthConfig>,
}

impl ParallelDownloaderBuilder {
    /// Set download configuration.
    #[must_use]
    pub fn config(mut self, config: DownloadConfig) -> Self {
        self.config = config;
        self
    }

    /// Set authentication configuration.
    #[must_use]
    pub fn auth(mut self, auth: AuthConfig) -> Self {
        self.auth = Some(auth);
        self
    }

    /// Set maximum concurrent downloads.
    #[must_use]
    pub const fn max_concurrent(mut self, n: usize) -> Self {
        self.config.max_concurrent = n;
        self
    }

    /// Set bandwidth limit in bytes per second.
    #[must_use]
    pub const fn bandwidth_limit(mut self, limit: Option<u64>) -> Self {
        self.config.bandwidth_limit = limit;
        self
    }

    /// Enable or disable progress display.
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

    /// Set mirror URLs.
    #[must_use]
    pub fn mirrors(mut self, mirrors: Vec<String>) -> Self {
        self.config.mirrors = mirrors;
        self
    }

    /// Build the downloader.
    ///
    /// # Errors
    /// Returns error if downloader cannot be created.
    pub fn build(self) -> Result<ParallelDownloader> {
        ParallelDownloader::new(self.config, self.auth)
    }
}

/// Calculate directory size recursively.
async fn dir_size(path: &Path) -> Result<u64> {
    if path.is_file() {
        let metadata = tokio::fs::metadata(path)
            .await
            .map_err(|e| DownloadError::io(path, e))?;
        return Ok(metadata.len());
    }

    let mut total = 0u64;
    let mut entries = tokio::fs::read_dir(path)
        .await
        .map_err(|e| DownloadError::io(path, e))?;

    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| DownloadError::io(path, e))?
    {
        let entry_path = entry.path();
        let metadata = entry
            .metadata()
            .await
            .map_err(|e| DownloadError::io(&entry_path, e))?;

        if metadata.is_file() {
            total += metadata.len();
        } else if metadata.is_dir() {
            total += Box::pin(dir_size(&entry_path)).await.unwrap_or(0);
        }
    }

    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parallel_downloader_builder() {
        let downloader = ParallelDownloader::builder()
            .max_concurrent(50)
            .bandwidth_limit(Some(1_000_000))
            .show_progress(false)
            .build();

        assert!(downloader.is_ok());
        let dl = downloader.unwrap();
        assert_eq!(dl.config.max_concurrent, 50);
        assert_eq!(dl.config.bandwidth_limit, Some(1_000_000));
    }

    #[test]
    fn download_stats_default() {
        let stats = DownloadStats::default();
        let snapshot = stats.snapshot();
        assert_eq!(snapshot.total_packages, 0);
        assert_eq!(snapshot.successful, 0);
        assert_eq!(snapshot.failed, 0);
    }

    #[tokio::test]
    async fn downloader_creation() {
        let downloader = ParallelDownloader::with_defaults();
        assert!(downloader.is_ok());
    }
}
