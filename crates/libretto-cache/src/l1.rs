//! L1 in-memory LRU cache using moka.
//!
//! This provides fast, thread-safe access to frequently used cache entries
//! with true LRU eviction based on weighted size limits.

use bytes::Bytes;
use moka::sync::Cache;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// L1 in-memory cache entry.
#[derive(Debug, Clone)]
pub struct L1Entry {
    /// Raw data (possibly compressed).
    pub data: Bytes,
    /// Size in bytes (stored data size).
    pub size: u64,
    /// Original (uncompressed) size.
    pub original_size: u64,
    /// Whether data is compressed.
    pub compressed: bool,
    /// Content hash for verification.
    pub hash: [u8; 32],
}

impl L1Entry {
    /// Create new L1 entry.
    #[must_use]
    pub fn new(data: Bytes, original_size: u64, compressed: bool, hash: [u8; 32]) -> Self {
        Self {
            size: data.len() as u64,
            data,
            original_size,
            compressed,
            hash,
        }
    }

    /// Get data bytes.
    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Calculate the weight of this entry for cache sizing.
    /// Includes data size plus overhead for the struct itself.
    #[inline]
    fn weight(&self) -> u32 {
        // Data size + struct overhead (hash + metadata fields)
        let total = self.data.len() + 32 + 24; // 32 bytes hash + ~24 bytes metadata
        total.try_into().unwrap_or(u32::MAX)
    }
}

/// L1 memory cache backed by moka with true LRU eviction.
pub struct L1Cache {
    /// The underlying moka cache.
    cache: Cache<String, Arc<L1Entry>>,
    /// Maximum size in bytes (for reporting).
    max_size: u64,
    /// Insertion counter for statistics.
    insertions: AtomicU64,
    /// Eviction counter for statistics.
    evictions: AtomicU64,
}

impl std::fmt::Debug for L1Cache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("L1Cache")
            .field("max_size", &self.max_size)
            .field("current_size", &self.cache.weighted_size())
            .field("entry_count", &self.cache.entry_count())
            .finish()
    }
}

impl L1Cache {
    /// Create a new L1 cache with the given size limit and optional TTL.
    ///
    /// # Arguments
    /// * `max_size` - Maximum cache size in bytes (default 256MB)
    /// * `ttl` - Optional time-to-live for entries
    #[must_use]
    pub fn new(max_size: u64, ttl: Option<Duration>) -> Self {
        let evictions = Arc::new(AtomicU64::new(0));
        let evictions_clone = Arc::clone(&evictions);

        let mut builder = Cache::builder()
            .max_capacity(max_size)
            .weigher(|_key: &String, value: &Arc<L1Entry>| value.weight())
            .eviction_listener(move |_key, _value, _cause| {
                evictions_clone.fetch_add(1, Ordering::Relaxed);
            });

        if let Some(ttl_duration) = ttl {
            builder = builder.time_to_live(ttl_duration);
        }

        // Extract the inner AtomicU64 from the Arc for struct storage
        let evictions_inner = Arc::try_unwrap(evictions)
            .unwrap_or_else(|arc| AtomicU64::new(arc.load(Ordering::Relaxed)));

        Self {
            cache: builder.build(),
            max_size,
            insertions: AtomicU64::new(0),
            evictions: evictions_inner,
        }
    }

    /// Create with default settings (256MB, no TTL).
    #[must_use]
    pub fn default_size() -> Self {
        Self::new(256 * 1024 * 1024, None)
    }

    /// Get an entry from the cache.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<Arc<L1Entry>> {
        self.cache.get(key)
    }

    /// Insert an entry into the cache.
    pub fn insert(&self, key: String, entry: L1Entry) {
        self.insertions.fetch_add(1, Ordering::Relaxed);
        self.cache.insert(key, Arc::new(entry));
    }

    /// Remove an entry from the cache.
    pub fn remove(&self, key: &str) {
        self.cache.invalidate(key);
    }

    /// Check if key exists.
    #[must_use]
    pub fn contains(&self, key: &str) -> bool {
        self.cache.contains_key(key)
    }

    /// Clear all entries.
    pub fn clear(&self) {
        self.cache.invalidate_all();
    }

    /// Get number of entries.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.cache.entry_count()
    }

    /// Check if cache is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cache.entry_count() == 0
    }

    /// Get current weighted size in bytes.
    #[must_use]
    pub fn size(&self) -> u64 {
        self.cache.weighted_size()
    }

    /// Get maximum size in bytes.
    #[must_use]
    pub fn max_size(&self) -> u64 {
        self.max_size
    }

    /// Get fill percentage (0.0 - 1.0).
    #[must_use]
    pub fn fill_ratio(&self) -> f64 {
        if self.max_size == 0 {
            return 0.0;
        }
        self.cache.weighted_size() as f64 / self.max_size as f64
    }

    /// Run pending maintenance tasks (LRU eviction, expiration).
    pub fn run_pending_tasks(&self) {
        self.cache.run_pending_tasks();
    }

    /// Get all keys currently in the cache.
    pub fn keys(&self) -> Vec<String> {
        self.cache.iter().map(|(k, _)| (*k).clone()).collect()
    }

    /// Get total insertions count.
    #[must_use]
    pub fn insertion_count(&self) -> u64 {
        self.insertions.load(Ordering::Relaxed)
    }

    /// Get total evictions count.
    #[must_use]
    pub fn eviction_count(&self) -> u64 {
        self.evictions.load(Ordering::Relaxed)
    }

    /// Sync method to iterate over all entries (for warming, etc.).
    pub fn for_each<F>(&self, mut f: F)
    where
        F: FnMut(&str, &L1Entry),
    {
        for (key, value) in self.cache.iter() {
            f(&key, &value);
        }
    }
}

