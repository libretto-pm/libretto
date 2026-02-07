//! Package index with concurrent caching.
//!
//! The package index provides fast, thread-safe access to package metadata.
//! It uses `DashMap` for lock-free concurrent reads and writes, with aggressive
//! caching of both package data and constraint evaluations.

use crate::package::{Dependency, PackageEntry, PackageName, PackageVersion};
use crate::version::{ComposerConstraint, ComposerVersion};
use ahash::AHashMap;
use dashmap::DashMap;
use parking_lot::RwLock;
use rayon::prelude::*;
use smallvec::SmallVec;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tracing::{debug, trace};

/// Statistics for cache performance monitoring.
#[derive(Debug, Default)]
pub struct CacheStats {
    hits: AtomicU64,
    misses: AtomicU64,
    constraint_cache_hits: AtomicU64,
    constraint_cache_misses: AtomicU64,
    prefetch_count: AtomicU64,
}

impl CacheStats {
    /// Record a cache hit.
    #[inline]
    pub fn record_hit(&self) {
        self.hits.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a cache miss.
    #[inline]
    pub fn record_miss(&self) {
        self.misses.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a constraint cache hit.
    #[inline]
    pub fn record_constraint_hit(&self) {
        self.constraint_cache_hits.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a constraint cache miss.
    #[inline]
    pub fn record_constraint_miss(&self) {
        self.constraint_cache_misses.fetch_add(1, Ordering::Relaxed);
    }

    /// Get the cache hit rate.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn hit_rate(&self) -> f64 {
        let hits = self.hits.load(Ordering::Relaxed);
        let misses = self.misses.load(Ordering::Relaxed);
        let total = hits + misses;
        if total == 0 {
            0.0
        } else {
            hits as f64 / total as f64
        }
    }

    /// Get the constraint cache hit rate.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn constraint_hit_rate(&self) -> f64 {
        let hits = self.constraint_cache_hits.load(Ordering::Relaxed);
        let misses = self.constraint_cache_misses.load(Ordering::Relaxed);
        let total = hits + misses;
        if total == 0 {
            0.0
        } else {
            hits as f64 / total as f64
        }
    }

    /// Get total operations.
    #[must_use]
    pub fn total_operations(&self) -> u64 {
        self.hits.load(Ordering::Relaxed) + self.misses.load(Ordering::Relaxed)
    }

    /// Reset all statistics.
    pub fn reset(&self) {
        self.hits.store(0, Ordering::Relaxed);
        self.misses.store(0, Ordering::Relaxed);
        self.constraint_cache_hits.store(0, Ordering::Relaxed);
        self.constraint_cache_misses.store(0, Ordering::Relaxed);
        self.prefetch_count.store(0, Ordering::Relaxed);
    }
}

/// Cached package entry with TTL.
#[derive(Debug, Clone)]
struct CachedEntry {
    entry: Arc<PackageEntry>,
    cached_at: Instant,
    ttl: Duration,
}

impl CachedEntry {
    fn new(entry: PackageEntry, ttl: Duration) -> Self {
        Self {
            entry: Arc::new(entry),
            cached_at: Instant::now(),
            ttl,
        }
    }

    fn is_expired(&self) -> bool {
        self.cached_at.elapsed() > self.ttl
    }
}

/// Key for constraint match cache.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ConstraintCacheKey {
    package: Arc<str>,
    constraint: Arc<str>,
}

/// Cached constraint evaluation result.
#[derive(Debug, Clone)]
struct ConstraintCacheEntry {
    /// Indices of matching versions in the package entry.
    matching_indices: SmallVec<[usize; 16]>,
    cached_at: Instant,
}

/// Configuration for the package index.
#[derive(Debug, Clone)]
pub struct IndexConfig {
    /// Default TTL for cached entries.
    pub default_ttl: Duration,
    /// Maximum number of packages to cache.
    pub max_cached_packages: usize,
    /// Maximum number of constraint evaluations to cache.
    pub max_constraint_cache: usize,
    /// TTL for constraint cache entries.
    pub constraint_cache_ttl: Duration,
}

impl Default for IndexConfig {
    fn default() -> Self {
        Self {
            default_ttl: Duration::from_secs(300),
            max_cached_packages: 8192,
            max_constraint_cache: 32768,
            constraint_cache_ttl: Duration::from_secs(60),
        }
    }
}

