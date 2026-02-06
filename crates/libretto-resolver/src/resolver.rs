//! High-performance dependency resolver using `PubGrub` algorithm.
//!
//! This is the main resolver implementation for Libretto. It uses streaming
//! parallel fetching combined with the `PubGrub` algorithm for efficient and
//! correct dependency resolution.
//!
//! # Key Features
//!
//! - **Streaming fetch**: Process packages as they arrive, don't wait for batches
//! - **Parallel prefetch**: Start fetching dependencies before parent completes
//! - **Request deduplication**: Never fetch the same package twice
//! - **HTTP/2 multiplexing**: Reuse connections aggressively
//! - **`PubGrub` solver**: Battle-tested algorithm with conflict-driven learning
//!
//! # Example
//!
//! ```rust,ignore
//! use libretto_resolver::{Resolver, ResolverConfig, PackageFetcher};
//!
//! let fetcher = MyFetcher::new();
//! let config = ResolverConfig::default();
//! let resolver = Resolver::new(Arc::new(fetcher), config);
//!
//! let resolution = resolver.resolve(&root_deps, &dev_deps).await?;
//! ```

use crate::fetcher::{FetchedPackage, PackageFetcher};
use crate::package::{Dependency, PackageEntry, PackageName, PackageVersion};
use crate::provider::ResolutionMode;
use crate::types::{Resolution, ResolveError, ResolvedPackage};
use crate::version::{ComposerConstraint, ComposerVersion, Stability};
use ahash::{AHashMap, AHashSet};
use dashmap::DashSet;
use futures::stream::{FuturesUnordered, StreamExt};
use petgraph::Direction;
use petgraph::graph::{DiGraph, NodeIndex};
use pubgrub::{
    DefaultStringReporter, Dependencies, DependencyConstraints, DependencyProvider,
    PackageResolutionStatistics, PubGrubError, Reporter, resolve,
};
use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tracing::info;
use version_ranges::Ranges;

/// Resolver statistics for monitoring and debugging.
#[derive(Debug, Default)]
pub struct ResolverStats {
    /// Number of packages fetched from repository.
    pub packages_fetched: AtomicU64,
    /// Total number of versions processed.
    pub versions_total: AtomicU64,
    /// Time spent fetching metadata (ms).
    pub fetch_time_ms: AtomicU64,
    /// Time spent in `PubGrub` solver (ms).
    pub solver_time_ms: AtomicU64,
    /// Total HTTP requests made.
    pub requests_total: AtomicU64,
    /// Failed HTTP requests.
    pub requests_failed: AtomicU64,
}

/// Resolver configuration.
#[derive(Debug, Clone)]
pub struct ResolverConfig {
    /// Maximum concurrent HTTP requests.
    pub max_concurrent: usize,
    /// Timeout per individual request.
    pub request_timeout: Duration,
    /// Version selection strategy.
    pub mode: ResolutionMode,
    /// Minimum acceptable stability level.
    pub min_stability: Stability,
    /// Whether to include dev dependencies.
    pub include_dev: bool,
}

impl Default for ResolverConfig {
    fn default() -> Self {
        Self {
            max_concurrent: 32,
            request_timeout: Duration::from_secs(10),
            mode: ResolutionMode::PreferStable,
            min_stability: Stability::Stable,
            include_dev: true,
        }
    }
}

/// The main dependency resolver.
///
/// Uses streaming parallel fetching combined with `PubGrub` for fast,
/// correct dependency resolution.
pub struct Resolver<F: PackageFetcher> {
    fetcher: Arc<F>,
    config: ResolverConfig,
    stats: Arc<ResolverStats>,
}

impl<F: PackageFetcher> std::fmt::Debug for Resolver<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Resolver")
            .field("config", &self.config)
            .field("stats", &self.stats)
            .finish_non_exhaustive()
    }
}

impl<F: PackageFetcher> Resolver<F> {
    /// Create a new resolver with the given fetcher and configuration.
    pub fn new(fetcher: Arc<F>, config: ResolverConfig) -> Self {
        Self {
            fetcher,
            config,
            stats: Arc::new(ResolverStats::default()),
        }
    }

    /// Get resolver statistics.
    #[must_use]
    pub fn stats(&self) -> &ResolverStats {
        &self.stats
    }