/// Builder for L1 cache.
#[derive(Debug, Default)]
pub struct L1CacheBuilder {
    max_size: Option<u64>,
    ttl: Option<Duration>,
    time_to_idle: Option<Duration>,
}

impl L1CacheBuilder {
    /// Create new builder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set maximum cache size in bytes.
    #[must_use]
    pub fn max_size(mut self, size: u64) -> Self {
        self.max_size = Some(size);
        self
    }

    /// Set time-to-live for entries.
    #[must_use]
    pub fn ttl(mut self, ttl: Duration) -> Self {
        self.ttl = Some(ttl);
        self
    }

    /// Set time-to-idle for entries.
    #[must_use]
    pub fn time_to_idle(mut self, tti: Duration) -> Self {
        self.time_to_idle = Some(tti);
        self
    }

    /// Build the cache.
    #[must_use]
    pub fn build(self) -> L1Cache {
        let max_size = self.max_size.unwrap_or(256 * 1024 * 1024);
        let evictions = Arc::new(AtomicU64::new(0));
        let evictions_clone = Arc::clone(&evictions);

        let mut builder = Cache::builder()
            .max_capacity(max_size)
            .weigher(|_key: &String, value: &Arc<L1Entry>| value.weight())
            .eviction_listener(move |_key, _value, _cause| {
                evictions_clone.fetch_add(1, Ordering::Relaxed);
            });

        if let Some(ttl) = self.ttl {
            builder = builder.time_to_live(ttl);
        }

        if let Some(tti) = self.time_to_idle {
            builder = builder.time_to_idle(tti);
        }

        let evictions_inner = Arc::try_unwrap(evictions)
            .unwrap_or_else(|arc| AtomicU64::new(arc.load(Ordering::Relaxed)));

        L1Cache {
            cache: builder.build(),
            max_size,
            insertions: AtomicU64::new(0),
            evictions: evictions_inner,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l1_basic_operations() {
        let cache = L1Cache::new(1024 * 1024, None);

        let entry = L1Entry::new(Bytes::from("test data"), 9, false, [0u8; 32]);

        cache.insert("key1".to_string(), entry);
        cache.run_pending_tasks();

        assert!(cache.contains("key1"));
        assert!(!cache.contains("key2"));

        let retrieved = cache.get("key1").expect("should exist");
        assert_eq!(retrieved.data(), b"test data");

        cache.remove("key1");
        cache.run_pending_tasks();
        assert!(!cache.contains("key1"));
    }

    #[test]
    fn l1_size_tracking() {
        let cache = L1Cache::new(1024 * 1024, None);

        let entry1 = L1Entry::new(Bytes::from(vec![0u8; 100]), 100, false, [0u8; 32]);
        let entry2 = L1Entry::new(Bytes::from(vec![0u8; 200]), 200, false, [0u8; 32]);

        cache.insert("key1".to_string(), entry1);
        cache.insert("key2".to_string(), entry2);
        cache.run_pending_tasks();

        // Size includes overhead, so it's more than just data size
        assert!(cache.size() >= 300);
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn l1_lru_eviction() {
        // Small cache that can only fit a few entries
        let cache = L1Cache::new(500, None);

        // Insert entries that will exceed capacity
        for i in 0..10 {
            let data = vec![i as u8; 100];
            let entry = L1Entry::new(Bytes::from(data), 100, false, [i as u8; 32]);
            cache.insert(format!("key{i}"), entry);
        }
        cache.run_pending_tasks();

        // Should have evicted some entries
        assert!(cache.len() < 10);
        assert!(cache.size() <= 500);
    }

    #[test]
    fn l1_clear() {
        let cache = L1Cache::new(1024, None);

        cache.insert(
            "key1".to_string(),
            L1Entry::new(Bytes::from("data"), 4, false, [0u8; 32]),
        );
        cache.run_pending_tasks();

        cache.clear();
        cache.run_pending_tasks();

        assert!(cache.is_empty());
    }

    #[test]
    fn l1_builder() {
        let cache = L1CacheBuilder::new()
            .max_size(512 * 1024)
            .ttl(Duration::from_secs(3600))
            .build();

        assert_eq!(cache.max_size(), 512 * 1024);
    }

    #[test]
    fn l1_fill_ratio() {
        let cache = L1Cache::new(1000, None);

        let entry = L1Entry::new(Bytes::from(vec![0u8; 400]), 400, false, [0u8; 32]);
        cache.insert("key1".to_string(), entry);
        cache.run_pending_tasks();

        let ratio = cache.fill_ratio();
        assert!(ratio > 0.0);
        assert!(ratio <= 1.0);
    }
}
