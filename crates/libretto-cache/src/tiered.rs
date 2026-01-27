//! Multi-tier cache combining L1 (memory) and L2 (disk) caches.
//!
//! This is the main cache interface that provides:
//! - Fast L1 memory lookups with true LRU eviction (moka)
//! - Persistent L2 disk storage with content-addressing
//! - Bloom filter for fast "not cached" checks with persistence
//! - Background warming and garbage collection
//! - Statistics tracking
//! - Pattern-based cache clearing

use crate::bloom::ConcurrentBloomFilter;
use crate::compression;
use crate::config::{CacheConfig, CacheEntryType};
use crate::l1::{L1Cache, L1Entry};
use crate::l2::L2Cache;
use crate::stats::{CacheStats, CacheStatsSnapshot, SizeTracker};
use bytes::Bytes;
use libretto_core::{ContentHash, Error, Result};
use libretto_platform::Platform;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Notify;
use tracing::{debug, info, warn};

/// Cache clearing pattern for selective clearing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClearPattern {
    /// Clear all cached packages.
    Packages,
    /// Clear all metadata.
    Metadata,
    /// Clear all repository data.
    Repositories,
    /// Clear all VCS clones.
    Vcs,
    /// Clear all dependency graphs.
    Graphs,
    /// Clear all autoloaders.
    Autoloaders,
    /// Clear everything.
    All,
}

impl ClearPattern {
    /// Convert pattern to entry types to clear.
    #[must_use]
    pub fn entry_types(self) -> Vec<CacheEntryType> {
        match self {
            Self::Packages => vec![CacheEntryType::Package],
            Self::Metadata => vec![CacheEntryType::Metadata],
            Self::Repositories => vec![CacheEntryType::Repository],
            Self::Vcs => vec![CacheEntryType::VcsClone],
            Self::Graphs => vec![CacheEntryType::DependencyGraph],
            Self::Autoloaders => vec![CacheEntryType::Autoloader],
            Self::All => vec![
                CacheEntryType::Package,
                CacheEntryType::Metadata,
                CacheEntryType::Repository,
                CacheEntryType::VcsClone,
                CacheEntryType::DependencyGraph,
                CacheEntryType::Autoloader,
            ],
        }
    }

    /// Parse from string.
    #[must_use]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "packages" | "pkg" => Some(Self::Packages),
            "metadata" | "meta" => Some(Self::Metadata),
            "repositories" | "repos" | "repo" => Some(Self::Repositories),
            "vcs" | "git" => Some(Self::Vcs),
            "graphs" | "graph" => Some(Self::Graphs),
            "autoloaders" | "autoload" => Some(Self::Autoloaders),
            "all" | "*" => Some(Self::All),
            _ => None,
        }
    }
}

/// Multi-tier content-addressable cache.
pub struct TieredCache {
    /// L1 in-memory cache.
    l1: L1Cache,
    /// L2 disk-based cache.
    l2: L2Cache,
    /// Bloom filter for fast negative lookups.
    bloom: Option<ConcurrentBloomFilter>,
    /// Cache root directory.
    root: PathBuf,
    /// Configuration.
    config: CacheConfig,
    /// Statistics.
    stats: Arc<CacheStats>,
    /// Size tracker.
    size_tracker: SizeTracker,
    /// Whether cache is warming.
    warming: AtomicBool,
    /// Shutdown signal.
    shutdown: Arc<Notify>,
}

impl std::fmt::Debug for TieredCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TieredCache")
            .field("l1", &self.l1)
            .field("l2", &self.l2)
            .field("root", &self.root)
            .field("warming", &self.warming.load(Ordering::Relaxed))
            .finish()
    }
}

impl TieredCache {
    /// Create cache at default platform location.
    ///
    /// # Errors
    /// Returns error if cache cannot be created.
    pub fn new() -> Result<Self> {
        Self::with_config(CacheConfig::default())
    }

    /// Create cache with custom configuration.
    ///
    /// # Errors
    /// Returns error if cache cannot be created.
    pub fn with_config(config: CacheConfig) -> Result<Self> {
        let root = config
            .root
            .clone()
            .unwrap_or_else(|| Platform::current().cache_dir.clone());

        Self::at_path(root, config)
    }

