//! Repository caching layer with TTL, `ETags`, and statistics.

use crate::client::CacheMetadata;
use dashmap::DashMap;
use libretto_cache::{CacheEntryType, TieredCache};
use libretto_core::{ContentHash, Error, Result};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tracing::debug;

/// Default TTL for package metadata (6 hours).
pub const DEFAULT_METADATA_TTL: Duration = Duration::from_secs(6 * 60 * 60);

/// Default TTL for repository index (5 minutes).
pub const DEFAULT_REPOSITORY_TTL: Duration = Duration::from_secs(5 * 60);

/// Default TTL for search results (1 hour).
pub const DEFAULT_SEARCH_TTL: Duration = Duration::from_secs(60 * 60);

/// Default TTL for security advisories (1 hour).
pub const DEFAULT_ADVISORY_TTL: Duration = Duration::from_secs(60 * 60);

/// Default TTL for packages (24 hours).
pub const DEFAULT_PACKAGE_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// Repository cache statistics.
#[derive(Debug, Default)]
pub struct RepositoryCacheStats {
    /// Cache hits.
    pub hits: AtomicU64,
    /// Cache misses.
    pub misses: AtomicU64,
    /// Cache puts.
    pub puts: AtomicU64,
    /// Cache invalidations.
    pub invalidations: AtomicU64,
    /// Conditional request hits (304).
    pub conditional_hits: AtomicU64,
    /// Total bytes cached.
    pub bytes_cached: AtomicU64,
    /// Total bytes served from cache.
    pub bytes_served: AtomicU64,
}

impl RepositoryCacheStats {
    /// Create new stats tracker.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Get cache hit rate.
    #[must_use]
    pub fn hit_rate(&self) -> f64 {
        let hits = self.hits.load(Ordering::Relaxed);
        let misses = self.misses.load(Ordering::Relaxed);
        let total = hits + misses;
        if total == 0 {
            0.0
        } else {
            (hits as f64 / total as f64) * 100.0
        }
    }

    /// Format stats as human-readable summary.
    #[must_use]
    pub fn summary(&self) -> String {
        let hits = self.hits.load(Ordering::Relaxed);
        let misses = self.misses.load(Ordering::Relaxed);
        let conditional = self.conditional_hits.load(Ordering::Relaxed);
        let bytes_cached = self.bytes_cached.load(Ordering::Relaxed);
        let bytes_served = self.bytes_served.load(Ordering::Relaxed);

        format!(
            "Cache: {:.1}% hit rate ({} hits, {} misses, {} conditional), {} cached, {} served",
            self.hit_rate(),
            hits,
            misses,
            conditional,
            format_bytes(bytes_cached),
            format_bytes(bytes_served)
        )
    }
}

/// Cached entry with metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedEntry {
    /// The cached data.
    pub data: Vec<u8>,
    /// When the entry was cached.
    pub cached_at_secs: u64,
    /// TTL in seconds.
    pub ttl_secs: u64,
    /// `ETag` if available.
    pub etag: Option<String>,
    /// Last-Modified if available.
    pub last_modified: Option<String>,
    /// Original URL.
    pub url: String,
}

impl CachedEntry {
    /// Check if the entry has expired.
    #[must_use]
    pub fn is_expired(&self) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        now > self.cached_at_secs + self.ttl_secs
    }

    /// Get remaining TTL.
    #[must_use]
    pub fn remaining_ttl(&self) -> Option<Duration> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let expires_at = self.cached_at_secs + self.ttl_secs;
        if now < expires_at {
            Some(Duration::from_secs(expires_at - now))
        } else {
            None
        }
    }
}

/// In-memory cache entry for fast lookups.
#[derive(Debug, Clone)]
struct MemoryCacheEntry {
    /// The cached data.
    data: Arc<Vec<u8>>,
    /// When the entry was cached.
    cached_at: Instant,
    /// TTL.
    ttl: Duration,
    /// `ETag` if available.
    etag: Option<String>,
    /// Last-Modified if available.
    last_modified: Option<String>,
}

impl MemoryCacheEntry {
    fn is_expired(&self) -> bool {
        self.cached_at.elapsed() > self.ttl
    }
}

/// Repository cache combining in-memory and tiered disk cache.
pub struct RepositoryCache {
    /// In-memory cache for hot data.
    memory: DashMap<String, MemoryCacheEntry>,
    /// Persistent tiered cache.
    tiered: Option<Arc<TieredCache>>,
    /// HTTP metadata cache (`ETags`, Last-Modified).
    http_metadata: DashMap<String, CacheMetadata>,
    /// Statistics.
    stats: Arc<RepositoryCacheStats>,
    /// Maximum memory cache size.
    max_memory_size: RwLock<u64>,
    /// Current memory cache size.
    current_memory_size: AtomicU64,
}

impl std::fmt::Debug for RepositoryCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RepositoryCache")
            .field("memory_entries", &self.memory.len())
            .field("has_tiered", &self.tiered.is_some())
            .finish()
    }
}

