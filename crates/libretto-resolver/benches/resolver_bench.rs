//! Benchmarks for the dependency resolver.

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use libretto_resolver::{
    ComposerConstraint, ComposerVersion, Dependency, FetchedPackage, FetchedVersion, MemorySource,
    PackageFetcher, PackageIndex, PackageName, ResolutionMode, Resolver, ResolverConfig, Stability,
};
use rand::prelude::*;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

/// A mock async fetcher that wraps a synchronous `MemorySource` for benchmarking.
struct MockFetcher {
    index: Arc<PackageIndex<MemorySource>>,
}

impl MockFetcher {
    fn new(source: MemorySource) -> Self {
        Self {
            index: Arc::new(PackageIndex::new(source)),
        }
    }
}

impl PackageFetcher for MockFetcher {
    fn fetch(
        &self,
        name: String,
    ) -> Pin<Box<dyn std::future::Future<Output = Option<FetchedPackage>> + Send + '_>> {
        let index = Arc::clone(&self.index);
        Box::pin(async move {
            let pkg_name = PackageName::parse(&name)?;
            let entry = index.get(&pkg_name)?;

            let versions: Vec<FetchedVersion> = entry
                .versions
                .iter()
                .map(|v| FetchedVersion {
                    version: v.version.to_string(),
                    require: v
                        .dependencies
                        .iter()
                        .map(|d| (d.name.as_str().to_string(), d.constraint.to_string()))
                        .collect(),
                    require_dev: v
                        .dev_dependencies
                        .iter()
                        .map(|d| (d.name.as_str().to_string(), d.constraint.to_string()))
                        .collect(),
                    replace: v
                        .replaces
                        .iter()
                        .map(|d| (d.name.as_str().to_string(), d.constraint.to_string()))
                        .collect(),
                    provide: v
                        .provides
                        .iter()
                        .map(|d| (d.name.as_str().to_string(), d.constraint.to_string()))
                        .collect(),
                    suggest: v
                        .suggests
                        .iter()
                        .map(|d| (d.name.as_str().to_string(), d.constraint.to_string()))
                        .collect(),
                    dist_url: v.dist_url.as_ref().map(ToString::to_string),
                    dist_type: v.dist_type.as_ref().map(ToString::to_string),
                    dist_shasum: v.dist_shasum.as_ref().map(ToString::to_string),
                    source_url: v.source_url.as_ref().map(ToString::to_string),
                    source_type: v.source_type.as_ref().map(ToString::to_string),
                    source_reference: v.source_reference.as_ref().map(ToString::to_string),
                    package_type: v.package_type.as_ref().map(ToString::to_string),
                    description: v.description.as_ref().map(ToString::to_string),
                    homepage: v.homepage.as_ref().map(ToString::to_string),
                    license: v.license.clone(),
                    authors: v.authors.clone(),
                    keywords: v.keywords.clone(),
                    time: v.time.as_ref().map(ToString::to_string),
                    autoload: v.autoload.clone(),
                    autoload_dev: v.autoload_dev.clone(),
                    extra: v.extra.clone(),
                    support: v.support.clone(),
                    funding: v.funding.clone(),
                    notification_url: v.notification_url.as_ref().map(ToString::to_string),
                    bin: v.bin.clone(),
                })
                .collect();

            Some(FetchedPackage {
                name: entry.name.as_str().to_string(),
                versions,
            })
        })
    }
}

/// Generate a synthetic package registry with the specified number of packages.
fn generate_registry(
    num_packages: usize,
    versions_per_package: usize,
    deps_per_version: usize,
) -> MemorySource {
    let source = MemorySource::new();
    let mut rng = rand::thread_rng();

    let package_names: Vec<String> = (0..num_packages)
        .map(|i| format!("vendor{}/package{}", i / 100, i % 100))
        .collect();

    for (pkg_idx, name) in package_names.iter().enumerate() {
        for v in 0..versions_per_package {
            let version = format!("{}.{}.0", v / 10 + 1, v % 10);

            // Generate random dependencies
            let mut deps = Vec::new();
            for _ in 0..deps_per_version {
                // Pick a random package (but not self)
                let dep_idx = rng.gen_range(0..num_packages);
                if dep_idx != pkg_idx {
                    let dep_name = &package_names[dep_idx];
                    let constraint =
                        format!("^{}.0", rng.gen_range(1..=versions_per_package / 10 + 1));
                    deps.push((dep_name.as_str(), constraint));
                }
            }

            let deps_refs: Vec<(&str, &str)> = deps.iter().map(|(n, c)| (*n, c.as_str())).collect();
            source.add_version(name, &version, deps_refs);
        }
    }

    source
}

/// Benchmark version parsing.
fn bench_version_parsing(c: &mut Criterion) {
    let versions = vec![
        "1.0.0",
        "1.2.3",
        "v2.0.0",
        "1.0.0-alpha",
        "1.0.0-beta.1",
        "1.0.0-RC1",
        "dev-master",
        "1.0.x-dev",
        "2.3.4.5",
    ];

    c.bench_function("version_parse", |b| {
        b.iter(|| {
            for v in &versions {
                black_box(ComposerVersion::parse(v));
            }
        });
    });
}