    /// Resolve dependencies.
    ///
    /// Takes root dependencies and dev dependencies, returns a complete
    /// resolution with all transitive dependencies in topological order.
    pub async fn resolve(
        &self,
        root_deps: &[Dependency],
        dev_deps: &[Dependency],
    ) -> Result<Resolution, ResolveError> {
        let start = Instant::now();

        // Phase 1: Stream-fetch all reachable packages
        let fetch_start = Instant::now();
        let packages = self.fetch_all_packages(root_deps, dev_deps).await?;
        self.stats
            .fetch_time_ms
            .store(fetch_start.elapsed().as_millis() as u64, Ordering::Relaxed);

        info!(
            packages = packages.len(),
            fetch_ms = fetch_start.elapsed().as_millis(),
            requests = self.stats.requests_total.load(Ordering::Relaxed),
            failed = self.stats.requests_failed.load(Ordering::Relaxed),
            "fetch complete"
        );

        // Phase 2: Run PubGrub solver
        let solver_start = Instant::now();
        let resolution = self.solve(root_deps, dev_deps, packages)?;
        self.stats
            .solver_time_ms
            .store(solver_start.elapsed().as_millis() as u64, Ordering::Relaxed);

        info!(
            total_ms = start.elapsed().as_millis(),
            packages = resolution.packages.len(),
            "resolution complete"
        );

        Ok(resolution)
    }

    /// Fetch all reachable packages using streaming parallel requests.
    async fn fetch_all_packages(
        &self,
        root_deps: &[Dependency],
        dev_deps: &[Dependency],
    ) -> Result<AHashMap<String, PackageEntry>, ResolveError> {
        let packages: Arc<dashmap::DashMap<String, PackageEntry>> =
            Arc::new(dashmap::DashMap::new());
        let seen: Arc<DashSet<String>> = Arc::new(DashSet::new());

        // Collect initial dependencies
        let all_deps: Vec<_> = root_deps
            .iter()
            .chain(if self.config.include_dev {
                dev_deps
            } else {
                &[]
            })
            .collect();

        let mut pending: Vec<String> = Vec::new();
        for dep in &all_deps {
            let name = dep.name.as_str().to_string();
            if !is_platform_package(&name) && seen.insert(name.clone()) {
                pending.push(name);
            }
        }

        info!(initial = pending.len(), "fetch starting");

        let mut in_flight = FuturesUnordered::new();

        loop {
            // Launch new requests up to max_concurrent
            while in_flight.len() < self.config.max_concurrent && !pending.is_empty() {
                let name = pending.pop().unwrap();
                let fetcher = Arc::clone(&self.fetcher);
                let timeout = self.config.request_timeout;
                let stats = Arc::clone(&self.stats);

                in_flight.push(async move {
                    stats.requests_total.fetch_add(1, Ordering::Relaxed);
                    if let Ok(result) =
                        tokio::time::timeout(timeout, fetcher.fetch(name.clone())).await
                    {
                        (name, result)
                    } else {
                        stats.requests_failed.fetch_add(1, Ordering::Relaxed);
                        (name, None)
                    }
                });
            }

            // Done when nothing in flight and nothing pending
            if in_flight.is_empty() {
                break;
            }

            // Process next completed request
            if let Some((name, result)) = in_flight.next().await
                && let Some(fetched) = result
            {
                self.stats.packages_fetched.fetch_add(1, Ordering::Relaxed);

                if let Some(entry) = convert_fetched_package(&fetched, self.config.min_stability) {
                    self.stats
                        .versions_total
                        .fetch_add(entry.versions.len() as u64, Ordering::Relaxed);

                    // Queue newly discovered dependencies
                    for version in &entry.versions {
                        for dep in &version.dependencies {
                            let dep_name = dep.name.as_str().to_string();
                            if !is_platform_package(&dep_name) && seen.insert(dep_name.clone()) {
                                pending.push(dep_name);
                            }
                        }
                    }

                    packages.insert(name, entry);
                }
            }

            // Log progress periodically
            let fetched = self.stats.packages_fetched.load(Ordering::Relaxed);
            if fetched.is_multiple_of(50) && fetched > 0 {
                info!(
                    fetched,
                    in_flight = in_flight.len(),
                    pending = pending.len(),
                    "fetch progress"
                );
            }
        }

        info!(
            total = packages.len(),
            requests = self.stats.requests_total.load(Ordering::Relaxed),
            "fetch complete"
        );

        // Convert DashMap to HashMap
        Ok(packages
            .iter()
            .map(|r| (r.key().clone(), r.value().clone()))
            .collect())
    }

