//! `DependencyProvider` implementation for Composer packages.
//!
//! This module implements the `pubgrub::DependencyProvider` trait for
//! Composer packages, enabling the `PubGrub` algorithm to resolve dependencies.

use crate::index::{PackageIndex, PackageSource};
use crate::package::PackageName;
use crate::version::{ComposerConstraint, ComposerVersion, Stability};
use ahash::AHashSet;
use parking_lot::Mutex;
use pubgrub::{
    Dependencies, DependencyConstraints, DependencyProvider, PackageResolutionStatistics,
};
use std::cmp::Reverse;
use std::convert::Infallible;
use std::fmt;
use std::sync::Arc;
use tracing::{debug, trace, warn};
use version_ranges::Ranges;

/// Resolution mode determining version selection strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ResolutionMode {
    /// Prefer highest compatible versions (default).
    #[default]
    PreferHighest,
    /// Prefer lowest compatible versions (for testing).
    PreferLowest,
    /// Prefer stable versions over pre-release.
    PreferStable,
}

/// Configuration for the dependency provider.
#[derive(Debug, Clone)]
pub struct ProviderConfig {
    /// Resolution mode.
    pub mode: ResolutionMode,
    /// Minimum stability to accept.
    pub min_stability: Stability,
    /// Include dev dependencies.
    pub include_dev: bool,
    /// Maximum number of versions to consider per package.
    pub max_versions_per_package: usize,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            mode: ResolutionMode::PreferHighest,
            min_stability: Stability::Stable,
            include_dev: false,
            max_versions_per_package: 100,
        }
    }
}

/// Error type for the provider.
#[derive(Debug, Clone)]
pub enum ProviderError {
    /// Package not found.
    PackageNotFound(String),
    /// Resolution cancelled.
    Cancelled,
    /// Internal error.
    Internal(String),
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PackageNotFound(name) => write!(f, "package not found: {name}"),
            Self::Cancelled => write!(f, "resolution cancelled"),
            Self::Internal(msg) => write!(f, "internal error: {msg}"),
        }
    }
}

impl std::error::Error for ProviderError {}

/// Custom incompatibility reasons.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IncompatibilityReason {
    /// Package is a platform requirement (php, ext-*, lib-*).
    PlatformRequirement(String),
    /// Package conflicts with another.
    Conflict(String, String),
    /// Package was explicitly excluded.
    Excluded(String),
}

impl fmt::Display for IncompatibilityReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PlatformRequirement(pkg) => {
                write!(f, "{pkg} is a platform requirement (not resolved)")
            }
            Self::Conflict(a, b) => write!(f, "{a} conflicts with {b}"),
            Self::Excluded(pkg) => write!(f, "{pkg} was explicitly excluded"),
        }
    }
}

/// The Composer dependency provider.
///
/// This implements `pubgrub::DependencyProvider` for Composer packages.
pub struct ComposerProvider<S: PackageSource> {
    /// Package index.
    index: Arc<PackageIndex<S>>,
    /// Configuration.
    config: ProviderConfig,
    /// Packages that have been requested (for prioritization).
    requested_packages: Mutex<AHashSet<Arc<str>>>,
    /// Platform packages to skip.
    platform_packages: Mutex<AHashSet<Arc<str>>>,
    /// Explicitly excluded packages.
    excluded_packages: AHashSet<Arc<str>>,
    /// Locked package versions (from lock file).
    locked_versions: Mutex<ahash::AHashMap<Arc<str>, ComposerVersion>>,
    /// Root package dependencies (set before resolution).
    root_dependencies: Mutex<DependencyConstraints<PackageName, Ranges<ComposerVersion>>>,
}

impl<S: PackageSource> std::fmt::Debug for ComposerProvider<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ComposerProvider")
            .field("config", &self.config)
            .field("excluded_packages", &self.excluded_packages.len())
            .field("locked_versions", &self.locked_versions.lock().len())
            .finish_non_exhaustive()
    }
}