/// Benchmark constraint parsing.
fn bench_constraint_parsing(c: &mut Criterion) {
    let constraints = vec![
        "^1.0",
        "~1.2.3",
        ">=1.0 <2.0",
        "1.0.*",
        "^1.0 || ^2.0",
        ">=1.0.0 <1.1.0 || >=1.2.0 <2.0.0",
        "1.0.0 - 2.0.0",
        ">=1.0@dev",
        "dev-master",
    ];

    c.bench_function("constraint_parse", |b| {
        b.iter(|| {
            for c in &constraints {
                black_box(ComposerConstraint::parse(c));
            }
        });
    });
}

/// Benchmark constraint matching.
fn bench_constraint_matching(c: &mut Criterion) {
    let constraint = ComposerConstraint::parse("^1.0").unwrap();
    let versions: Vec<_> = (0..100)
        .map(|i| ComposerVersion::parse(&format!("{}.{}.0", i / 10, i % 10)).unwrap())
        .collect();

    c.bench_function("constraint_match_100", |b| {
        b.iter(|| {
            for v in &versions {
                black_box(constraint.matches(v));
            }
        });
    });
}

/// Benchmark package index lookup.
fn bench_index_lookup(c: &mut Criterion) {
    let source = generate_registry(100, 10, 3);
    let index = PackageIndex::new(source);

    let names: Vec<_> = (0..100)
        .map(|i| PackageName::parse(&format!("vendor{}/package{}", i / 100, i % 100)).unwrap())
        .collect();

    c.bench_function("index_lookup_100", |b| {
        b.iter(|| {
            for name in &names {
                black_box(index.get(name));
            }
        });
    });
}

/// Create a tokio runtime for async benchmarks.
fn create_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

/// Benchmark resolution with different graph sizes.
fn bench_resolution(c: &mut Criterion) {
    let mut group = c.benchmark_group("resolution");

    for size in [10, 50, 100] {
        let source = generate_registry(size, 5, 2);
        let fetcher = Arc::new(MockFetcher::new(source));
        let config = ResolverConfig {
            max_concurrent: 32,
            request_timeout: Duration::from_secs(10),
            mode: ResolutionMode::PreferStable,
            min_stability: Stability::Stable,
            include_dev: false,
        };
        let resolver = Resolver::new(Arc::clone(&fetcher), config);

        // Create root dependencies (pick a few packages)
        let deps: Vec<_> = (0..3)
            .map(|i| {
                Dependency::new(
                    PackageName::parse(&format!("vendor{}/package{}", i / 100, i % 100)).unwrap(),
                    ComposerConstraint::parse(">=1.0").unwrap(),
                )
            })
            .collect();

        let rt = create_runtime();

        group.throughput(Throughput::Elements(size as u64));
        group.bench_with_input(BenchmarkId::new("packages", size), &size, |b, _| {
            b.iter(|| rt.block_on(async { black_box(resolver.resolve(&deps, &[]).await) }));
        });
    }

    group.finish();
}

/// Benchmark warm cache resolution.
fn bench_warm_resolution(c: &mut Criterion) {
    let source = generate_registry(100, 5, 2);
    let fetcher = Arc::new(MockFetcher::new(source));
    let config = ResolverConfig {
        max_concurrent: 32,
        request_timeout: Duration::from_secs(10),
        mode: ResolutionMode::PreferStable,
        min_stability: Stability::Stable,
        include_dev: false,
    };
    let resolver = Resolver::new(Arc::clone(&fetcher), config);

    let deps: Vec<_> = (0..3)
        .map(|i| {
            Dependency::new(
                PackageName::parse(&format!("vendor{}/package{}", i / 100, i % 100)).unwrap(),
                ComposerConstraint::parse(">=1.0").unwrap(),
            )
        })
        .collect();

    let rt = create_runtime();

    // Warm up the cache
    let _ = rt.block_on(resolver.resolve(&deps, &[]));

    c.bench_function("resolution_warm_100", |b| {
        b.iter(|| rt.block_on(async { black_box(resolver.resolve(&deps, &[]).await) }));
    });
}

/// Benchmark prefer-lowest mode.
fn bench_prefer_lowest(c: &mut Criterion) {
    let source = generate_registry(100, 10, 2);
    let fetcher = Arc::new(MockFetcher::new(source));
    let config = ResolverConfig {
        max_concurrent: 32,
        request_timeout: Duration::from_secs(10),
        mode: ResolutionMode::PreferLowest,
        min_stability: Stability::Stable,
        include_dev: false,
    };
    let resolver = Resolver::new(fetcher, config);

    let deps: Vec<_> = (0..3)
        .map(|i| {
            Dependency::new(
                PackageName::parse(&format!("vendor{}/package{}", i / 100, i % 100)).unwrap(),
                ComposerConstraint::parse(">=1.0").unwrap(),
            )
        })
        .collect();

    let rt = create_runtime();

    c.bench_function("resolution_prefer_lowest", |b| {
        b.iter(|| rt.block_on(async { black_box(resolver.resolve(&deps, &[]).await) }));
    });
}

criterion_group!(
    benches,
    bench_version_parsing,
    bench_constraint_parsing,
    bench_constraint_matching,
    bench_index_lookup,
    bench_resolution,
    bench_warm_resolution,
    bench_prefer_lowest,
);

criterion_main!(benches);