    /// Run `PubGrub` solver on fetched packages.
    ///
    /// Uses a two-pass approach to handle Composer's `replace` semantics:
    ///
    /// **Pass 1**: Resolve without replacement awareness to discover which
    /// package versions are selected. From those selected versions, collect
    /// their `replace` declarations to build an authoritative replacement set.
    ///
    /// **Pass 2**: Re-resolve with replaced packages filtered out of the
    /// dependency graph. This mirrors Composer's behavior where replaced
    /// packages are never independently selected by the solver.
    ///
    /// Pass 2 is near-instant since all metadata is already in memory.
    fn solve(
        &self,
        root_deps: &[Dependency],
        dev_deps: &[Dependency],
        packages: AHashMap<String, PackageEntry>,
    ) -> Result<Resolution, ResolveError> {
        let all_deps: Vec<_> = if self.config.include_dev {
            root_deps.iter().chain(dev_deps.iter()).cloned().collect()
        } else {
            root_deps.to_vec()
        };

        let root_dep_ranges: Vec<_> = all_deps
            .iter()
            .filter(|d| !is_platform_package(d.name.as_str()))
            .map(|d| (d.name.clone(), d.constraint.ranges().clone()))
            .collect();

        let root = PackageName::new("__root__", "__root__");
        let root_ver = ComposerVersion::new(1, 0, 0);

        // --- Pass 1: Resolve to discover replacements ---
        let provider = PubGrubProvider::new(
            packages.clone(),
            self.config.mode,
            self.config.min_stability,
            AHashSet::new(),
        );
        provider.set_root_deps(root_dep_ranges.clone());

        let solution = match resolve(&provider, root.clone(), root_ver.clone()) {
            Ok(s) => s,
            Err(PubGrubError::NoSolution(mut tree)) => {
                tree.collapse_no_versions();
                return Err(ResolveError::Conflict {
                    explanation: DefaultStringReporter::report(&tree),
                });
            }
            Err(PubGrubError::ErrorChoosingVersion { package, .. }) => {
                return Err(ResolveError::PackageNotFound {
                    name: package.to_string(),
                });
            }
            Err(_) => return Err(ResolveError::Cancelled),
        };

        // Collect replacements from the *selected* versions only
        let mut replaced: AHashSet<String> = AHashSet::new();
        for (name, version) in &solution {
            if name.as_str() == "__root__/__root__" {
                continue;
            }
            if let Some(info) = provider.version_info(name, version) {
                for rep in &info.replaces {
                    replaced.insert(rep.name.as_str().to_string());
                }
            }
        }

        // If nothing is replaced, skip pass 2 — use pass 1 result directly
        if replaced.is_empty() {
            return self.build_resolution(solution, &provider, dev_deps);
        }

        info!(
            replaced_count = replaced.len(),
            "re-resolving without replaced packages"
        );

        // --- Pass 2: Re-resolve with replaced packages excluded ---
        let provider2 = PubGrubProvider::new(
            packages,
            self.config.mode,
            self.config.min_stability,
            replaced,
        );
        provider2.set_root_deps(root_dep_ranges);

        match resolve(&provider2, root, root_ver) {
            Ok(solution) => self.build_resolution(solution, &provider2, dev_deps),
            Err(PubGrubError::NoSolution(mut tree)) => {
                tree.collapse_no_versions();
                Err(ResolveError::Conflict {
                    explanation: DefaultStringReporter::report(&tree),
                })
            }
            Err(PubGrubError::ErrorChoosingVersion { package, .. }) => {
                Err(ResolveError::PackageNotFound {
                    name: package.to_string(),
                })
            }
            Err(_) => Err(ResolveError::Cancelled),
        }
    }