    /// Create cache at specific path.
    ///
    /// # Errors
    /// Returns error if cache cannot be created.
    pub fn at_path(root: PathBuf, config: CacheConfig) -> Result<Self> {
        let l1 = L1Cache::new(config.l1_size_limit, Some(config.default_ttl));
        let l2 = L2Cache::open(root.clone(), config.clone())?;

        // Try to load persisted bloom filter, or create new
        let bloom = if config.bloom_filter_enabled {
            let bloom_path = root.join("bloom.bin");
            let bloom = ConcurrentBloomFilter::load_or_create(
                &bloom_path,
                config.bloom_filter_capacity,
                config.bloom_filter_fp_rate,
            )
            .unwrap_or_else(|_| {
                ConcurrentBloomFilter::new(
                    config.bloom_filter_capacity,
                    config.bloom_filter_fp_rate,
                )
            });
            Some(bloom)
        } else {
            None
        };

        let cache = Self {
            l1,
            l2,
            bloom,
            root,
            config,
            stats: Arc::new(CacheStats::new()),
            size_tracker: SizeTracker::new(),
            warming: AtomicBool::new(false),
            shutdown: Arc::new(Notify::new()),
        };

        // If bloom filter was newly created, populate from index
        if let Some(ref bloom) = cache.bloom {
            if bloom.is_empty() && cache.l2.len() > 0 {
                cache.rebuild_bloom_filter();
            }
        }

        Ok(cache)
    }

    /// Get the cache root directory.
    #[must_use]
    pub fn root(&self) -> &PathBuf {
        &self.root
    }

    /// Get data by content hash.
    ///
    /// Checks L1 first, then L2, promoting to L1 on L2 hit.
    ///
    /// # Errors
    /// Returns error if retrieval fails.
    pub fn get(&self, hash: &ContentHash) -> Result<Option<Vec<u8>>> {
        let start = Instant::now();
        let key = hash.to_hex();

        // Check bloom filter first for fast negative
        if let Some(bloom) = &self.bloom {
            if !bloom.may_contain(&key) {
                self.stats.record_miss();
                self.stats.record_bloom_true_negative();
                self.stats.record_lookup_time(start.elapsed());
                return Ok(None);
            }
        }

        // Check L1 (memory)
        if let Some(entry) = self.l1.get(&key) {
            self.stats.record_hit(true);
            self.stats.record_bytes_read(entry.original_size);
            self.stats.record_lookup_time(start.elapsed());

            // Decompress if needed
            let data = if entry.compressed {
                if let Some(compressed) = compression::strip_magic(&entry.data) {
                    compression::decompress_with_hint(compressed, entry.original_size as usize)
                        .map_err(|e| Error::Cache(format!("decompression failed: {e}")))?
                } else {
                    entry.data.to_vec()
                }
            } else {
                entry.data.to_vec()
            };

            return Ok(Some(data));
        }

        // Check L2 (disk)
        if let Some((raw_data, index_entry)) = self.l2.get_raw(hash)? {
            self.stats.record_hit(false);
            self.stats.record_bytes_read(index_entry.original_size);

            // Promote to L1
            let l1_entry = L1Entry::new(
                Bytes::from(raw_data.clone()),
                index_entry.original_size,
                index_entry.compressed,
                *hash.as_bytes(),
            );
            self.l1.insert(key, l1_entry);

            // Decompress for return
            let data = if index_entry.compressed {
                if let Some(compressed) = compression::strip_magic(&raw_data) {
                    compression::decompress_with_hint(
                        compressed,
                        index_entry.original_size as usize,
                    )
                    .map_err(|e| Error::Cache(format!("decompression failed: {e}")))?
                } else {
                    raw_data
                }
            } else {
                raw_data
            };

            self.stats.record_lookup_time(start.elapsed());
            return Ok(Some(data));
        }

        // Bloom filter false positive
        if self.bloom.is_some() {
            self.stats.record_bloom_false_positive();
        }

        self.stats.record_miss();
        self.stats.record_lookup_time(start.elapsed());
        Ok(None)
    }