impl RepositoryCache {
    /// Create a new repository cache.
    #[must_use]
    pub fn new() -> Self {
        Self {
            memory: DashMap::new(),
            tiered: None,
            http_metadata: DashMap::new(),
            stats: Arc::new(RepositoryCacheStats::new()),
            max_memory_size: RwLock::new(256 * 1024 * 1024), // 256MB default
            current_memory_size: AtomicU64::new(0),
        }
    }

    /// Create cache with tiered backing storage.
    pub fn with_tiered_cache(tiered: Arc<TieredCache>) -> Self {
        Self {
            memory: DashMap::new(),
            tiered: Some(tiered),
            http_metadata: DashMap::new(),
            stats: Arc::new(RepositoryCacheStats::new()),
            max_memory_size: RwLock::new(256 * 1024 * 1024),
            current_memory_size: AtomicU64::new(0),
        }
    }

    /// Set maximum memory cache size.
    pub fn set_max_memory_size(&self, size: u64) {
        *self.max_memory_size.write() = size;
    }

    /// Get metadata from cache.
    ///
    /// Returns cached data and whether it's a hit.
    pub fn get_metadata(&self, key: &str) -> Option<Arc<Vec<u8>>> {
        // Check memory cache first
        if let Some(entry) = self.memory.get(key) {
            if !entry.is_expired() {
                self.stats.hits.fetch_add(1, Ordering::Relaxed);
                self.stats
                    .bytes_served
                    .fetch_add(entry.data.len() as u64, Ordering::Relaxed);
                debug!(key = %key, "memory cache hit");
                return Some(Arc::clone(&entry.data));
            }
            // Remove expired entry
            drop(entry);
            self.memory.remove(key);
        }

        // Check tiered cache
        if let Some(ref tiered) = self.tiered {
            let hash = ContentHash::from_bytes(key.as_bytes());
            if let Ok(Some(data)) = tiered.get(&hash) {
                // Promote to memory cache
                let entry = MemoryCacheEntry {
                    data: Arc::new(data.clone()),
                    cached_at: Instant::now(),
                    ttl: DEFAULT_METADATA_TTL,
                    etag: None,
                    last_modified: None,
                };
                self.maybe_evict_memory(data.len() as u64);
                self.memory.insert(key.to_string(), entry);
                self.current_memory_size
                    .fetch_add(data.len() as u64, Ordering::Relaxed);

                self.stats.hits.fetch_add(1, Ordering::Relaxed);
                self.stats
                    .bytes_served
                    .fetch_add(data.len() as u64, Ordering::Relaxed);
                debug!(key = %key, "tiered cache hit");
                return Some(Arc::new(data));
            }
        }

        self.stats.misses.fetch_add(1, Ordering::Relaxed);
        None
    }

    /// Put metadata in cache.
    ///
    /// # Errors
    /// Returns error if caching fails.
    pub fn put_metadata(
        &self,
        key: &str,
        data: &[u8],
        ttl: Duration,
        http_meta: Option<&CacheMetadata>,
    ) -> Result<()> {
        let data_len = data.len() as u64;

        // Store in memory cache
        let entry = MemoryCacheEntry {
            data: Arc::new(data.to_vec()),
            cached_at: Instant::now(),
            ttl,
            etag: http_meta.and_then(|m| m.etag.clone()),
            last_modified: http_meta.and_then(|m| m.last_modified.clone()),
        };

        self.maybe_evict_memory(data_len);
        self.memory.insert(key.to_string(), entry);
        self.current_memory_size
            .fetch_add(data_len, Ordering::Relaxed);

        // Store in tiered cache
        if let Some(ref tiered) = self.tiered {
            let metadata = http_meta.map(|m| {
                sonic_rs::to_string(&CachedEntryMetadata {
                    etag: m.etag.clone(),
                    last_modified: m.last_modified.clone(),
                })
                .unwrap_or_default()
            });

            tiered
                .put(data, CacheEntryType::Metadata, Some(ttl), metadata)
                .map_err(|e| Error::cache(e.to_string()))?;
        }

        // Store HTTP metadata
        if let Some(meta) = http_meta {
            self.http_metadata.insert(key.to_string(), meta.clone());
        }

        self.stats.puts.fetch_add(1, Ordering::Relaxed);
        self.stats
            .bytes_cached
            .fetch_add(data_len, Ordering::Relaxed);

        debug!(key = %key, size = data_len, ttl_secs = ttl.as_secs(), "cached metadata");
        Ok(())
    }

    /// Get HTTP cache metadata for conditional requests.
    #[must_use]
    pub fn get_http_metadata(&self, key: &str) -> Option<CacheMetadata> {
        // Check memory cache first
        if let Some(entry) = self.memory.get(key)
            && (entry.etag.is_some() || entry.last_modified.is_some())
        {
            return Some(CacheMetadata {
                etag: entry.etag.clone(),
                last_modified: entry.last_modified.clone(),
                cached_at: entry.cached_at,
            });
        }

        // Check dedicated HTTP metadata cache
        self.http_metadata.get(key).map(|v| v.clone())
    }