    /// Build resolution result from `PubGrub` solution.
    fn build_resolution(
        &self,
        solution: impl IntoIterator<Item = (PackageName, ComposerVersion)>,
        provider: &PubGrubProvider,
        dev_deps: &[Dependency],
    ) -> Result<Resolution, ResolveError> {
        let dev_names: AHashSet<_> = dev_deps
            .iter()
            .map(|d| d.name.as_str().to_string())
            .collect();

        let mut graph: DiGraph<PackageName, ()> = DiGraph::new();
        let mut indices: AHashMap<String, NodeIndex> = AHashMap::new();
        let mut pkg_map: AHashMap<String, (PackageName, ComposerVersion)> = AHashMap::new();

        for (name, version) in solution {
            if name.as_str() == "__root__/__root__" {
                continue;
            }
            let key = name.as_str().to_string();
            let idx = graph.add_node(name.clone());
            indices.insert(key.clone(), idx);
            pkg_map.insert(key, (name, version));
        }

        // Collect packages that are replaced by other resolved packages.
        // For example, laravel/framework replaces illuminate/* — those should
        // not appear as separate entries in the resolution.
        let mut replaced: AHashSet<String> = AHashSet::new();
        for (_, (name, version)) in &pkg_map {
            if let Some(info) = provider.version_info(name, version) {
                for rep in &info.replaces {
                    let rep_name = rep.name.as_str();
                    if pkg_map.contains_key(rep_name) {
                        replaced.insert(rep_name.to_string());
                    }
                }
            }
        }

        // Remove replaced packages from the graph and maps
        for name in &replaced {
            if let Some(idx) = indices.remove(name) {
                graph.remove_node(idx);
                // petgraph may swap indices when removing nodes, so rebuild
            }
            pkg_map.remove(name);
        }

        // Rebuild graph after removals (petgraph node removal invalidates indices)
        if !replaced.is_empty() {
            graph = DiGraph::new();
            indices.clear();
            for (key, (name, _)) in &pkg_map {
                let idx = graph.add_node(name.clone());
                indices.insert(key.clone(), idx);
            }
        }

        // Add dependency edges
        for (key, (name, version)) in &pkg_map {
            if let Some(deps) = provider.deps_for(name, version) {
                let from = indices[key];
                for dep in deps {
                    if let Some(&to) = indices.get(dep.name.as_str()) {
                        graph.add_edge(to, from, ());
                    }
                }
            }
        }

        // Topological sort
        let packages = self.topological_sort(&graph, &indices, pkg_map, &dev_names, provider)?;

        Ok(Resolution {
            packages,
            graph,
            indices,
            platform_packages: vec![],
            duration: Duration::ZERO,
        })
    }

    /// Sort packages in topological order (dependencies first).
    fn topological_sort(
        &self,
        graph: &DiGraph<PackageName, ()>,
        indices: &AHashMap<String, NodeIndex>,
        mut pkg_map: AHashMap<String, (PackageName, ComposerVersion)>,
        dev_names: &AHashSet<String>,
        provider: &PubGrubProvider,
    ) -> Result<Vec<ResolvedPackage>, ResolveError> {
        let mut result = Vec::with_capacity(pkg_map.len());
        let mut in_degree: AHashMap<NodeIndex, usize> = AHashMap::new();

        for &idx in indices.values() {
            in_degree.insert(
                idx,
                graph.neighbors_directed(idx, Direction::Incoming).count(),
            );
        }

        let mut queue: Vec<_> = in_degree
            .iter()
            .filter(|(_, d)| **d == 0)
            .map(|(&i, _)| i)
            .collect();

        while !in_degree.is_empty() {
            if queue.is_empty() {
                // Cycle detected - pick minimum degree to break it
                if let Some((&idx, _)) = in_degree.iter().min_by_key(|(_, d)| *d) {
                    queue.push(idx);
                } else {
                    break;
                }
            }

            while let Some(idx) = queue.pop() {
                if in_degree.remove(&idx).is_none() {
                    continue;
                }

                let name = &graph[idx];
                let key = name.as_str();

                if let Some((pkg_name, version)) = pkg_map.remove(key) {
                    let resolved = build_resolved_package(
                        pkg_name,
                        version,
                        graph,
                        idx,
                        dev_names.contains(key),
                        provider,
                    );
                    result.push(resolved);
                }

                for neighbor in graph.neighbors_directed(idx, Direction::Outgoing) {
                    if let Some(deg) = in_degree.get_mut(&neighbor) {
                        *deg = deg.saturating_sub(1);
                        if *deg == 0 {
                            queue.push(neighbor);
                        }
                    }
                }
            }
        }

        Ok(result)
    }
}