    /// Store data in cache.
    ///
    /// # Errors
    /// Returns error if storage fails.
    pub fn put(
        &self,
        data: &[u8],
        entry_type: CacheEntryType,
        ttl: Option<Duration>,
        metadata: Option<String>,
    ) -> Result<ContentHash> {
        let hash = ContentHash::from_bytes(data);
        let key = hash.to_hex();

        // Store in L2 (persistent)
        self.l2.put(data, entry_type, ttl, metadata)?;

        // Update bloom filter
        if let Some(bloom) = &self.bloom {
            bloom.insert(&key);
        }

        // Store in L1 (memory)
        let (l1_data, compressed) =
            if self.config.compression_enabled && compression::should_compress(data) {
                let compressed = compression::compress(data, self.config.compression_level)
                    .map_err(|e| Error::Cache(format!("compression failed: {e}")))?;

                if compressed.len() < data.len() {
                    (compression::with_magic(compressed), true)
                } else {
                    (data.to_vec(), false)
                }
            } else {
                (data.to_vec(), false)
            };

        let l1_entry = L1Entry::new(
            Bytes::from(l1_data),
            data.len() as u64,
            compressed,
            *hash.as_bytes(),
        );
        self.l1.insert(key, l1_entry);

        self.stats.record_bytes_written(data.len() as u64);
        self.size_tracker.add(data.len() as u64);

        Ok(hash)
    }

    /// Store data with known hash.
    ///
    /// # Errors
    /// Returns error if storage fails.
    pub fn put_with_hash(
        &self,
        hash: ContentHash,
        data: &[u8],
        entry_type: CacheEntryType,
        ttl: Option<Duration>,
        metadata: Option<String>,
    ) -> Result<()> {
        let key = hash.to_hex();

        self.l2
            .put_with_hash(hash, data, entry_type, ttl, metadata)?;

        if let Some(bloom) = &self.bloom {
            bloom.insert(&key);
        }

        let (l1_data, compressed) =
            if self.config.compression_enabled && compression::should_compress(data) {
                let compressed = compression::compress(data, self.config.compression_level)
                    .map_err(|e| Error::Cache(format!("compression failed: {e}")))?;

                if compressed.len() < data.len() {
                    (compression::with_magic(compressed), true)
                } else {
                    (data.to_vec(), false)
                }
            } else {
                (data.to_vec(), false)
            };

        let l1_entry = L1Entry::new(
            Bytes::from(l1_data),
            data.len() as u64,
            compressed,
            *hash.as_bytes(),
        );
        self.l1.insert(key, l1_entry);

        self.stats.record_bytes_written(data.len() as u64);
        Ok(())
    }

    /// Check if hash exists in cache.
    #[must_use]
    pub fn contains(&self, hash: &ContentHash) -> bool {
        let key = hash.to_hex();

        // Fast bloom check
        if let Some(bloom) = &self.bloom {
            if !bloom.may_contain(&key) {
                self.stats.record_bloom_true_negative();
                return false;
            }
        }

        // Check L1 then L2
        let exists = self.l1.contains(&key) || self.l2.contains(hash);

        // Record bloom false positive if bloom said maybe but it wasn't there
        if !exists && self.bloom.is_some() {
            self.stats.record_bloom_false_positive();
        }

        exists
    }

    /// Remove entry by hash.
    ///
    /// # Errors
    /// Returns error if removal fails.
    pub fn remove(&self, hash: &ContentHash) -> Result<bool> {
        let key = hash.to_hex();

        self.l1.remove(&key);
        let removed = self.l2.remove(hash)?;

        // Note: Can't remove from bloom filter, need to rebuild periodically
        if removed {
            self.stats.record_eviction();
        }

        Ok(removed)
    }

    /// Clear cache by entry type.
    ///
    /// # Errors
    /// Returns error if clearing fails.
    pub fn clear_by_type(&self, entry_type: CacheEntryType) -> Result<usize> {
        // Get keys to remove from L1
        let keys = self.l2.keys_by_type(entry_type);
        for key in &keys {
            self.l1.remove(key);
        }

        let removed = self.l2.clear_by_type(entry_type)?;

        // Rebuild bloom filter after mass removal
        if self.bloom.is_some() && removed > 100 {
            self.rebuild_bloom_filter();
        }

        Ok(removed)
    }

