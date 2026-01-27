//! Cache configuration.

use std::path::PathBuf;
use std::time::Duration;

/// Cache configuration.
#[derive(Debug, Clone)]
pub struct CacheConfig {
    /// Root directory for cache storage.
    pub root: Option<PathBuf>,

    /// L1 (memory) cache size limit in bytes (default: 256MB).
    pub l1_size_limit: u64,

    /// L2 (disk) cache size limit in bytes (default: 10GB).
    pub l2_size_limit: u64,

    /// Default TTL for cached entries.
    pub default_ttl: Duration,

    /// TTL for package metadata.
    pub metadata_ttl: Duration,

    /// TTL for repository data.
    pub repository_ttl: Duration,

    /// TTL for resolved dependency graphs.
    pub graph_ttl: Duration,

    /// Zstd compression level (0-22, default: 3).
    pub compression_level: i32,

    /// Enable compression for cached data.
    pub compression_enabled: bool,

    /// Enable bloom filter for fast "not cached" checks.
    pub bloom_filter_enabled: bool,

    /// Expected number of items for bloom filter sizing.
    pub bloom_filter_capacity: usize,

    /// False positive rate for bloom filter (default: 0.01 = 1%).
    pub bloom_filter_fp_rate: f64,

    /// Enable background cache warming on startup.
    pub warm_on_startup: bool,

    /// Maximum entries to warm from disk on startup.
    pub max_warm_entries: usize,

    /// Garbage collection interval.
    pub gc_interval: Duration,

    /// Enable statistics tracking.
    pub stats_enabled: bool,

    /// Memory map threshold (files larger than this use mmap).
    pub mmap_threshold: u64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            root: None,
            l1_size_limit: 256 * 1024 * 1024,                // 256MB
            l2_size_limit: 10 * 1024 * 1024 * 1024,          // 10GB
            default_ttl: Duration::from_secs(7 * 24 * 3600), // 7 days
            metadata_ttl: Duration::from_secs(3600),         // 1 hour
            repository_ttl: Duration::from_secs(300),        // 5 minutes
            graph_ttl: Duration::from_secs(24 * 3600),       // 1 day
            compression_level: 3,
            compression_enabled: true,
            bloom_filter_enabled: true,
            bloom_filter_capacity: 100_000,
            bloom_filter_fp_rate: 0.01,
            warm_on_startup: true,
            max_warm_entries: 10_000,
            gc_interval: Duration::from_secs(3600), // 1 hour
            stats_enabled: true,
            mmap_threshold: 10 * 1024 * 1024, // 10MB
        }
    }
}

impl CacheConfig {
    /// Create a new cache configuration builder.
    #[must_use]
    pub fn builder() -> CacheConfigBuilder {
        CacheConfigBuilder::default()
    }
}

/// Builder for cache configuration.
#[derive(Debug, Default)]
pub struct CacheConfigBuilder {
    config: CacheConfig,
}

impl CacheConfigBuilder {
    /// Set the cache root directory.
    #[must_use]
    pub fn root(mut self, root: PathBuf) -> Self {
        self.config.root = Some(root);
        self
    }

    /// Set L1 (memory) cache size limit.
    #[must_use]
    pub fn l1_size_limit(mut self, limit: u64) -> Self {
        self.config.l1_size_limit = limit;
        self
    }

    /// Set L2 (disk) cache size limit.
    #[must_use]
    pub fn l2_size_limit(mut self, limit: u64) -> Self {
        self.config.l2_size_limit = limit;
        self
    }

    /// Set default TTL for cached entries.
    #[must_use]
    pub fn default_ttl(mut self, ttl: Duration) -> Self {
        self.config.default_ttl = ttl;
        self
    }

    /// Set TTL for package metadata.
    #[must_use]
    pub fn metadata_ttl(mut self, ttl: Duration) -> Self {
        self.config.metadata_ttl = ttl;
        self
    }

    /// Set TTL for repository data.
    #[must_use]
    pub fn repository_ttl(mut self, ttl: Duration) -> Self {
        self.config.repository_ttl = ttl;
        self
    }

    /// Set TTL for resolved dependency graphs.
    #[must_use]
    pub fn graph_ttl(mut self, ttl: Duration) -> Self {
        self.config.graph_ttl = ttl;
        self
    }

    /// Set zstd compression level.
    #[must_use]
    pub fn compression_level(mut self, level: i32) -> Self {
        self.config.compression_level = level.clamp(0, 22);
        self
    }

    /// Enable or disable compression.
    #[must_use]
    pub fn compression_enabled(mut self, enabled: bool) -> Self {
        self.config.compression_enabled = enabled;
        self
    }

    /// Enable or disable bloom filter.
    #[must_use]
    pub fn bloom_filter_enabled(mut self, enabled: bool) -> Self {
        self.config.bloom_filter_enabled = enabled;
        self
    }

    /// Set bloom filter capacity.
    #[must_use]
    pub fn bloom_filter_capacity(mut self, capacity: usize) -> Self {
        self.config.bloom_filter_capacity = capacity;
        self
    }

    /// Enable or disable cache warming on startup.
    #[must_use]
    pub fn warm_on_startup(mut self, enabled: bool) -> Self {
        self.config.warm_on_startup = enabled;
        self
    }

    /// Set maximum entries to warm on startup.
    #[must_use]
    pub fn max_warm_entries(mut self, max: usize) -> Self {
        self.config.max_warm_entries = max;
        self
    }

    /// Set garbage collection interval.
    #[must_use]
    pub fn gc_interval(mut self, interval: Duration) -> Self {
        self.config.gc_interval = interval;
        self
    }

    /// Enable or disable statistics tracking.
    #[must_use]
    pub fn stats_enabled(mut self, enabled: bool) -> Self {
        self.config.stats_enabled = enabled;
        self
    }

    /// Set memory map threshold.
    #[must_use]
    pub fn mmap_threshold(mut self, threshold: u64) -> Self {
        self.config.mmap_threshold = threshold;
        self
    }

    /// Build the configuration.
    #[must_use]
    pub fn build(self) -> CacheConfig {
        self.config
    }
}

/// Cache entry type for TTL selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CacheEntryType {
    /// Package archive.
    Package,
    /// Package metadata (composer.json, etc.).
    Metadata,
    /// Repository index data.
    Repository,
    /// Resolved dependency graph.
    DependencyGraph,
    /// Generated autoloader.
    Autoloader,
    /// VCS clone (bare repository).
    VcsClone,
}

impl CacheEntryType {
    /// Get TTL for this entry type from config.
    #[must_use]
    pub fn ttl(self, config: &CacheConfig) -> Duration {
        match self {
            Self::Package => config.default_ttl,
            Self::Metadata => config.metadata_ttl,
            Self::Repository => config.repository_ttl,
            Self::DependencyGraph => config.graph_ttl,
            Self::Autoloader => config.default_ttl,
            Self::VcsClone => config.default_ttl,
        }
    }

    /// Get subdirectory name for this entry type.
    #[must_use]
    pub const fn subdir(self) -> &'static str {
        match self {
            Self::Package => "packages",
            Self::Metadata => "metadata",
            Self::Repository => "repos",
            Self::DependencyGraph => "graphs",
            Self::Autoloader => "autoload",
            Self::VcsClone => "vcs",
        }
    }
}