// ============================================================================
// PubGrub Provider
// ============================================================================

struct PubGrubProvider {
    packages: AHashMap<String, PackageEntry>,
    mode: ResolutionMode,
    min_stability: Stability,
    root_deps: parking_lot::Mutex<DependencyConstraints<PackageName, Ranges<ComposerVersion>>>,
    /// Packages replaced by selected packages (populated between pass 1 and 2).
    /// In pass 1 this is empty; in pass 2 it contains the names of packages
    /// that are replaced by the versions selected in pass 1.
    replaced_packages: AHashSet<String>,
}

impl PubGrubProvider {
    fn new(
        packages: AHashMap<String, PackageEntry>,
        mode: ResolutionMode,
        min_stability: Stability,
        replaced_packages: AHashSet<String>,
    ) -> Self {
        Self {
            packages,
            mode,
            min_stability,
            root_deps: parking_lot::Mutex::new(DependencyConstraints::default()),
            replaced_packages,
        }
    }

    fn set_root_deps(
        &self,
        deps: impl IntoIterator<Item = (PackageName, Ranges<ComposerVersion>)>,
    ) {
        let mut root = self.root_deps.lock();
        root.clear();
        for (n, r) in deps {
            root.insert(n, r);
        }
    }

    fn deps_for(&self, name: &PackageName, version: &ComposerVersion) -> Option<Vec<Dependency>> {
        self.packages
            .get(name.as_str())?
            .versions
            .iter()
            .find(|v| &v.version == version)
            .map(|v| v.dependencies.iter().cloned().collect())
    }

    fn version_info(
        &self,
        name: &PackageName,
        version: &ComposerVersion,
    ) -> Option<&PackageVersion> {
        self.packages
            .get(name.as_str())?
            .versions
            .iter()
            .find(|v| &v.version == version)
    }
}

impl DependencyProvider for PubGrubProvider {
    type P = PackageName;
    type V = ComposerVersion;
    type VS = Ranges<ComposerVersion>;
    type M = String;
    type Err = Infallible;
    type Priority = std::cmp::Reverse<usize>;

    fn prioritize(
        &self,
        pkg: &PackageName,
        range: &Ranges<ComposerVersion>,
        _: &PackageResolutionStatistics,
    ) -> Self::Priority {
        let count = self.packages.get(pkg.as_str()).map_or(0, |e| {
            e.versions
                .iter()
                .filter(|v| range.contains(&v.version))
                .count()
        });
        std::cmp::Reverse(count)
    }

    fn choose_version(
        &self,
        pkg: &PackageName,
        range: &Ranges<ComposerVersion>,
    ) -> Result<Option<ComposerVersion>, Infallible> {
        if pkg.as_str() == "__root__/__root__" {
            let v = ComposerVersion::new(1, 0, 0);
            return Ok(if range.contains(&v) { Some(v) } else { None });
        }

        if is_platform_package(pkg.as_str()) {
            return Ok(None);
        }

        // In pass 2, replaced packages return no versions so PubGrub
        // never selects them — the replacer already provides them.
        if self.replaced_packages.contains(pkg.as_str()) {
            return Ok(None);
        }

        let entry = match self.packages.get(pkg.as_str()) {
            Some(e) => e,
            None => return Ok(None),
        };

        // Filter by range and stability
        let matching: Vec<_> = entry
            .versions
            .iter()
            .filter(|v| range.contains(&v.version) && v.version.stability >= self.min_stability)
            .collect();

        let best = match self.mode {
            ResolutionMode::PreferStable => {
                // Prefer stable versions, then highest
                matching
                    .iter()
                    .filter(|v| v.version.stability == Stability::Stable)
                    .max_by(|a, b| a.version.cmp(&b.version))
                    .copied()
                    .or_else(|| matching.first().copied())
            }
            ResolutionMode::PreferHighest => matching.first().copied(),
            ResolutionMode::PreferLowest => matching.last().copied(),
        };

        Ok(best.map(|v| v.version.clone()))
    }