    /// Clear cache by pattern (packages, metadata, repos, all, etc.).
    ///
    /// # Errors
    /// Returns error if clearing fails.
    pub fn clear_by_pattern(&self, pattern: ClearPattern) -> Result<ClearResult> {
        let start = Instant::now();
        let mut total_removed = 0;
        let mut removed_by_type = Vec::new();

        for entry_type in pattern.entry_types() {
            let removed = self.clear_by_type(entry_type)?;
            removed_by_type.push((entry_type, removed));
            total_removed += removed;
        }

        // Rebuild bloom filter after clearing
        if self.bloom.is_some() && total_removed > 0 {
            self.rebuild_bloom_filter();
            self.save_bloom_filter();
        }

        Ok(ClearResult {
            pattern,
            total_removed,
            removed_by_type,
            duration: start.elapsed(),
        })
    }

    /// Clear all caches.
    ///
    /// # Errors
    /// Returns error if clearing fails.
    pub fn clear(&self) -> Result<()> {
        self.l1.clear();
        self.l2.clear()?;

        if let Some(bloom) = &self.bloom {
            bloom.clear();
        }

        self.stats.reset();
        self.size_tracker.reset();

        // Remove bloom filter file
        let bloom_path = self.root.join("bloom.bin");
        let _ = std::fs::remove_file(bloom_path);

        info!("cache cleared");
        Ok(())
    }

    /// Run garbage collection (remove expired entries).
    ///
    /// # Errors
    /// Returns error if GC fails.
    pub fn gc(&self) -> Result<GcResult> {
        let start = Instant::now();

        // Remove expired entries from L2
        let expired = self.l2.remove_expired()?;

        // Evict if over disk limit
        let disk_usage = self.l2.disk_usage();
        let evicted = if disk_usage > self.config.l2_size_limit {
            let to_free = disk_usage - self.config.l2_size_limit;
            self.l2.evict_lru(to_free)?
        } else {
            0
        };

        // Rebuild bloom filter if we removed many entries
        if self.bloom.is_some() && (expired + evicted) > 100 {
            self.rebuild_bloom_filter();
            self.save_bloom_filter();
        }

        // Run L1 pending tasks (moka maintenance)
        self.l1.run_pending_tasks();

        for _ in 0..(expired + evicted) {
            self.stats.record_expiration();
        }

        let result = GcResult {
            expired_removed: expired,
            lru_evicted: evicted,
            duration: start.elapsed(),
        };

        debug!(?result, "garbage collection complete");
        Ok(result)
    }

    /// Warm L1 cache from L2 (background operation).
    pub fn warm(&self) {
        if self.warming.swap(true, Ordering::SeqCst) {
            // Already warming
            return;
        }

        let entries = self.l2.entries();
        let max_entries = self.config.max_warm_entries;
        let mut warmed = 0;

        // Sort by access time (most recent first)
        let mut sorted = entries;
        sorted.sort_by(|a, b| b.accessed_at.cmp(&a.accessed_at));

        for entry in sorted.into_iter().take(max_entries) {
            if entry.is_expired() {
                continue;
            }

            // Skip if already in L1
            if self.l1.contains(&entry.key) {
                continue;
            }

            // Check L1 size limit
            if self.l1.size() + entry.size > self.config.l1_size_limit {
                break;
            }

            // Read from disk and add to L1
            let hash = match ContentHash::from_hex(&entry.key) {
                Some(h) => h,
                None => continue,
            };

            if let Ok(Some((data, _))) = self.l2.get_raw(&hash) {
                let l1_entry = L1Entry::new(
                    Bytes::from(data),
                    entry.original_size,
                    entry.compressed,
                    *hash.as_bytes(),
                );
                self.l1.insert(entry.key, l1_entry);
                warmed += 1;
            }
        }

        self.warming.store(false, Ordering::SeqCst);
        debug!(warmed, "cache warming complete");
    }