impl<S: PackageSource> ComposerProvider<S> {
    /// Create a new provider with the given index.
    pub fn new(index: Arc<PackageIndex<S>>, config: ProviderConfig) -> Self {
        Self {
            index,
            config,
            requested_packages: Mutex::new(AHashSet::new()),
            platform_packages: Mutex::new(AHashSet::new()),
            excluded_packages: AHashSet::new(),
            locked_versions: Mutex::new(ahash::AHashMap::new()),
            root_dependencies: Mutex::new(DependencyConstraints::default()),
        }
    }

    /// Exclude a package from resolution.
    pub fn exclude(&mut self, package: &str) {
        self.excluded_packages.insert(Arc::from(package));
    }

    /// Lock a package to a specific version.
    pub fn lock_version(&self, package: &PackageName, version: ComposerVersion) {
        self.locked_versions
            .lock()
            .insert(Arc::from(package.as_str()), version);
    }

    /// Get the locked version for a package, if any.
    fn get_locked_version(&self, package: &str) -> Option<ComposerVersion> {
        self.locked_versions.lock().get(package).cloned()
    }

    /// Set root dependencies (called before resolution).
    pub fn set_root_dependencies(
        &self,
        deps: impl IntoIterator<Item = (PackageName, Ranges<ComposerVersion>)>,
    ) {
        let mut root_deps = self.root_dependencies.lock();
        root_deps.clear();
        for (name, ranges) in deps {
            root_deps.insert(name, ranges);
        }
    }

    /// Check if a package is a platform package.
    fn is_platform_package(name: &str) -> bool {
        libretto_core::is_platform_package_name(name)
    }

    /// Record that a package was requested (for prioritization).
    fn record_requested(&self, name: &str) {
        self.requested_packages.lock().insert(Arc::from(name));
    }

    /// Record a platform package.
    fn record_platform(&self, name: &str) {
        self.platform_packages.lock().insert(Arc::from(name));
    }

    /// Get all platform packages encountered.
    pub fn platform_packages(&self) -> Vec<Arc<str>> {
        self.platform_packages.lock().iter().cloned().collect()
    }

    /// Filter and sort versions according to configuration.
    fn filter_versions(&self, versions: Vec<ComposerVersion>) -> Vec<ComposerVersion> {
        let mut filtered: Vec<_> = versions
            .into_iter()
            .filter(|v| v.stability.satisfies_minimum(self.config.min_stability))
            .take(self.config.max_versions_per_package)
            .collect();

        match self.config.mode {
            ResolutionMode::PreferHighest => {
                filtered.sort_by(|a, b| b.cmp(a));
            }
            ResolutionMode::PreferLowest => {
                filtered.sort();
            }
            ResolutionMode::PreferStable => {
                filtered.sort_by(|a, b| match (a.is_prerelease(), b.is_prerelease()) {
                    (true, false) => std::cmp::Ordering::Greater,
                    (false, true) => std::cmp::Ordering::Less,
                    _ => b.cmp(a),
                });
            }
        }

        filtered
    }
}