    fn get_dependencies(
        &self,
        pkg: &PackageName,
        ver: &ComposerVersion,
    ) -> Result<Dependencies<PackageName, Ranges<ComposerVersion>, String>, Infallible> {
        if pkg.as_str() == "__root__/__root__" {
            return Ok(Dependencies::Available(self.root_deps.lock().clone()));
        }

        if is_platform_package(pkg.as_str()) {
            return Ok(Dependencies::Available(DependencyConstraints::default()));
        }

        let entry = match self.packages.get(pkg.as_str()) {
            Some(e) => e,
            None => return Ok(Dependencies::Available(DependencyConstraints::default())),
        };

        let version = match entry.versions.iter().find(|v| &v.version == ver) {
            Some(v) => v,
            None => return Ok(Dependencies::Available(DependencyConstraints::default())),
        };

        let mut deps = DependencyConstraints::default();
        for dep in &version.dependencies {
            let dep_name = dep.name.as_str();
            // In pass 2, skip dependencies on replaced packages — the
            // replacer already satisfies them.
            if !is_platform_package(dep_name) && !self.replaced_packages.contains(dep_name) {
                deps.insert(dep.name.clone(), dep.constraint.ranges().clone());
            }
        }

        Ok(Dependencies::Available(deps))
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Check if a package name is a platform package (php, ext-*, lib-*).
#[inline]
fn is_platform_package(name: &str) -> bool {
    name == "php"
        || name.starts_with("php-")
        || name.starts_with("ext-")
        || name.starts_with("lib-")
        || name == "composer"
        || name == "composer-plugin-api"
        || name == "composer-runtime-api"
}

/// Parse a constraint string from replace/provide declarations.
///
/// Handles the special `"self.version"` value used in Composer's `replace` and
/// `provide` fields, which means "the same version as this package". We treat
/// it as a wildcard match (`*`) while preserving the original string for lock
/// file output.
fn parse_link_constraint(constraint: &str) -> Option<ComposerConstraint> {
    if constraint == "self.version" {
        Some(ComposerConstraint::with_original(
            Ranges::full(),
            Stability::Stable,
            "self.version",
        ))
    } else {
        ComposerConstraint::parse(constraint)
    }
}

/// Convert fetched package data to internal package entry.
fn convert_fetched_package(
    pkg: &FetchedPackage,
    _min_stability: Stability,
) -> Option<PackageEntry> {
    let name = PackageName::parse(&pkg.name)?;
    let mut entry = PackageEntry::new(name.clone());

    for v in &pkg.versions {
        let version = ComposerVersion::parse(&v.version)?;
        let mut pv = PackageVersion::new(name.clone(), version);

        // Dependencies
        for (dep_name, constraint) in &v.require {
            if let (Some(n), Some(c)) = (
                PackageName::parse(dep_name),
                ComposerConstraint::parse(constraint),
            ) {
                pv.add_dependency(Dependency::new(n, c));
            }
        }

        // Replacements
        for (dep_name, constraint) in &v.replace {
            if let (Some(n), Some(c)) = (
                PackageName::parse(dep_name),
                parse_link_constraint(constraint),
            ) {
                pv.add_replace(Dependency::new(n, c));
            }
        }

        // Provides
        for (dep_name, constraint) in &v.provide {
            if let (Some(n), Some(c)) = (
                PackageName::parse(dep_name),
                parse_link_constraint(constraint),
            ) {
                pv.add_provide(Dependency::new(n, c));
            }
        }

        // Distribution info
        pv.dist_url = v.dist_url.as_ref().map(|s| Arc::from(s.as_str()));
        pv.dist_type = v.dist_type.as_ref().map(|s| Arc::from(s.as_str()));
        pv.dist_shasum = v.dist_shasum.as_ref().map(|s| Arc::from(s.as_str()));
        pv.source_url = v.source_url.as_ref().map(|s| Arc::from(s.as_str()));
        pv.source_type = v.source_type.as_ref().map(|s| Arc::from(s.as_str()));
        pv.source_reference = v.source_reference.as_ref().map(|s| Arc::from(s.as_str()));

        // Full metadata
        pv.package_type = v.package_type.as_ref().map(|s| Arc::from(s.as_str()));
        pv.description = v.description.as_ref().map(|s| Arc::from(s.as_str()));
        pv.homepage = v.homepage.as_ref().map(|s| Arc::from(s.as_str()));
        pv.license = v.license.clone();
        pv.authors = v.authors.clone();
        pv.keywords = v.keywords.clone();
        pv.time = v.time.as_ref().map(|s| Arc::from(s.as_str()));
        pv.autoload = v.autoload.clone();
        pv.autoload_dev = v.autoload_dev.clone();
        pv.extra = v.extra.clone();
        pv.support = v.support.clone();
        pv.funding = v.funding.clone();
        pv.notification_url = v.notification_url.as_ref().map(|s| Arc::from(s.as_str()));
        pv.bin = v.bin.clone();

        entry.add_version(pv);
    }

    entry.sort_versions();

    if entry.versions.is_empty() {
        None
    } else {
        Some(entry)
    }
}

/// Build a resolved package from provider data.
fn build_resolved_package(
    pkg_name: PackageName,
    version: ComposerVersion,
    graph: &DiGraph<PackageName, ()>,
    idx: NodeIndex,
    is_dev: bool,
    provider: &PubGrubProvider,
) -> ResolvedPackage {
    let deps: Vec<_> = graph
        .neighbors_directed(idx, Direction::Incoming)
        .filter_map(|n| graph.node_weight(n).cloned())
        .collect();

    let pkg_info = provider.version_info(&pkg_name, &version);

    let (dist_url, dist_type, dist_shasum, src_url, src_type, src_ref) =
        pkg_info.map_or((None, None, None, None, None, None), |v| {
            (
                v.dist_url.as_ref().map(ToString::to_string),
                v.dist_type.as_ref().map(ToString::to_string),
                v.dist_shasum.as_ref().map(ToString::to_string),
                v.source_url.as_ref().map(ToString::to_string),
                v.source_type.as_ref().map(ToString::to_string),
                v.source_reference.as_ref().map(ToString::to_string),
            )
        });

    let (require, require_dev, suggest, provide, replace, conflict) =
        pkg_info.map_or((None, None, None, None, None, None), |v| {
            let to_vec = |deps: &[Dependency]| -> Option<Vec<(String, String)>> {
                if deps.is_empty() {
                    None
                } else {
                    Some(
                        deps.iter()
                            .map(|d| (d.name.as_str().to_string(), d.constraint.to_string()))
                            .collect(),
                    )
                }
            };

            (
                to_vec(&v.dependencies),
                to_vec(&v.dev_dependencies),
                to_vec(&v.suggests),
                to_vec(&v.provides),
                to_vec(&v.replaces),
                to_vec(&v.conflicts),
            )
        });

    let (package_type, description, homepage, license, authors, keywords, time) =
        pkg_info.map_or((None, None, None, None, None, None, None), |v| {
            (
                v.package_type.as_ref().map(ToString::to_string),
                v.description.as_ref().map(ToString::to_string),
                v.homepage.as_ref().map(ToString::to_string),
                v.license.clone(),
                v.authors.clone(),
                v.keywords.clone(),
                v.time.as_ref().map(ToString::to_string),
            )
        });

    let (autoload, autoload_dev, extra, support, funding, notification_url, bin) =
        pkg_info.map_or((None, None, None, None, None, None, None), |v| {
            (
                v.autoload.clone(),
                v.autoload_dev.clone(),
                v.extra.clone(),
                v.support.clone(),
                v.funding.clone(),
                v.notification_url.as_ref().map(ToString::to_string),
                v.bin.clone(),
            )
        });

    ResolvedPackage {
        name: pkg_name,
        version,
        dependencies: deps,
        is_dev,
        dist_url,
        dist_type,
        dist_shasum,
        source_url: src_url,
        source_type: src_type,
        source_reference: src_ref,
        require,
        require_dev,
        suggest,
        provide,
        replace,
        conflict,
        package_type,
        description,
        homepage,
        license,
        authors,
        keywords,
        time,
        autoload,
        autoload_dev,
        extra,
        support,
        funding,
        notification_url,
        bin,
    }
}

// ============================================================================
// Type Aliases for Backward Compatibility
// ============================================================================

/// Backward-compatible alias for `ResolverConfig`.
pub type TurboConfig = ResolverConfig;

/// Backward-compatible alias for Resolver.
pub type TurboResolver<F> = Resolver<F>;

/// Backward-compatible alias for `ResolverStats`.
pub type TurboStats = ResolverStats;