    /// Start background maintenance tasks.
    pub fn start_background_tasks(self: &Arc<Self>) {
        let cache = Arc::clone(self);
        let gc_interval = cache.config.gc_interval;

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(gc_interval);

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        if let Err(e) = cache.gc() {
                            warn!(error = %e, "background GC failed");
                        }
                    }
                    _ = cache.shutdown.notified() => {
                        break;
                    }
                }
            }
        });

        // Warm cache on startup if configured
        if self.config.warm_on_startup {
            let cache = Arc::clone(self);
            tokio::spawn(async move {
                cache.warm();
            });
        }
    }

    /// Stop background tasks and save state.
    pub fn shutdown(&self) {
        self.shutdown.notify_waiters();
        self.save_bloom_filter();
    }

    /// Get cache statistics.
    #[must_use]
    pub fn stats(&self) -> CacheStatsSnapshot {
        self.stats.snapshot()
    }

    /// Get L1 fill ratio.
    #[must_use]
    pub fn l1_fill_ratio(&self) -> f64 {
        self.l1.fill_ratio()
    }

    /// Get L2 disk usage.
    #[must_use]
    pub fn l2_disk_usage(&self) -> u64 {
        self.l2.disk_usage()
    }

    /// Get total entry count.
    #[must_use]
    pub fn entry_count(&self) -> usize {
        self.l2.len()
    }

    /// Get bloom filter statistics.
    #[must_use]
    pub fn bloom_stats(&self) -> Option<crate::bloom::BloomFilterStats> {
        self.bloom.as_ref().map(|b| b.stats())
    }

    /// Flush L2 index to disk.
    ///
    /// # Errors
    /// Returns error if flush fails.
    pub fn flush(&self) -> Result<()> {
        self.l2.flush()?;
        self.save_bloom_filter();
        Ok(())
    }

    fn rebuild_bloom_filter(&self) {
        if let Some(bloom) = &self.bloom {
            let keys = self.l2.entries().into_iter().map(|e| e.key);
            bloom.rebuild(keys);
            debug!("rebuilt bloom filter");
        }
    }

    fn save_bloom_filter(&self) {
        if let Some(bloom) = &self.bloom {
            let bloom_path = self.root.join("bloom.bin");
            if let Err(e) = bloom.save(&bloom_path) {
                warn!(error = %e, "failed to save bloom filter");
            }
        }
    }
}

impl Drop for TieredCache {
    fn drop(&mut self) {
        self.shutdown();
        let _ = self.flush();
    }
}

/// Result of garbage collection.
#[derive(Debug, Clone)]
pub struct GcResult {
    /// Number of expired entries removed.
    pub expired_removed: usize,
    /// Number of entries evicted via LRU.
    pub lru_evicted: usize,
    /// Time taken for GC.
    pub duration: Duration,
}

/// Result of cache clearing by pattern.
#[derive(Debug, Clone)]
pub struct ClearResult {
    /// Pattern used for clearing.
    pub pattern: ClearPattern,
    /// Total number of entries removed.
    pub total_removed: usize,
    /// Entries removed per type.
    pub removed_by_type: Vec<(CacheEntryType, usize)>,
    /// Time taken for clearing.
    pub duration: Duration,
}

impl ClearResult {
    /// Format as human-readable summary.
    #[must_use]
    pub fn summary(&self) -> String {
        let mut lines = Vec::new();
        lines.push(format!(
            "Cleared {} entries in {:.2}ms",
            self.total_removed,
            self.duration.as_secs_f64() * 1000.0
        ));

        for (entry_type, count) in &self.removed_by_type {
            if *count > 0 {
                lines.push(format!("  {}: {}", entry_type.subdir(), count));
            }
        }

        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiered_basic_operations() {
        let dir = tempfile::tempdir().unwrap();
        let config = CacheConfig {
            root: Some(dir.path().join("cache")),
            ..Default::default()
        };
        let cache = TieredCache::with_config(config).unwrap();

        let data = b"test data for tiered cache";
        let hash = cache
            .put(data, CacheEntryType::Package, None, None)
            .unwrap();

        assert!(cache.contains(&hash));

        // Should hit L1
        let retrieved = cache.get(&hash).unwrap().unwrap();
        assert_eq!(retrieved, data);

        // Check stats
        let stats = cache.stats();
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.l1_hits, 1);
    }