/// Trait for package data sources.
///
/// Implement this trait to provide package data from different sources
/// (e.g., Packagist API, local cache, filesystem).
pub trait PackageSource: Send + Sync {
    /// Fetch a package by name.
    ///
    /// Returns `None` if the package doesn't exist.
    fn fetch(&self, name: &PackageName) -> Option<PackageEntry>;

    /// Check if a package exists without fetching full data.
    fn exists(&self, name: &PackageName) -> bool {
        self.fetch(name).is_some()
    }

    /// Fetch multiple packages in parallel.
    ///
    /// Default implementation fetches sequentially; override for batch optimization.
    fn fetch_batch(&self, names: &[PackageName]) -> Vec<Option<PackageEntry>> {
        names.iter().map(|n| self.fetch(n)).collect()
    }
}

/// Map from a virtual/replacement package name to the packages that provide/replace it.
type ProviderMap =
    DashMap<Arc<str>, SmallVec<[(PackageName, ComposerVersion); 4]>, ahash::RandomState>;

/// The package index providing cached access to package metadata.
pub struct PackageIndex<S: PackageSource> {
    /// The underlying package source.
    source: Arc<S>,
    /// Package cache.
    packages: DashMap<Arc<str>, CachedEntry, ahash::RandomState>,
    /// Constraint evaluation cache.
    constraint_cache: DashMap<ConstraintCacheKey, ConstraintCacheEntry, ahash::RandomState>,
    /// Virtual packages (packages provided by others).
    virtual_packages: ProviderMap,
    /// Package replacements.
    replacements: ProviderMap,
    /// Configuration.
    config: IndexConfig,
    /// Cache statistics.
    pub stats: CacheStats,
}

impl<S: PackageSource> std::fmt::Debug for PackageIndex<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PackageIndex")
            .field("cached_packages", &self.packages.len())
            .field("cached_constraints", &self.constraint_cache.len())
            .field("virtual_packages", &self.virtual_packages.len())
            .field("replacements", &self.replacements.len())
            .field("config", &self.config)
            .field("stats", &self.stats)
            .finish_non_exhaustive()
    }
}

impl<S: PackageSource> PackageIndex<S> {
    /// Create a new package index with the given source.
    pub fn new(source: S) -> Self {
        Self::with_config(source, IndexConfig::default())
    }

    /// Create a new package index with custom configuration.
    pub fn with_config(source: S, config: IndexConfig) -> Self {
        Self {
            source: Arc::new(source),
            packages: DashMap::with_capacity_and_hasher(
                config.max_cached_packages,
                ahash::RandomState::new(),
            ),
            constraint_cache: DashMap::with_capacity_and_hasher(
                config.max_constraint_cache,
                ahash::RandomState::new(),
            ),
            virtual_packages: DashMap::with_hasher(ahash::RandomState::new()),
            replacements: DashMap::with_hasher(ahash::RandomState::new()),
            config,
            stats: CacheStats::default(),
        }
    }

    /// Get the underlying source.
    #[must_use]
    pub fn source(&self) -> &S {
        &self.source
    }

    /// Get a package by name.
    pub fn get(&self, name: &PackageName) -> Option<Arc<PackageEntry>> {
        let key: Arc<str> = Arc::from(name.as_str());

        // Check cache first
        if let Some(cached) = self.packages.get(&key)
            && !cached.is_expired()
        {
            self.stats.record_hit();
            trace!(package = %name, "cache hit");
            return Some(Arc::clone(&cached.entry));
        }
        // Expired, will refetch

        self.stats.record_miss();
        trace!(package = %name, "cache miss");

        // Fetch from source
        let entry = self.source.fetch(name)?;

        // Track virtual packages and replacements
        self.track_virtuals(&entry);

        // Evict if cache is full
        self.maybe_evict_packages();

        // Cache the entry
        let cached = CachedEntry::new(entry, self.config.default_ttl);
        let result = Arc::clone(&cached.entry);
        self.packages.insert(key, cached);

        Some(result)
    }

    /// Check if a package exists.
    pub fn exists(&self, name: &PackageName) -> bool {
        let key: Arc<str> = Arc::from(name.as_str());

        // Check cache first
        if let Some(cached) = self.packages.get(&key)
            && !cached.is_expired()
        {
            return true;
        }

        // Check source
        self.source.exists(name)
    }

