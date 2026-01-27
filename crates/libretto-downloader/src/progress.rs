//! Progress tracking with multi-progress bars.
//!
//! Provides a unified progress display for concurrent downloads.

use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

/// Shared progress tracker for multiple concurrent downloads.
#[derive(Clone)]
pub struct ProgressTracker {
    inner: Arc<ProgressTrackerInner>,
}

struct ProgressTrackerInner {
    multi: MultiProgress,
    main_bar: ProgressBar,
    active_bars: Mutex<HashMap<String, ProgressBar>>,
    total_downloads: AtomicUsize,
    completed_downloads: AtomicUsize,
    total_bytes: AtomicU64,
    downloaded_bytes: AtomicU64,
    start_time: Instant,
    enabled: bool,
}

impl std::fmt::Debug for ProgressTracker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProgressTracker")
            .field("enabled", &self.inner.enabled)
            .field(
                "total_downloads",
                &self.inner.total_downloads.load(Ordering::Relaxed),
            )
            .field(
                "completed_downloads",
                &self.inner.completed_downloads.load(Ordering::Relaxed),
            )
            .finish()
    }
}

impl ProgressTracker {
    /// Create a new progress tracker.
    #[must_use]
    pub fn new(enabled: bool) -> Self {
        let multi = MultiProgress::new();
        if !enabled {
            multi.set_draw_target(ProgressDrawTarget::hidden());
        }

        let main_style = ProgressStyle::default_bar()
            .template(
                "{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} packages ({percent}%) {msg}",
            )
            .unwrap_or_else(|_| ProgressStyle::default_bar())
            .progress_chars("█▓░");

        let main_bar = multi.add(ProgressBar::new(0));
        main_bar.set_style(main_style);
        main_bar.set_message("downloading...");

        Self {
            inner: Arc::new(ProgressTrackerInner {
                multi,
                main_bar,
                active_bars: Mutex::new(HashMap::new()),
                total_downloads: AtomicUsize::new(0),
                completed_downloads: AtomicUsize::new(0),
                total_bytes: AtomicU64::new(0),
                downloaded_bytes: AtomicU64::new(0),
                start_time: Instant::now(),
                enabled,
            }),
        }
    }

    /// Create a disabled progress tracker.
    #[must_use]
    pub fn disabled() -> Self {
        Self::new(false)
    }

    /// Check if progress display is enabled.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.inner.enabled
    }

    /// Set the total number of downloads.
    pub fn set_total(&self, total: usize) {
        self.inner.total_downloads.store(total, Ordering::Relaxed);
        self.inner.main_bar.set_length(total as u64);
    }

    /// Start tracking a new download.
    pub fn start_download(
        &self,
        id: &str,
        name: &str,
        total_size: Option<u64>,
    ) -> DownloadProgress {
        if !self.inner.enabled {
            return DownloadProgress::disabled(id.to_string());
        }

        let style = ProgressStyle::default_bar()
            .template("  {spinner:.dim} {msg:<30} [{bar:25.green/dim}] {bytes:>10}/{total_bytes:<10} {bytes_per_sec:>12}")
            .unwrap_or_else(|_| ProgressStyle::default_bar())
            .progress_chars("━╸─");

        let pb = self
            .inner
            .multi
            .add(ProgressBar::new(total_size.unwrap_or(0)));
        pb.set_style(style);
        pb.set_message(truncate_name(name, 30));

        if let Some(size) = total_size {
            self.inner.total_bytes.fetch_add(size, Ordering::Relaxed);
        }

        self.inner
            .active_bars
            .lock()
            .insert(id.to_string(), pb.clone());

        DownloadProgress {
            id: id.to_string(),
            bar: Some(pb),
            tracker: self.clone(),
        }
    }

    /// Mark a download as complete.
    pub fn complete_download(&self, id: &str, bytes: u64) {
        self.inner
            .completed_downloads
            .fetch_add(1, Ordering::Relaxed);
        self.inner
            .downloaded_bytes
            .fetch_add(bytes, Ordering::Relaxed);
        self.inner.main_bar.inc(1);

        if let Some(pb) = self.inner.active_bars.lock().remove(id) {
            pb.finish_and_clear();
        }

        self.update_main_message();
    }

    /// Mark a download as failed.
    pub fn fail_download(&self, id: &str, _error: &str) {
        if let Some(pb) = self.inner.active_bars.lock().remove(id) {
            pb.abandon_with_message("failed");
        }
    }

    /// Finish all progress tracking.
    pub fn finish(&self) {
        let elapsed = self.inner.start_time.elapsed();
        let total_bytes = self.inner.downloaded_bytes.load(Ordering::Relaxed);
        let completed = self.inner.completed_downloads.load(Ordering::Relaxed);

        let rate = if elapsed.as_secs_f64() > 0.0 {
            total_bytes as f64 / elapsed.as_secs_f64()
        } else {
            0.0
        };

        self.inner.main_bar.finish_with_message(format!(
            "done! {} packages ({}) in {:.1}s ({}/s)",
            completed,
            format_bytes(total_bytes),
            elapsed.as_secs_f64(),
            format_bytes(rate as u64)
        ));

        // Clear any remaining bars
        for pb in self.inner.active_bars.lock().values() {
            pb.finish_and_clear();
        }
    }

    /// Get current statistics.
    #[must_use]
    pub fn stats(&self) -> ProgressStats {
        ProgressStats {
            total_downloads: self.inner.total_downloads.load(Ordering::Relaxed),
            completed_downloads: self.inner.completed_downloads.load(Ordering::Relaxed),
            total_bytes: self.inner.total_bytes.load(Ordering::Relaxed),
            downloaded_bytes: self.inner.downloaded_bytes.load(Ordering::Relaxed),
            elapsed: self.inner.start_time.elapsed(),
        }
    }

    fn update_main_message(&self) {
        let stats = self.stats();
        let rate = if stats.elapsed.as_secs_f64() > 0.0 {
            stats.downloaded_bytes as f64 / stats.elapsed.as_secs_f64()
        } else {
            0.0
        };

        self.inner
            .main_bar
            .set_message(format!("{}/s", format_bytes(rate as u64)));
    }
}

