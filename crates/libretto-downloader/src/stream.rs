//! Streaming download with resume support and memory mapping.
//!
//! Provides efficient download strategies for files of all sizes.

use crate::checksum::MultiHasher;
use crate::client::HttpClient;
use crate::config::{DownloadConfig, ExpectedChecksum};
use crate::error::{DownloadError, Result};
use crate::progress::DownloadProgress;
use crate::throttle::BandwidthThrottler;
use futures_util::StreamExt;
use memmap2::MmapMut;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use tempfile::NamedTempFile;
use tokio::io::AsyncWriteExt;
use tracing::{debug, trace, warn};
use url::Url;

/// Download state for tracking progress and resumption.
#[derive(Debug, Clone, Default)]
pub struct DownloadState {
    /// Bytes downloaded so far.
    pub downloaded: u64,
    /// Total size if known.
    pub total_size: Option<u64>,
    /// Whether resume is supported.
    pub resume_supported: bool,
    /// Temporary file path.
    pub temp_path: Option<std::path::PathBuf>,
}

/// Stream downloader with resume support.
pub struct StreamDownloader {
    client: HttpClient,
    throttler: BandwidthThrottler,
    config: Arc<DownloadConfig>,
}

impl std::fmt::Debug for StreamDownloader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamDownloader")
            .field("client", &self.client)
            .field("throttler", &self.throttler)
            .finish()
    }
}

impl StreamDownloader {
    /// Create a new stream downloader.
    #[must_use]
    pub fn new(client: HttpClient, throttler: BandwidthThrottler) -> Self {
        let config = Arc::clone(client.config());
        Self {
            client,
            throttler,
            config,
        }
    }

    /// Download a URL to a file with optional resume support.
    ///
    /// # Errors
    /// Returns error if download fails.
    pub async fn download(
        &self,
        url: &Url,
        dest: &Path,
        checksums: &[ExpectedChecksum],
        progress: &DownloadProgress,
    ) -> Result<DownloadedFile> {
        debug!(url = %url, dest = %dest.display(), "StreamDownloader::download starting");

        // Check for existing partial download
        let mut state = self.check_partial_download(dest);

        // Get content info
        debug!(url = %url, "getting content info (HEAD request)");
        let (total_size, resume_supported) = self.get_content_info(url).await?;
        debug!(url = %url, total_size = ?total_size, resume_supported, "got content info");

        state.total_size = total_size;
        state.resume_supported = resume_supported && self.config.resume_downloads;

        if let Some(size) = total_size {
            progress.set_total(size);
        }

        // Decide download strategy based on file size
        let use_mmap = total_size.is_some_and(|s| s >= self.config.mmap_threshold);

        debug!(url = %url, use_mmap, state_downloaded = state.downloaded, "choosing download strategy");

        let result = if use_mmap && state.resume_supported {
            debug!(url = %url, "using mmap download");
            self.download_mmap(url, dest, &state, checksums, progress)
                .await
        } else if state.downloaded > 0 && state.resume_supported {
            debug!(url = %url, "using resume download");
            self.download_resume(url, dest, &state, checksums, progress)
                .await
        } else {
            debug!(url = %url, "using streaming download");
            self.download_streaming(url, dest, checksums, progress)
                .await
        };

        debug!(url = %url, success = result.is_ok(), "StreamDownloader::download finished");
        result
    }