    /// Get versions matching a constraint.
    pub fn get_matching_versions(
        &self,
        name: &PackageName,
        constraint: &ComposerConstraint,
    ) -> Vec<ComposerVersion> {
        let cache_key = ConstraintCacheKey {
            package: Arc::from(name.as_str()),
            constraint: Arc::from(constraint.as_str()),
        };

        // Check constraint cache
        if let Some(cached) = self.constraint_cache.get(&cache_key)
            && cached.cached_at.elapsed() < self.config.constraint_cache_ttl
        {
            self.stats.record_constraint_hit();

            // Get the package and extract versions by index
            if let Some(entry) = self.get(name) {
                return cached
                    .matching_indices
                    .iter()
                    .filter_map(|&i| entry.versions.get(i))
                    .map(|v| v.version.clone())
                    .collect();
            }
        }

        self.stats.record_constraint_miss();

        // Get package and filter
        let Some(entry) = self.get(name) else {
            return Vec::new();
        };

        // Find matching versions and their indices
        let matching: Vec<(usize, &PackageVersion)> = entry
            .versions
            .iter()
            .enumerate()
            .filter(|(_, v)| constraint.matches(&v.version))
            .collect();

        let indices: SmallVec<[usize; 16]> = matching.iter().map(|(i, _)| *i).collect();
        let versions: Vec<ComposerVersion> = matching
            .into_iter()
            .map(|(_, v)| v.version.clone())
            .collect();

        // Cache the result
        self.maybe_evict_constraints();
        self.constraint_cache.insert(
            cache_key,
            ConstraintCacheEntry {
                matching_indices: indices,
                cached_at: Instant::now(),
            },
        );

        versions
    }

    /// Get the highest version matching a constraint.
    pub fn get_highest_matching(
        &self,
        name: &PackageName,
        constraint: &ComposerConstraint,
    ) -> Option<ComposerVersion> {
        let entry = self.get(name)?;
        entry
            .versions
            .iter()
            .find(|v| constraint.matches(&v.version))
            .map(|v| v.version.clone())
    }

    /// Get the lowest version matching a constraint.
    pub fn get_lowest_matching(
        &self,
        name: &PackageName,
        constraint: &ComposerConstraint,
    ) -> Option<ComposerVersion> {
        let entry = self.get(name)?;
        entry
            .versions
            .iter()
            .rev()
            .find(|v| constraint.matches(&v.version))
            .map(|v| v.version.clone())
    }

    /// Get dependencies for a specific version.
    pub fn get_dependencies(
        &self,
        name: &PackageName,
        version: &ComposerVersion,
    ) -> Option<Vec<Dependency>> {
        let entry = self.get(name)?;
        entry
            .versions
            .iter()
            .find(|v| &v.version == version)
            .map(|v| v.dependencies.to_vec())
    }

    /// Get dev dependencies for a specific version.
    pub fn get_dev_dependencies(
        &self,
        name: &PackageName,
        version: &ComposerVersion,
    ) -> Option<Vec<Dependency>> {
        let entry = self.get(name)?;
        entry
            .versions
            .iter()
            .find(|v| &v.version == version)
            .map(|v| v.dev_dependencies.to_vec())
    }

    /// Get packages that provide a virtual package.
    #[must_use]
    pub fn get_providers(&self, virtual_name: &str) -> Vec<(PackageName, ComposerVersion)> {
        self.virtual_packages
            .get(virtual_name)
            .map(|r| r.value().to_vec())
            .unwrap_or_default()
    }

    /// Get packages that replace another package.
    #[must_use]
    pub fn get_replacers(&self, package_name: &str) -> Vec<(PackageName, ComposerVersion)> {
        self.replacements
            .get(package_name)
            .map(|r| r.value().to_vec())
            .unwrap_or_default()
    }

    /// Prefetch multiple packages in parallel.
    pub fn prefetch(&self, names: &[PackageName]) {
        self.stats
            .prefetch_count
            .fetch_add(names.len() as u64, Ordering::Relaxed);

        // Filter out already cached packages
        let to_fetch: Vec<_> = names
            .iter()
            .filter(|name| {
                let key: Arc<str> = Arc::from(name.as_str());
                self.packages.get(&key).is_none_or(|c| c.is_expired())
            })
            .cloned()
            .collect();

        if to_fetch.is_empty() {
            return;
        }

        debug!(count = to_fetch.len(), "prefetching packages");

        // Fetch in parallel
        let results: Vec<_> = to_fetch
            .par_iter()
            .filter_map(|name| self.source.fetch(name).map(|e| (name.clone(), e)))
            .collect();

        // Insert into cache
        for (name, entry) in results {
            self.track_virtuals(&entry);
            let key: Arc<str> = Arc::from(name.as_str());
            self.packages
                .insert(key, CachedEntry::new(entry, self.config.default_ttl));
        }
    }