impl Default for ProgressTracker {
    fn default() -> Self {
        Self::new(true)
    }
}

/// Progress handle for a single download.
pub struct DownloadProgress {
    id: String,
    bar: Option<ProgressBar>,
    tracker: ProgressTracker,
}

impl std::fmt::Debug for DownloadProgress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DownloadProgress")
            .field("id", &self.id)
            .field("has_bar", &self.bar.is_some())
            .finish()
    }
}

impl DownloadProgress {
    /// Create a disabled progress handle.
    fn disabled(id: String) -> Self {
        Self {
            id,
            bar: None,
            tracker: ProgressTracker::disabled(),
        }
    }

    /// Update the download progress with bytes downloaded.
    pub fn update(&self, bytes: u64) {
        if let Some(ref pb) = self.bar {
            pb.set_position(bytes);
        }
    }

    /// Increment progress by bytes.
    pub fn inc(&self, bytes: u64) {
        if let Some(ref pb) = self.bar {
            pb.inc(bytes);
        }
    }

    /// Set the total size (if not known at start).
    pub fn set_total(&self, total: u64) {
        if let Some(ref pb) = self.bar {
            pb.set_length(total);
        }
    }

    /// Set a status message.
    pub fn set_message(&self, msg: &str) {
        if let Some(ref pb) = self.bar {
            pb.set_message(truncate_name(msg, 30));
        }
    }

    /// Mark as complete.
    pub fn complete(self, bytes: u64) {
        self.tracker.complete_download(&self.id, bytes);
    }

    /// Mark as failed.
    pub fn fail(self, error: &str) {
        self.tracker.fail_download(&self.id, error);
    }
}

/// Progress statistics.
#[derive(Debug, Clone)]
pub struct ProgressStats {
    /// Total number of downloads.
    pub total_downloads: usize,
    /// Completed downloads.
    pub completed_downloads: usize,
    /// Total bytes to download.
    pub total_bytes: u64,
    /// Bytes downloaded so far.
    pub downloaded_bytes: u64,
    /// Time elapsed.
    pub elapsed: std::time::Duration,
}

impl ProgressStats {
    /// Calculate download rate in bytes per second.
    #[must_use]
    pub fn rate(&self) -> f64 {
        if self.elapsed.as_secs_f64() > 0.0 {
            self.downloaded_bytes as f64 / self.elapsed.as_secs_f64()
        } else {
            0.0
        }
    }

    /// Calculate completion percentage.
    #[must_use]
    pub fn percent(&self) -> f64 {
        if self.total_downloads > 0 {
            (self.completed_downloads as f64 / self.total_downloads as f64) * 100.0
        } else {
            0.0
        }
    }
}

/// Truncate a string to fit within a given width.
fn truncate_name(name: &str, max_len: usize) -> String {
    if name.len() <= max_len {
        name.to_string()
    } else {
        format!("{}...", &name[..max_len - 3])
    }
}

/// Format bytes as human-readable string.
fn format_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit_idx = 0;

    while size >= 1024.0 && unit_idx < UNITS.len() - 1 {
        size /= 1024.0;
        unit_idx += 1;
    }

    if unit_idx == 0 {
        format!("{size:.0} {}", UNITS[unit_idx])
    } else {
        format!("{size:.1} {}", UNITS[unit_idx])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_bytes_test() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(500), "500 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(1024 * 1024), "1.0 MB");
        assert_eq!(format_bytes(1024 * 1024 * 1024), "1.0 GB");
    }

    #[test]
    fn truncate_name_test() {
        assert_eq!(truncate_name("short", 10), "short");
        assert_eq!(truncate_name("verylongpackagename", 10), "verylon...");
    }

    #[test]
    fn progress_tracker_disabled() {
        let tracker = ProgressTracker::disabled();
        assert!(!tracker.is_enabled());
        tracker.set_total(10);
        let progress = tracker.start_download("test", "test-package", Some(1000));
        progress.update(500);
        progress.complete(1000);
        tracker.finish();
    }

    #[test]
    fn progress_stats() {
        let tracker = ProgressTracker::new(false);
        tracker.set_total(10);
        let stats = tracker.stats();
        assert_eq!(stats.total_downloads, 10);
        assert_eq!(stats.completed_downloads, 0);
    }
}