    /// Download using memory-mapped file (for large files).
    async fn download_mmap(
        &self,
        url: &Url,
        dest: &Path,
        state: &DownloadState,
        checksums: &[ExpectedChecksum],
        progress: &DownloadProgress,
    ) -> Result<DownloadedFile> {
        let total_size = state
            .total_size
            .ok_or_else(|| DownloadError::network("Cannot use mmap without known size"))?;

        debug!(url = %url, size = total_size, "downloading with mmap");

        // Create or open the destination file
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| DownloadError::io(parent, e))?;
        }

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(state.downloaded == 0)
            .open(dest)
            .map_err(|e| DownloadError::io(dest, e))?;

        // Set file size
        file.set_len(total_size)
            .map_err(|e| DownloadError::io(dest, e))?;

        // Create mmap
        // SAFETY: We own the file exclusively and control all access to it.
        // The file handle remains valid for the lifetime of the mmap.
        // No other process or thread accesses this file concurrently.
        #[allow(unsafe_code)]
        let mut mmap = unsafe { MmapMut::map_mut(&file) }
            .map_err(|e| DownloadError::io(dest, std::io::Error::other(e)))?;

        // Download with range request if resuming
        let start_offset = state.downloaded;
        let response = if start_offset > 0 {
            self.client.get_range(url, start_offset, None).await?
        } else {
            self.client.get(url).await?
        };

        let mut stream = response.bytes_stream();
        let mut position = start_offset as usize;
        let mut hasher = MultiHasher::for_checksums(checksums);

        // If resuming, we need to hash the already downloaded portion
        if start_offset > 0 {
            hasher.update(&mmap[..position]);
        }

        progress.update(start_offset);

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(DownloadError::from_reqwest)?;

            // Apply throttling
            self.throttler.acquire(chunk.len()).await;

            // Write to mmap
            let end = (position + chunk.len()).min(mmap.len());
            mmap[position..end].copy_from_slice(&chunk[..end - position]);
            hasher.update(&chunk);

            position = end;
            progress.update(position as u64);

            trace!(bytes = chunk.len(), position, "wrote chunk to mmap");
        }

        // Flush mmap
        mmap.flush().map_err(|e| DownloadError::io(dest, e))?;
        drop(mmap);

        let checksums_result = hasher.finalize();

        Ok(DownloadedFile {
            path: dest.to_path_buf(),
            size: position as u64,
            checksums: checksums_result,
            resumed: start_offset > 0,
        })
    }

    /// Download with resume support using Range headers.
    async fn download_resume(
        &self,
        url: &Url,
        dest: &Path,
        state: &DownloadState,
        checksums: &[ExpectedChecksum],
        progress: &DownloadProgress,
    ) -> Result<DownloadedFile> {
        debug!(url = %url, offset = state.downloaded, "resuming download");

        // Open existing file for appending
        let temp_path = state
            .temp_path
            .as_ref()
            .map_or_else(|| dest.with_extension("partial"), std::clone::Clone::clone);

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&temp_path)
            .map_err(|e| DownloadError::io(&temp_path, e))?;

        // Get Range response
        let response = self.client.get_range(url, state.downloaded, None).await?;

        let mut stream = response.bytes_stream();
        let mut downloaded = state.downloaded;
        let mut hasher = MultiHasher::for_checksums(checksums);

        // Hash already downloaded portion
        if state.downloaded > 0 {
            let existing =
                std::fs::File::open(&temp_path).map_err(|e| DownloadError::io(&temp_path, e))?;
            let mut reader = std::io::BufReader::with_capacity(128 * 1024, existing);
            let mut buf = [0u8; 128 * 1024];
            let mut to_read = state.downloaded as usize;

            while to_read > 0 {
                let read_len = to_read.min(buf.len());
                let n = std::io::Read::read(&mut reader, &mut buf[..read_len])
                    .map_err(|e| DownloadError::io(&temp_path, e))?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
                to_read -= n;
            }
        }

        progress.update(downloaded);

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(DownloadError::from_reqwest)?;

            self.throttler.acquire(chunk.len()).await;

            file.write_all(&chunk)
                .map_err(|e| DownloadError::io(&temp_path, e))?;
            hasher.update(&chunk);

            downloaded += chunk.len() as u64;
            progress.update(downloaded);
        }

        file.flush().map_err(|e| DownloadError::io(&temp_path, e))?;
        drop(file);

        // Move to final destination
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| DownloadError::io(parent, e))?;
        }
        std::fs::rename(&temp_path, dest).map_err(|e| DownloadError::io(dest, e))?;

        let checksums_result = hasher.finalize();

        Ok(DownloadedFile {
            path: dest.to_path_buf(),
            size: downloaded,
            checksums: checksums_result,
            resumed: state.downloaded > 0,
        })
    }

    /// Standard streaming download to temp file.
    async fn download_streaming(
        &self,
        url: &Url,
        dest: &Path,
        checksums: &[ExpectedChecksum],
        progress: &DownloadProgress,
    ) -> Result<DownloadedFile> {
        debug!(url = %url, "streaming download starting");

        // Create temp file in same directory for atomic move
        let parent = dest.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent).map_err(|e| DownloadError::io(parent, e))?;

        let temp_file = NamedTempFile::new_in(parent).map_err(|e| DownloadError::io(parent, e))?;
        let temp_path = temp_file.path().to_path_buf();

        let mut file = tokio::fs::File::from_std(
            temp_file
                .reopen()
                .map_err(|e| DownloadError::io(&temp_path, e))?,
        );

        debug!(url = %url, "sending GET request");
        let response = self.client.get(url).await?;
        debug!(url = %url, status = %response.status(), "got response");

        if let Some(size) = response.content_length() {
            debug!(url = %url, content_length = size, "got content length");
            progress.set_total(size);
        }

        let mut stream = response.bytes_stream();
        let mut downloaded: u64 = 0;
        let mut hasher = MultiHasher::for_checksums(checksums);
        let mut chunk_count = 0u64;

        debug!(url = %url, "starting to read chunks");
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(DownloadError::from_reqwest)?;
            chunk_count += 1;

            self.throttler.acquire(chunk.len()).await;

            file.write_all(&chunk)
                .await
                .map_err(|e| DownloadError::io(&temp_path, e))?;
            hasher.update(&chunk);

            downloaded += chunk.len() as u64;
            progress.update(downloaded);

            if chunk_count.is_multiple_of(100) {
                trace!(url = %url, chunks = chunk_count, bytes = downloaded, "download progress");
            }
        }

        debug!(url = %url, total_chunks = chunk_count, total_bytes = downloaded, "finished reading chunks");

        file.flush()
            .await
            .map_err(|e| DownloadError::io(&temp_path, e))?;
        drop(file);

        // Atomically move to destination
        debug!(url = %url, dest = %dest.display(), "moving temp file to destination");
        temp_file
            .persist(dest)
            .map_err(|e| DownloadError::io(dest, e.error))?;

        let checksums_result = hasher.finalize();

        debug!(url = %url, size = downloaded, "streaming download complete");

        Ok(DownloadedFile {
            path: dest.to_path_buf(),
            size: downloaded,
            checksums: checksums_result,
            resumed: false,
        })
    }

    /// Check for existing partial download.
    fn check_partial_download(&self, dest: &Path) -> DownloadState {
        let partial_path = dest.with_extension("partial");

        if let Ok(metadata) = std::fs::metadata(&partial_path) {
            return DownloadState {
                downloaded: metadata.len(),
                total_size: None,
                resume_supported: false,
                temp_path: Some(partial_path),
            };
        }

        DownloadState::default()
    }

    /// Get content length and resume support from HEAD request.
    async fn get_content_info(&self, url: &Url) -> Result<(Option<u64>, bool)> {
        // Skip HEAD request for codeload.github.com - it doesn't return content-length
        // and can cause HTTP/2 connection issues with many concurrent requests
        if let Some(host) = url.host_str()
            && host == "codeload.github.com"
        {
            debug!(url = %url, "skipping HEAD request for codeload.github.com");
            return Ok((None, false));
        }

        // Use a timeout for HEAD requests to avoid hanging
        let head_future = self.client.head(url);
        let timeout_duration = std::time::Duration::from_secs(5);

        match tokio::time::timeout(timeout_duration, head_future).await {
            Ok(Ok(response)) => {
                let size = response.content_length();
                let resume = response
                    .headers()
                    .get("accept-ranges")
                    .and_then(|v| v.to_str().ok())
                    .is_some_and(|v| v != "none");

                debug!(url = %url, size = ?size, resume, "HEAD request succeeded");
                Ok((size, resume))
            }
            Ok(Err(e)) => {
                // HEAD might not be supported, continue without info
                warn!(error = %e, "HEAD request failed, continuing without content info");
                Ok((None, false))
            }
            Err(_) => {
                // Timeout - continue without content info
                warn!(url = %url, "HEAD request timed out, continuing without content info");
                Ok((None, false))
            }
        }
    }
}

/// Result of a file download.
#[derive(Debug, Clone)]
pub struct DownloadedFile {
    /// Path to downloaded file.
    pub path: std::path::PathBuf,
    /// File size in bytes.
    pub size: u64,
    /// Computed checksums.
    pub checksums: crate::checksum::ComputedChecksums,
    /// Whether download was resumed.
    pub resumed: bool,
}

impl DownloadedFile {
    /// Verify checksums against expected values.
    ///
    /// # Errors
    /// Returns error if checksums don't match.
    pub fn verify(&self, expected: &[ExpectedChecksum], name: &str) -> Result<()> {
        self.checksums.verify(expected, name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_state_default() {
        let state = DownloadState::default();
        assert_eq!(state.downloaded, 0);
        assert!(state.total_size.is_none());
        assert!(!state.resume_supported);
    }

    #[test]
    fn downloaded_file_debug() {
        let checksums = MultiHasher::new().finalize();
        let file = DownloadedFile {
            path: std::path::PathBuf::from("/tmp/test"),
            size: 1024,
            checksums,
            resumed: false,
        };

        let debug = format!("{file:?}");
        assert!(debug.contains("DownloadedFile"));
    }
}