    /// Clear all caches.
    pub fn clear(&self) {
        self.packages.clear();
        self.constraint_cache.clear();
        self.virtual_packages.clear();
        self.replacements.clear();
        self.stats.reset();
    }

    /// Get cache statistics summary.
    #[must_use]
    pub fn cache_summary(&self) -> CacheSummary {
        CacheSummary {
            cached_packages: self.packages.len(),
            cached_constraints: self.constraint_cache.len(),
            virtual_packages: self.virtual_packages.len(),
            replacements: self.replacements.len(),
            hit_rate: self.stats.hit_rate(),
            constraint_hit_rate: self.stats.constraint_hit_rate(),
            total_operations: self.stats.total_operations(),
        }
    }

    /// Track virtual packages and replacements from a package entry.
    fn track_virtuals(&self, entry: &PackageEntry) {
        for version in &entry.versions {
            // Track provides
            for provide in &version.provides {
                self.virtual_packages
                    .entry(Arc::from(provide.name.as_str()))
                    .or_default()
                    .push((entry.name.clone(), version.version.clone()));
            }

            // Track replaces
            for replace in &version.replaces {
                self.replacements
                    .entry(Arc::from(replace.name.as_str()))
                    .or_default()
                    .push((entry.name.clone(), version.version.clone()));
            }
        }
    }

    /// Evict packages if cache is too large.
    fn maybe_evict_packages(&self) {
        if self.packages.len() >= self.config.max_cached_packages {
            // Remove oldest 25% of entries
            let to_remove = self.config.max_cached_packages / 4;
            let mut entries: Vec<_> = self
                .packages
                .iter()
                .map(|r| (r.key().clone(), r.value().cached_at))
                .collect();

            entries.sort_by_key(|(_, time)| *time);

            for (key, _) in entries.into_iter().take(to_remove) {
                self.packages.remove(&key);
            }

            debug!(removed = to_remove, "evicted package cache entries");
        }
    }

    /// Evict constraint cache entries if too large.
    fn maybe_evict_constraints(&self) {
        if self.constraint_cache.len() >= self.config.max_constraint_cache {
            // Remove oldest 25% of entries
            let to_remove = self.config.max_constraint_cache / 4;
            let mut entries: Vec<_> = self
                .constraint_cache
                .iter()
                .map(|r| (r.key().clone(), r.value().cached_at))
                .collect();

            entries.sort_by_key(|(_, time)| *time);

            for (key, _) in entries.into_iter().take(to_remove) {
                self.constraint_cache.remove(&key);
            }

            debug!(removed = to_remove, "evicted constraint cache entries");
        }
    }
}

/// Summary of cache state.
#[derive(Debug, Clone)]
pub struct CacheSummary {
    /// Number of cached packages.
    pub cached_packages: usize,
    /// Number of cached constraint evaluations.
    pub cached_constraints: usize,
    /// Number of tracked virtual packages.
    pub virtual_packages: usize,
    /// Number of tracked replacements.
    pub replacements: usize,
    /// Package cache hit rate.
    pub hit_rate: f64,
    /// Constraint cache hit rate.
    pub constraint_hit_rate: f64,
    /// Total cache operations.
    pub total_operations: u64,
}

/// In-memory package source for testing.
#[derive(Debug, Default)]
pub struct MemorySource {
    packages: RwLock<AHashMap<Arc<str>, PackageEntry>>,
}

impl MemorySource {
    /// Create a new empty memory source.
    #[must_use]
    pub fn new() -> Self {
        Self {
            packages: RwLock::new(AHashMap::new()),
        }
    }

    /// Add a package entry, merging with an existing entry if present.
    pub fn add(&self, entry: PackageEntry) {
        let key = Arc::from(entry.name.as_str());
        let mut packages = self.packages.write();

        if let Some(existing) = packages.get_mut(&key) {
            // Merge versions from new entry into existing
            for version in entry.versions {
                // Only add if this version doesn't already exist
                if !existing
                    .versions
                    .iter()
                    .any(|v| v.version == version.version)
                {
                    existing.add_version(version);
                }
            }
            existing.sort_versions();
        } else {
            // No existing entry, just insert
            packages.insert(key, entry);
        }
    }