    /// Record a conditional request hit (304 Not Modified).
    pub fn record_conditional_hit(&self) {
        self.stats.conditional_hits.fetch_add(1, Ordering::Relaxed);
    }

    /// Invalidate a cache entry.
    pub fn invalidate(&self, key: &str) {
        self.memory.remove(key);
        self.http_metadata.remove(key);

        if let Some(ref tiered) = self.tiered {
            let hash = ContentHash::from_bytes(key.as_bytes());
            let _ = tiered.remove(&hash);
        }

        self.stats.invalidations.fetch_add(1, Ordering::Relaxed);
        debug!(key = %key, "cache invalidated");
    }

    /// Clear all cached entries.
    pub fn clear(&self) {
        self.memory.clear();
        self.http_metadata.clear();
        self.current_memory_size.store(0, Ordering::Relaxed);

        if let Some(ref tiered) = self.tiered {
            let _ = tiered.clear_by_type(CacheEntryType::Metadata);
            let _ = tiered.clear_by_type(CacheEntryType::Repository);
        }

        debug!("cache cleared");
    }

    /// Get cache statistics.
    #[must_use]
    pub fn stats(&self) -> &RepositoryCacheStats {
        &self.stats
    }

    /// Get number of entries in memory cache.
    #[must_use]
    pub fn memory_entry_count(&self) -> usize {
        self.memory.len()
    }

    /// Get current memory usage.
    #[must_use]
    pub fn memory_usage(&self) -> u64 {
        self.current_memory_size.load(Ordering::Relaxed)
    }

    /// Evict memory cache entries if needed to make room.
    fn maybe_evict_memory(&self, needed: u64) {
        let max_size = *self.max_memory_size.read();
        let current = self.current_memory_size.load(Ordering::Relaxed);

        if current + needed <= max_size {
            return;
        }

        // Evict expired entries first
        let mut to_remove = Vec::new();
        for entry in &self.memory {
            if entry.is_expired() {
                to_remove.push(entry.key().clone());
            }
        }

        for key in to_remove {
            if let Some((_, entry)) = self.memory.remove(&key) {
                self.current_memory_size
                    .fetch_sub(entry.data.len() as u64, Ordering::Relaxed);
            }
        }

        // If still over limit, evict oldest entries
        let current = self.current_memory_size.load(Ordering::Relaxed);
        if current + needed > max_size {
            let mut entries: Vec<_> = self
                .memory
                .iter()
                .map(|e| (e.key().clone(), e.cached_at, e.data.len()))
                .collect();

            entries.sort_by_key(|(_, cached_at, _)| *cached_at);

            let mut freed = 0u64;
            let target = (current + needed).saturating_sub(max_size);

            for (key, _, size) in entries {
                if freed >= target {
                    break;
                }
                if let Some((_, _)) = self.memory.remove(&key) {
                    freed += size as u64;
                }
            }

            self.current_memory_size.fetch_sub(freed, Ordering::Relaxed);
        }
    }
}

impl Default for RepositoryCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Cached entry metadata for serialization.
#[derive(Debug, Serialize, Deserialize)]
struct CachedEntryMetadata {
    etag: Option<String>,
    last_modified: Option<String>,
}

/// Format bytes as human-readable string.
fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.1}GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1}MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1}KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes}B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_hit_miss() {
        let cache = RepositoryCache::new();

        // Miss
        assert!(cache.get_metadata("test-key").is_none());
        assert_eq!(cache.stats().misses.load(Ordering::Relaxed), 1);

        // Put
        cache
            .put_metadata("test-key", b"test data", Duration::from_secs(60), None)
            .unwrap();
        assert_eq!(cache.stats().puts.load(Ordering::Relaxed), 1);

        // Hit
        let data = cache.get_metadata("test-key").unwrap();
        assert_eq!(&*data, b"test data");
        assert_eq!(cache.stats().hits.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_cache_expiry() {
        let cache = RepositoryCache::new();

        // Put with very short TTL
        cache
            .put_metadata("expire-key", b"data", Duration::from_millis(1), None)
            .unwrap();

        // Wait for expiry
        std::thread::sleep(Duration::from_millis(10));

        // Should miss
        assert!(cache.get_metadata("expire-key").is_none());
    }

    #[test]
    fn test_invalidation() {
        let cache = RepositoryCache::new();

        cache
            .put_metadata("key", b"data", Duration::from_secs(60), None)
            .unwrap();
        assert!(cache.get_metadata("key").is_some());

        cache.invalidate("key");
        assert!(cache.get_metadata("key").is_none());
        assert_eq!(cache.stats().invalidations.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_stats_hit_rate() {
        let stats = RepositoryCacheStats::new();
        stats.hits.store(75, Ordering::Relaxed);
        stats.misses.store(25, Ordering::Relaxed);

        assert!((stats.hit_rate() - 75.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(500), "500B");
        assert_eq!(format_bytes(1024), "1.0KB");
        assert_eq!(format_bytes(1024 * 1024), "1.0MB");
        assert_eq!(format_bytes(1024 * 1024 * 1024), "1.0GB");
    }
}