impl<S: PackageSource + 'static> DependencyProvider for ComposerProvider<S> {
    type P = PackageName;
    type V = ComposerVersion;
    type VS = Ranges<ComposerVersion>;
    type M = IncompatibilityReason;
    type Err = Infallible;
    type Priority = PackagePriority;

    fn prioritize(
        &self,
        package: &PackageName,
        range: &Ranges<ComposerVersion>,
        _stats: &PackageResolutionStatistics,
    ) -> PackagePriority {
        self.record_requested(package.as_str());

        // Count matching versions
        let constraint = ranges_to_constraint(range);
        let versions = self.index.get_matching_versions(package, &constraint);
        let version_count = versions.len();

        // Check if this was a root dependency
        let is_root = self.requested_packages.lock().contains(package.as_str());

        PackagePriority {
            version_count: Reverse(version_count),
            is_root,
        }
    }

    fn choose_version(
        &self,
        package: &PackageName,
        range: &Ranges<ComposerVersion>,
    ) -> Result<Option<ComposerVersion>, Infallible> {
        trace!(package = %package, "choosing version");

        // Handle root package - always return version 1.0.0
        if package.as_str() == "__root__/__root__" {
            let root_version = ComposerVersion::new(1, 0, 0);
            if range.contains(&root_version) {
                return Ok(Some(root_version));
            }
            return Ok(None);
        }

        // Check exclusions
        if self.excluded_packages.contains(package.as_str()) {
            return Ok(None);
        }

        // Check if platform package
        if Self::is_platform_package(package.as_str()) {
            self.record_platform(package.as_str());
            return Ok(None);
        }

        // Check for locked version first
        if let Some(locked) = self.get_locked_version(package.as_str()) {
            if range.contains(&locked) {
                trace!(package = %package, version = %locked, "using locked version");
                return Ok(Some(locked));
            }
            // Locked version doesn't satisfy range - conflict will be detected
            debug!(
                package = %package,
                locked = %locked,
                "locked version does not satisfy constraint"
            );
        }

        // Get all versions for this package
        let Some(entry) = self.index.get(package) else {
            debug!(package = %package, "package not found");
            return Ok(None);
        };

        // Filter versions that satisfy the range constraint
        let versions: Vec<ComposerVersion> = entry
            .versions
            .iter()
            .map(|v| v.version.clone())
            .filter(|v| range.contains(v))
            .collect();

        if versions.is_empty() {
            debug!(package = %package, "no versions match constraint");
            return Ok(None);
        }

        // Filter and sort according to config
        let sorted = self.filter_versions(versions);

        trace!(
            package = %package,
            count = sorted.len(),
            selected = ?sorted.first(),
            "selected version"
        );

        Ok(sorted.into_iter().next())
    }

    fn get_dependencies(
        &self,
        package: &PackageName,
        version: &ComposerVersion,
    ) -> Result<Dependencies<PackageName, Ranges<ComposerVersion>, IncompatibilityReason>, Infallible>
    {
        trace!(package = %package, version = %version, "getting dependencies");

        // Handle root package specially
        if package.as_str() == "__root__/__root__" {
            let root_deps = self.root_dependencies.lock().clone();
            return Ok(Dependencies::Available(root_deps));
        }

        // Check if platform package
        if Self::is_platform_package(package.as_str()) {
            return Ok(Dependencies::Available(DependencyConstraints::default()));
        }

        // Get dependencies from index
        let Some(deps) = self.index.get_dependencies(package, version) else {
            warn!(package = %package, version = %version, "dependencies not found");
            return Ok(Dependencies::Available(DependencyConstraints::default()));
        };

        // Convert to pubgrub format
        let mut result: DependencyConstraints<PackageName, Ranges<ComposerVersion>> =
            DependencyConstraints::default();

        for dep in deps {
            // Skip platform packages (they're handled externally)
            if Self::is_platform_package(dep.name.as_str()) {
                self.record_platform(dep.name.as_str());
                continue;
            }

            // Include all dependencies, even excluded ones
            // Exclusion is handled in choose_version by returning None,
            // which will cause pubgrub to report an unsatisfiable constraint
            let ranges = constraint_to_ranges(&dep.constraint);
            result.insert(dep.name, ranges);
        }

        trace!(
            package = %package,
            version = %version,
            dep_count = result.len(),
            "resolved dependencies"
        );

        Ok(Dependencies::Available(result))
    }
}

/// Priority for package selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackagePriority {
    /// Fewer versions = higher priority.
    version_count: Reverse<usize>,
    /// Root dependencies get priority.
    is_root: bool,
}