    /// Add a simple package version.
    ///
    /// # Panics
    ///
    /// Panics if `name` is not a valid package name or `version` is not a valid
    /// semantic version string.
    pub fn add_version(&self, name: &str, version: &str, deps: Vec<(&str, &str)>) {
        let pkg_name = PackageName::parse(name).expect("valid package name");
        let pkg_version = ComposerVersion::parse(version).expect("valid version");

        let mut packages = self.packages.write();
        let key = Arc::from(name);

        let entry = packages
            .entry(key)
            .or_insert_with(|| PackageEntry::new(pkg_name.clone()));

        let mut ver = PackageVersion::new(pkg_name, pkg_version);

        for (dep_name, dep_constraint) in deps {
            if let (Some(name), Some(constraint)) = (
                PackageName::parse(dep_name),
                ComposerConstraint::parse(dep_constraint),
            ) {
                ver.add_dependency(Dependency::new(name, constraint));
            }
        }

        entry.add_version(ver);
        entry.sort_versions();
    }

    /// Get the number of packages.
    #[must_use]
    pub fn len(&self) -> usize {
        self.packages.read().len()
    }

    /// Check if empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.packages.read().is_empty()
    }
}

impl PackageSource for MemorySource {
    fn fetch(&self, name: &PackageName) -> Option<PackageEntry> {
        let key: Arc<str> = Arc::from(name.as_str());
        self.packages.read().get(&key).cloned()
    }

    fn exists(&self, name: &PackageName) -> bool {
        let key: Arc<str> = Arc::from(name.as_str());
        self.packages.read().contains_key(&key)
    }

    fn fetch_batch(&self, names: &[PackageName]) -> Vec<Option<PackageEntry>> {
        let packages = self.packages.read();
        names
            .iter()
            .map(|n| {
                let key: Arc<str> = Arc::from(n.as_str());
                packages.get(&key).cloned()
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_index() -> PackageIndex<MemorySource> {
        let source = MemorySource::new();
        source.add_version("test/a", "1.0.0", vec![]);
        source.add_version("test/a", "1.1.0", vec![]);
        source.add_version("test/a", "2.0.0", vec![]);
        source.add_version("test/b", "1.0.0", vec![("test/a", "^1.0")]);

        PackageIndex::new(source)
    }

    #[test]
    fn test_get_package() {
        let index = create_test_index();
        let name = PackageName::parse("test/a").unwrap();

        let entry = index.get(&name).unwrap();
        assert_eq!(entry.versions.len(), 3);
    }

    #[test]
    fn test_cache_hit() {
        let index = create_test_index();
        let name = PackageName::parse("test/a").unwrap();

        // First access - miss
        let _ = index.get(&name);
        assert_eq!(index.stats.misses.load(Ordering::Relaxed), 1);

        // Second access - hit
        let _ = index.get(&name);
        assert_eq!(index.stats.hits.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_matching_versions() {
        let index = create_test_index();
        let name = PackageName::parse("test/a").unwrap();
        let constraint = ComposerConstraint::parse("^1.0").unwrap();

        let versions = index.get_matching_versions(&name, &constraint);
        assert_eq!(versions.len(), 2);
        assert!(versions.iter().all(|v| v.major == 1));
    }

    #[test]
    fn test_highest_matching() {
        let index = create_test_index();
        let name = PackageName::parse("test/a").unwrap();
        let constraint = ComposerConstraint::parse("^1.0").unwrap();

        let version = index.get_highest_matching(&name, &constraint).unwrap();
        assert_eq!(version.minor, 1);
    }

    #[test]
    fn test_lowest_matching() {
        let index = create_test_index();
        let name = PackageName::parse("test/a").unwrap();
        let constraint = ComposerConstraint::parse("^1.0").unwrap();

        let version = index.get_lowest_matching(&name, &constraint).unwrap();
        assert_eq!(version.minor, 0);
    }

    #[test]
    fn test_prefetch() {
        let index = create_test_index();
        let names: Vec<_> = vec!["test/a", "test/b"]
            .into_iter()
            .filter_map(PackageName::parse)
            .collect();

        index.prefetch(&names);

        // Both should now be cached
        assert_eq!(index.packages.len(), 2);
    }

    #[test]
    fn test_cache_summary() {
        let index = create_test_index();
        let name = PackageName::parse("test/a").unwrap();

        let _ = index.get(&name);
        let _ = index.get(&name);

        let summary = index.cache_summary();
        assert_eq!(summary.cached_packages, 1);
        assert!(summary.hit_rate > 0.0);
    }
}
