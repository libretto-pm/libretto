//! Advanced multi-tier, content-addressable cache system for Libretto.
//!
//! This crate provides a high-performance caching system with:
//!
//! - **L1 Memory Cache**: Fast in-memory LRU cache using [moka](https://crates.io/crates/moka)
//!   with configurable size limits (default 256MB)
//!
//! - **L2 Disk Cache**: Persistent content-addressable storage using BLAKE3 hashes
//!   for deduplication and fast lookups
//!
//! - **Bloom Filter**: Probabilistic data structure for fast "not cached" checks,
//!   avoiding expensive disk lookups for items definitely not in cache
//!
//! - **Zero-Copy Deserialization**: Uses [rkyv](https://crates.io/crates/rkyv) for
//!   fast metadata index serialization
//!
//! - **Compression**: Zstd compression (level 3) for cached data to reduce disk usage
//!
//! - **TTL-Based Expiration**: Configurable time-to-live for different entry types
//!
//! - **LRU Eviction**: Automatic eviction of least-recently-used entries when limits exceeded
//!
//! - **Background Maintenance**: Garbage collection and cache warming tasks
//!
//! ## Performance Targets
//!
//! - Cache operations < 1ms
//! - Support 100k+ cached packages
//! - Startup cache load < 50ms
//!
//! ## Cache Locations (XDG Base Directory Compliant)
//!
//! - Linux: `~/.cache/libretto/`
//! - macOS: `~/Library/Caches/libretto/`
//! - Windows: `%LOCALAPPDATA%\libretto\cache\`
//!
//! ## Example
//!
//! ```no_run
//! use libretto_cache::{TieredCache, CacheConfig, CacheEntryType};
//! use std::time::Duration;
//!
//! # fn main() -> libretto_core::Result<()> {
//! // Create cache with default settings
//! let cache = TieredCache::new()?;
//!
//! // Or customize configuration
//! let config = CacheConfig::builder()
//!     .l1_size_limit(512 * 1024 * 1024) // 512MB
//!     .compression_level(5)
//!     .build();
//! let cache = TieredCache::with_config(config)?;
//!
//! // Store data
//! let data = b"package contents...";
//! let hash = cache.put(data, CacheEntryType::Package, None, None)?;
//!
//! // Retrieve data
//! if let Some(cached_data) = cache.get(&hash)? {
//!     assert_eq!(cached_data, data);
//! }
//!
//! // Check statistics
//! let stats = cache.stats();
//! println!("Hit rate: {:.1}%", stats.hit_rate * 100.0);
//! # Ok(())
//! # }
//! ```

#![deny(clippy::all)]
#![allow(clippy::module_name_repetitions)]
#![allow(unsafe_code)] // Required for memmap2

mod bloom;
mod compression;
mod config;
mod index;
mod l1;
mod l2;
pub mod simd;
mod stats;
mod tiered;

// Re-export main types
pub use bloom::{BloomFilter, BloomFilterStats, ConcurrentBloomFilter};
pub use compression::{
    compress, compress_with_stats, decompress, decompress_with_hint, is_compressed,
    should_compress, strip_magic, with_magic, CompressionStats, COMPRESSED_MAGIC,
};
pub use config::{CacheConfig, CacheConfigBuilder, CacheEntryType};
pub use index::{CacheIndex, IndexEntry};
pub use l1::{L1Cache, L1CacheBuilder, L1Entry};
pub use l2::L2Cache;
pub use stats::{CacheStats, CacheStatsSnapshot, SizeTracker};
pub use tiered::{ClearPattern, ClearResult, GcResult, TieredCache};

// Legacy API for backwards compatibility
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use libretto_core::{ContentHash, Error, PackageId, Result};
use libretto_platform::Platform;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{debug, info};

/// Legacy cache entry metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    /// Package identifier.
    pub package_id: PackageId,
    /// Version string.
    pub version: String,
    /// Content hash.
    pub hash: ContentHash,
    /// When cached.
    pub cached_at: DateTime<Utc>,
    /// Last accessed.
    pub last_accessed: DateTime<Utc>,
    /// Size in bytes.
    pub size: u64,
}