    #[test]
    fn tiered_l2_promotion() {
        let dir = tempfile::tempdir().unwrap();
        let config = CacheConfig {
            root: Some(dir.path().join("cache")),
            l1_size_limit: 1024, // Small L1
            ..Default::default()
        };
        let cache = TieredCache::with_config(config).unwrap();

        let data = b"test data";
        let hash = cache
            .put(data, CacheEntryType::Package, None, None)
            .unwrap();

        // Clear L1
        cache.l1.clear();

        // Should hit L2 and promote to L1
        let retrieved = cache.get(&hash).unwrap().unwrap();
        assert_eq!(retrieved, data);

        let stats = cache.stats();
        assert_eq!(stats.l2_hits, 1);

        // Now should hit L1
        let _ = cache.get(&hash).unwrap().unwrap();
        let stats = cache.stats();
        assert_eq!(stats.l1_hits, 1);
    }

    #[test]
    fn tiered_bloom_filter() {
        let dir = tempfile::tempdir().unwrap();
        let config = CacheConfig {
            root: Some(dir.path().join("cache")),
            bloom_filter_enabled: true,
            ..Default::default()
        };
        let cache = TieredCache::with_config(config).unwrap();

        // Non-existent key should be fast negative
        let fake_hash = ContentHash::from_bytes(b"not in cache");
        assert!(!cache.contains(&fake_hash));

        let stats = cache.stats();
        assert!(stats.bloom_true_negatives > 0 || stats.misses > 0);
    }

    #[test]
    fn tiered_clear_by_pattern() {
        let dir = tempfile::tempdir().unwrap();
        let config = CacheConfig {
            root: Some(dir.path().join("cache")),
            ..Default::default()
        };
        let cache = TieredCache::with_config(config).unwrap();

        // Add different types of data
        cache
            .put(b"package data", CacheEntryType::Package, None, None)
            .unwrap();
        cache
            .put(b"metadata", CacheEntryType::Metadata, None, None)
            .unwrap();

        assert_eq!(cache.entry_count(), 2);

        // Clear only packages
        let result = cache.clear_by_pattern(ClearPattern::Packages).unwrap();
        assert_eq!(result.total_removed, 1);
        assert_eq!(cache.entry_count(), 1);

        // Clear all
        let result = cache.clear_by_pattern(ClearPattern::All).unwrap();
        assert_eq!(result.total_removed, 1);
        assert_eq!(cache.entry_count(), 0);
    }

    #[test]
    fn clear_pattern_parsing() {
        assert_eq!(
            ClearPattern::from_str("packages"),
            Some(ClearPattern::Packages)
        );
        assert_eq!(ClearPattern::from_str("pkg"), Some(ClearPattern::Packages));
        assert_eq!(ClearPattern::from_str("all"), Some(ClearPattern::All));
        assert_eq!(ClearPattern::from_str("*"), Some(ClearPattern::All));
        assert_eq!(ClearPattern::from_str("vcs"), Some(ClearPattern::Vcs));
        assert_eq!(ClearPattern::from_str("invalid"), None);
    }

    #[test]
    fn tiered_bloom_persistence() {
        let dir = tempfile::tempdir().unwrap();
        let cache_path = dir.path().join("cache");

        // Create cache and add data
        {
            let config = CacheConfig {
                root: Some(cache_path.clone()),
                bloom_filter_enabled: true,
                ..Default::default()
            };
            let cache = TieredCache::with_config(config).unwrap();

            for i in 0..100 {
                let data = format!("data {i}");
                cache
                    .put(data.as_bytes(), CacheEntryType::Package, None, None)
                    .unwrap();
            }

            // Explicitly save bloom filter
            cache.flush().unwrap();
        }

        // Verify bloom filter file exists
        assert!(cache_path.join("bloom.bin").exists());

        // Reopen cache and verify bloom filter is loaded
        {
            let config = CacheConfig {
                root: Some(cache_path),
                bloom_filter_enabled: true,
                ..Default::default()
            };
            let cache = TieredCache::with_config(config).unwrap();

            let bloom_stats = cache.bloom_stats().unwrap();
            assert!(bloom_stats.count >= 100);
        }
    }
}