impl PartialOrd for PackagePriority {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PackagePriority {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (self.is_root, other.is_root) {
            (true, false) => std::cmp::Ordering::Greater,
            (false, true) => std::cmp::Ordering::Less,
            _ => self.version_count.cmp(&other.version_count),
        }
    }
}

/// Convert pubgrub Ranges to `ComposerConstraint`.
fn ranges_to_constraint(ranges: &Ranges<ComposerVersion>) -> ComposerConstraint {
    if ranges.is_empty() {
        return ComposerConstraint::empty();
    }
    ComposerConstraint::any()
}

/// Convert `ComposerConstraint` to pubgrub Ranges.
fn constraint_to_ranges(constraint: &ComposerConstraint) -> Ranges<ComposerVersion> {
    constraint.ranges().clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::MemorySource;

    fn create_test_provider() -> ComposerProvider<MemorySource> {
        let source = MemorySource::new();
        source.add_version("test/a", "1.0.0", vec![]);
        source.add_version("test/a", "1.1.0", vec![]);
        source.add_version("test/a", "2.0.0", vec![]);
        source.add_version("test/b", "1.0.0", vec![("test/a", "^1.0")]);
        source.add_version("test/c", "1.0.0", vec![("test/a", "^2.0")]);

        let index = Arc::new(PackageIndex::new(source));
        ComposerProvider::new(index, ProviderConfig::default())
    }

    #[test]
    fn test_choose_version_highest() {
        let provider = create_test_provider();
        let pkg = PackageName::parse("test/a").unwrap();
        let range = Ranges::full();

        let version = provider.choose_version(&pkg, &range).unwrap().unwrap();
        assert_eq!(version.major, 2);
    }

    #[test]
    fn test_choose_version_lowest() {
        let source = MemorySource::new();
        source.add_version("test/a", "1.0.0", vec![]);
        source.add_version("test/a", "2.0.0", vec![]);

        let index = Arc::new(PackageIndex::new(source));
        let config = ProviderConfig {
            mode: ResolutionMode::PreferLowest,
            ..Default::default()
        };
        let provider = ComposerProvider::new(index, config);

        let pkg = PackageName::parse("test/a").unwrap();
        let range = Ranges::full();

        let version = provider.choose_version(&pkg, &range).unwrap().unwrap();
        assert_eq!(version.major, 1);
    }

    #[test]
    fn test_platform_packages() {
        assert!(ComposerProvider::<MemorySource>::is_platform_package("php"));
        assert!(ComposerProvider::<MemorySource>::is_platform_package(
            "ext-json"
        ));
        assert!(ComposerProvider::<MemorySource>::is_platform_package(
            "lib-openssl"
        ));
        assert!(!ComposerProvider::<MemorySource>::is_platform_package(
            "php-open-source-saver/jwt-auth"
        ));
        assert!(!ComposerProvider::<MemorySource>::is_platform_package(
            "test/pkg"
        ));
    }

    #[test]
    fn test_excluded_packages() {
        let source = MemorySource::new();
        source.add_version("test/a", "1.0.0", vec![]);

        let index = Arc::new(PackageIndex::new(source));
        let mut provider = ComposerProvider::new(index, ProviderConfig::default());
        provider.exclude("test/a");

        let pkg = PackageName::parse("test/a").unwrap();
        let range = Ranges::full();

        let version = provider.choose_version(&pkg, &range).unwrap();
        assert!(version.is_none());
    }

    #[test]
    fn test_get_dependencies() {
        let provider = create_test_provider();
        let pkg = PackageName::parse("test/b").unwrap();
        let version = ComposerVersion::parse("1.0.0").unwrap();

        let deps = provider.get_dependencies(&pkg, &version).unwrap();

        match deps {
            Dependencies::Available(d) => {
                assert_eq!(d.len(), 1);
            }
            Dependencies::Unavailable(_) => panic!("expected available dependencies"),
        }
    }
}