/// Legacy cache statistics.
#[derive(Debug, Default, Clone)]
pub struct LegacyCacheStats {
    /// Number of entries.
    pub entries: usize,
    /// Total size in bytes.
    pub total_size: u64,
    /// Cache hits.
    pub hits: u64,
    /// Cache misses.
    pub misses: u64,
}

/// Legacy package cache manager.
///
/// This provides backward compatibility with the original Cache API.
/// For new code, use [`TieredCache`] instead.
#[derive(Debug)]
pub struct Cache {
    root: PathBuf,
    entries: DashMap<String, CacheEntry>,
    stats: Arc<RwLock<LegacyCacheStats>>,
}

impl Cache {
    /// Create cache at default location.
    ///
    /// # Errors
    /// Returns error if cache directory cannot be created.
    pub fn new() -> Result<Self> {
        Self::at_path(Platform::current().cache_dir.join("packages"))
    }

    /// Create cache at specific path.
    ///
    /// # Errors
    /// Returns error if cache directory cannot be created.
    pub fn at_path(root: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&root).map_err(|e| Error::io(&root, e))?;

        let cache = Self {
            root,
            entries: DashMap::new(),
            stats: Arc::new(RwLock::new(LegacyCacheStats::default())),
        };

        cache.load_index()?;
        Ok(cache)
    }

    /// Get cache root path.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Check if package is cached.
    #[must_use]
    pub fn contains(&self, package_id: &PackageId, version: &str) -> bool {
        let key = Self::make_key(package_id, version);
        self.entries.contains_key(&key)
    }

    /// Get cached package path.
    #[must_use]
    pub fn get_path(&self, package_id: &PackageId, version: &str) -> Option<PathBuf> {
        let key = Self::make_key(package_id, version);
        if let Some(mut entry) = self.entries.get_mut(&key) {
            entry.last_accessed = Utc::now();
            self.stats.write().hits += 1;
            let path = self.package_path(package_id, version);
            if path.exists() {
                return Some(path);
            }
        }
        self.stats.write().misses += 1;
        None
    }

    /// Store package in cache.
    ///
    /// # Errors
    /// Returns error if package cannot be stored.
    pub fn store(
        &self,
        package_id: &PackageId,
        version: &str,
        source: &Path,
        hash: ContentHash,
    ) -> Result<PathBuf> {
        let dest = self.package_path(package_id, version);

        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }

        let size = copy_recursive(source, &dest)?;

        let entry = CacheEntry {
            package_id: package_id.clone(),
            version: version.to_string(),
            hash,
            cached_at: Utc::now(),
            last_accessed: Utc::now(),
            size,
        };

        let key = Self::make_key(package_id, version);
        self.entries.insert(key, entry);

        {
            let mut stats = self.stats.write();
            stats.entries = self.entries.len();
            stats.total_size += size;
        }

        self.save_index()?;
        info!(
            package = %package_id,
            version = %version,
            "cached package"
        );

        Ok(dest)
    }

    /// Remove package from cache.
    ///
    /// # Errors
    /// Returns error if package cannot be removed.
    pub fn remove(&self, package_id: &PackageId, version: &str) -> Result<bool> {
        let key = Self::make_key(package_id, version);
        if let Some((_, entry)) = self.entries.remove(&key) {
            let path = self.package_path(package_id, version);
            if path.exists() {
                std::fs::remove_dir_all(&path).map_err(|e| Error::io(&path, e))?;
            }

            {
                let mut stats = self.stats.write();
                stats.entries = self.entries.len();
                stats.total_size = stats.total_size.saturating_sub(entry.size);
            }

            self.save_index()?;
            debug!(package = %package_id, version = %version, "removed from cache");
            return Ok(true);
        }
        Ok(false)
    }

    /// Clear entire cache.
    ///
    /// # Errors
    /// Returns error if cache cannot be cleared.
    pub fn clear(&self) -> Result<()> {
        self.entries.clear();
        *self.stats.write() = LegacyCacheStats::default();

        if self.root.exists() {
            std::fs::remove_dir_all(&self.root).map_err(|e| Error::io(&self.root, e))?;
            std::fs::create_dir_all(&self.root).map_err(|e| Error::io(&self.root, e))?;
        }

        info!("cache cleared");
        Ok(())
    }

    /// Get cache statistics.
    #[must_use]
    pub fn legacy_stats(&self) -> LegacyCacheStats {
        self.stats.read().clone()
    }

    /// Prune old entries.
    ///
    /// # Errors
    /// Returns error if pruning fails.
    pub fn prune(&self, max_age_days: i64) -> Result<usize> {
        let cutoff = Utc::now() - chrono::Duration::days(max_age_days);
        let mut removed = 0;

        let to_remove: Vec<_> = self
            .entries
            .iter()
            .filter(|e| e.last_accessed < cutoff)
            .map(|e| (e.package_id.clone(), e.version.clone()))
            .collect();

        for (package_id, version) in to_remove {
            if self.remove(&package_id, &version)? {
                removed += 1;
            }
        }

        if removed > 0 {
            info!(count = removed, "pruned cache entries");
        }

        Ok(removed)
    }

    fn package_path(&self, package_id: &PackageId, version: &str) -> PathBuf {
        self.root
            .join(package_id.vendor())
            .join(package_id.name())
            .join(version)
    }

    fn make_key(package_id: &PackageId, version: &str) -> String {
        format!("{}:{}", package_id, version)
    }

    fn index_path(&self) -> PathBuf {
        self.root.join("cache-index.json")
    }

    fn load_index(&self) -> Result<()> {
        let path = self.index_path();
        if !path.exists() {
            return Ok(());
        }

        let data = std::fs::read(&path).map_err(|e| Error::io(&path, e))?;
        let entries: Vec<CacheEntry> =
            sonic_rs::from_slice(&data).map_err(libretto_core::Error::from)?;

        let mut total_size = 0u64;
        for entry in entries {
            total_size += entry.size;
            let key = Self::make_key(&entry.package_id, &entry.version);
            self.entries.insert(key, entry);
        }

        let mut stats = self.stats.write();
        stats.entries = self.entries.len();
        stats.total_size = total_size;

        debug!(entries = stats.entries, "loaded cache index");
        Ok(())
    }

    fn save_index(&self) -> Result<()> {
        let entries: Vec<CacheEntry> = self.entries.iter().map(|e| e.value().clone()).collect();
        let data = sonic_rs::to_string_pretty(&entries).map_err(libretto_core::Error::from)?;
        let path = self.index_path();
        std::fs::write(&path, data).map_err(|e| Error::io(&path, e))?;
        Ok(())
    }
}

