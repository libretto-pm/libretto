//! Cache statistics tracking.

use parking_lot::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Atomic cache statistics.
#[derive(Debug, Default)]
pub struct CacheStats {
    /// Number of cache hits.
    hits: AtomicU64,
    /// Number of cache misses.
    misses: AtomicU64,
    /// Number of L1 (memory) hits.
    l1_hits: AtomicU64,
    /// Number of L2 (disk) hits.
    l2_hits: AtomicU64,
    /// Total bytes read from cache.
    bytes_read: AtomicU64,
    /// Total bytes written to cache.
    bytes_written: AtomicU64,
    /// Number of entries evicted.
    evictions: AtomicU64,
    /// Number of expired entries removed.
    expirations: AtomicU64,
    /// Number of compression operations.
    compressions: AtomicU64,
    /// Bytes saved by compression.
    compression_savings: AtomicU64,
    /// Total lookup time in microseconds.
    total_lookup_time_us: AtomicU64,
    /// Number of lookups for average calculation.
    lookup_count: AtomicU64,
    /// Number of bloom filter true negatives (avoided disk lookups).
    bloom_true_negatives: AtomicU64,
    /// Number of bloom filter false positives.
    bloom_false_positives: AtomicU64,
    /// Start time for uptime calculation.
    start_time: RwLock<Option<Instant>>,
}

impl CacheStats {
    /// Create new stats tracker.
    #[must_use]
    pub fn new() -> Self {
        Self {
            start_time: RwLock::new(Some(Instant::now())),
            ..Default::default()
        }
    }