fn copy_recursive(src: &Path, dest: &Path) -> Result<u64> {
    let mut total_size = 0u64;

    if src.is_file() {
        std::fs::copy(src, dest).map_err(|e| Error::io(src, e))?;
        return Ok(std::fs::metadata(src).map_err(|e| Error::io(src, e))?.len());
    }

    std::fs::create_dir_all(dest).map_err(|e| Error::io(dest, e))?;

    for entry in walkdir::WalkDir::new(src).min_depth(1) {
        let entry = entry.map_err(|e| Error::Cache(e.to_string()))?;
        let relative = entry
            .path()
            .strip_prefix(src)
            .map_err(|e| Error::Cache(e.to_string()))?;
        let dest_path = dest.join(relative);

        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&dest_path).map_err(|e| Error::io(&dest_path, e))?;
        } else {
            if let Some(parent) = dest_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
            }
            std::fs::copy(entry.path(), &dest_path).map_err(|e| Error::io(entry.path(), e))?;
            total_size += entry
                .metadata()
                .map_err(|e| Error::Cache(e.to_string()))?
                .len();
        }
    }

    Ok(total_size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_operations() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::at_path(dir.path().join("cache")).unwrap();

        let pkg_id = PackageId::new("test", "package");
        assert!(!cache.contains(&pkg_id, "1.0.0"));

        let stats = cache.legacy_stats();
        assert_eq!(stats.entries, 0);
    }

    #[test]
    fn tiered_cache_creation() {
        let dir = tempfile::tempdir().unwrap();
        let config = CacheConfig {
            root: Some(dir.path().join("tiered")),
            ..Default::default()
        };
        let cache = TieredCache::with_config(config).unwrap();

        assert_eq!(cache.entry_count(), 0);
    }
}