    /// Record a cache hit.
    pub fn record_hit(&self, is_l1: bool) {
        self.hits.fetch_add(1, Ordering::Relaxed);
        if is_l1 {
            self.l1_hits.fetch_add(1, Ordering::Relaxed);
        } else {
            self.l2_hits.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Record a cache miss.
    pub fn record_miss(&self) {
        self.misses.fetch_add(1, Ordering::Relaxed);
    }

    /// Record bytes read.
    pub fn record_bytes_read(&self, bytes: u64) {
        self.bytes_read.fetch_add(bytes, Ordering::Relaxed);
    }

    /// Record bytes written.
    pub fn record_bytes_written(&self, bytes: u64) {
        self.bytes_written.fetch_add(bytes, Ordering::Relaxed);
    }

    /// Record an eviction.
    pub fn record_eviction(&self) {
        self.evictions.fetch_add(1, Ordering::Relaxed);
    }

    /// Record an expiration.
    pub fn record_expiration(&self) {
        self.expirations.fetch_add(1, Ordering::Relaxed);
    }

    /// Record compression operation.
    pub fn record_compression(&self, original_size: u64, compressed_size: u64) {
        self.compressions.fetch_add(1, Ordering::Relaxed);
        if original_size > compressed_size {
            self.compression_savings
                .fetch_add(original_size - compressed_size, Ordering::Relaxed);
        }
    }

    /// Record lookup timing.
    pub fn record_lookup_time(&self, duration: Duration) {
        self.total_lookup_time_us
            .fetch_add(duration.as_micros() as u64, Ordering::Relaxed);
        self.lookup_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Record bloom filter true negative (correctly avoided disk lookup).
    pub fn record_bloom_true_negative(&self) {
        self.bloom_true_negatives.fetch_add(1, Ordering::Relaxed);
    }

    /// Record bloom filter false positive (checked disk but wasn't there).
    pub fn record_bloom_false_positive(&self) {
        self.bloom_false_positives.fetch_add(1, Ordering::Relaxed);
    }

    /// Get current snapshot of stats.
    #[must_use]
    pub fn snapshot(&self) -> CacheStatsSnapshot {
        let hits = self.hits.load(Ordering::Relaxed);
        let misses = self.misses.load(Ordering::Relaxed);
        let total = hits + misses;
        let hit_rate = if total > 0 {
            hits as f64 / total as f64
        } else {
            0.0
        };

        let lookup_count = self.lookup_count.load(Ordering::Relaxed);
        let total_lookup_time_us = self.total_lookup_time_us.load(Ordering::Relaxed);
        let avg_lookup_time = if lookup_count > 0 {
            Duration::from_micros(total_lookup_time_us / lookup_count)
        } else {
            Duration::ZERO
        };

        let bloom_tn = self.bloom_true_negatives.load(Ordering::Relaxed);
        let bloom_fp = self.bloom_false_positives.load(Ordering::Relaxed);
        let bloom_total = bloom_tn + bloom_fp + misses;
        let bloom_effectiveness = if bloom_total > 0 {
            bloom_tn as f64 / bloom_total as f64
        } else {
            0.0
        };

        let uptime = self
            .start_time
            .read()
            .map(|t| t.elapsed())
            .unwrap_or_default();

        CacheStatsSnapshot {
            hits,
            misses,
            hit_rate,
            l1_hits: self.l1_hits.load(Ordering::Relaxed),
            l2_hits: self.l2_hits.load(Ordering::Relaxed),
            bytes_read: self.bytes_read.load(Ordering::Relaxed),
            bytes_written: self.bytes_written.load(Ordering::Relaxed),
            evictions: self.evictions.load(Ordering::Relaxed),
            expirations: self.expirations.load(Ordering::Relaxed),
            compressions: self.compressions.load(Ordering::Relaxed),
            compression_savings: self.compression_savings.load(Ordering::Relaxed),
            avg_lookup_time,
            bloom_true_negatives: bloom_tn,
            bloom_false_positives: bloom_fp,
            bloom_effectiveness,
            uptime,
        }
    }

    /// Reset all statistics.
    pub fn reset(&self) {
        self.hits.store(0, Ordering::Relaxed);
        self.misses.store(0, Ordering::Relaxed);
        self.l1_hits.store(0, Ordering::Relaxed);
        self.l2_hits.store(0, Ordering::Relaxed);
        self.bytes_read.store(0, Ordering::Relaxed);
        self.bytes_written.store(0, Ordering::Relaxed);
        self.evictions.store(0, Ordering::Relaxed);
        self.expirations.store(0, Ordering::Relaxed);
        self.compressions.store(0, Ordering::Relaxed);
        self.compression_savings.store(0, Ordering::Relaxed);
        self.total_lookup_time_us.store(0, Ordering::Relaxed);
        self.lookup_count.store(0, Ordering::Relaxed);
        self.bloom_true_negatives.store(0, Ordering::Relaxed);
        self.bloom_false_positives.store(0, Ordering::Relaxed);
        *self.start_time.write() = Some(Instant::now());
    }
}

/// Snapshot of cache statistics.
#[derive(Debug, Clone)]
pub struct CacheStatsSnapshot {
    /// Number of cache hits.
    pub hits: u64,
    /// Number of cache misses.
    pub misses: u64,
    /// Hit rate (0.0 - 1.0).
    pub hit_rate: f64,
    /// Number of L1 (memory) hits.
    pub l1_hits: u64,
    /// Number of L2 (disk) hits.
    pub l2_hits: u64,
    /// Total bytes read from cache.
    pub bytes_read: u64,
    /// Total bytes written to cache.
    pub bytes_written: u64,
    /// Number of entries evicted.
    pub evictions: u64,
    /// Number of expired entries removed.
    pub expirations: u64,
    /// Number of compression operations.
    pub compressions: u64,
    /// Bytes saved by compression.
    pub compression_savings: u64,
    /// Average lookup time.
    pub avg_lookup_time: Duration,
    /// Bloom filter true negatives.
    pub bloom_true_negatives: u64,
    /// Bloom filter false positives.
    pub bloom_false_positives: u64,
    /// Bloom filter effectiveness (0.0 - 1.0).
    pub bloom_effectiveness: f64,
    /// Cache uptime.
    pub uptime: Duration,
}

impl CacheStatsSnapshot {
    /// Format statistics as a human-readable string.
    #[must_use]
    pub fn format_summary(&self) -> String {
        let mut lines = Vec::new();

        lines.push(format!(
            "Hit Rate: {:.1}% ({} hits, {} misses)",
            self.hit_rate * 100.0,
            self.hits,
            self.misses
        ));
        lines.push(format!(
            "  L1 (memory): {} hits, L2 (disk): {} hits",
            self.l1_hits, self.l2_hits
        ));
        lines.push(format!(
            "Bytes: {} read, {} written",
            format_bytes(self.bytes_read),
            format_bytes(self.bytes_written)
        ));
        lines.push(format!(
            "Evictions: {}, Expirations: {}",
            self.evictions, self.expirations
        ));

        if self.compressions > 0 {
            lines.push(format!(
                "Compression: {} ops, {} saved",
                self.compressions,
                format_bytes(self.compression_savings)
            ));
        }

        lines.push(format!(
            "Avg Lookup: {:.2}ms",
            self.avg_lookup_time.as_secs_f64() * 1000.0
        ));

        if self.bloom_true_negatives > 0 || self.bloom_false_positives > 0 {
            lines.push(format!(
                "Bloom Filter: {:.1}% effective ({} TN, {} FP)",
                self.bloom_effectiveness * 100.0,
                self.bloom_true_negatives,
                self.bloom_false_positives
            ));
        }

        lines.push(format!("Uptime: {:.1}s", self.uptime.as_secs_f64()));

        lines.join("\n")
    }
}

fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

/// Per-entry size tracking for cache size management.
#[derive(Debug, Default)]
pub struct SizeTracker {
    /// Total size in bytes.
    total: AtomicU64,
    /// Number of entries.
    count: AtomicU64,
}

impl SizeTracker {
    /// Create new size tracker.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add entry size.
    pub fn add(&self, size: u64) {
        self.total.fetch_add(size, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);
    }

    /// Remove entry size.
    pub fn remove(&self, size: u64) {
        self.total
            .fetch_sub(size.min(self.total()), Ordering::Relaxed);
        let current_count = self.count.load(Ordering::Relaxed);
        if current_count > 0 {
            self.count.fetch_sub(1, Ordering::Relaxed);
        }
    }

    /// Get total size.
    #[must_use]
    pub fn total(&self) -> u64 {
        self.total.load(Ordering::Relaxed)
    }

    /// Get entry count.
    #[must_use]
    pub fn count(&self) -> u64 {
        self.count.load(Ordering::Relaxed)
    }

    /// Reset tracker.
    pub fn reset(&self) {
        self.total.store(0, Ordering::Relaxed);
        self.count.store(0, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_basic() {
        let stats = CacheStats::new();

        stats.record_hit(true);
        stats.record_hit(false);
        stats.record_miss();

        let snap = stats.snapshot();
        assert_eq!(snap.hits, 2);
        assert_eq!(snap.misses, 1);
        assert_eq!(snap.l1_hits, 1);
        assert_eq!(snap.l2_hits, 1);
        assert!((snap.hit_rate - 0.666).abs() < 0.01);
    }

    #[test]
    fn stats_reset() {
        let stats = CacheStats::new();

        stats.record_hit(true);
        stats.reset();

        let snap = stats.snapshot();
        assert_eq!(snap.hits, 0);
    }

    #[test]
    fn size_tracker() {
        let tracker = SizeTracker::new();

        tracker.add(100);
        tracker.add(200);
        assert_eq!(tracker.total(), 300);
        assert_eq!(tracker.count(), 2);

        tracker.remove(100);
        assert_eq!(tracker.total(), 200);
        assert_eq!(tracker.count(), 1);
    }
}
