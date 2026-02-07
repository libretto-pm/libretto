//! Real-world scenario benchmarks.
//!
//! Comprehensive benchmarks simulating real PHP project installations like Laravel, Symfony.

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use libretto_bench::fixtures::{
    generate_composer_json, generate_composer_lock, laravel_like_dependencies,
    symfony_like_dependencies,
};
use libretto_bench::generators::PhpProjectGenerator;
use std::collections::HashMap;
use std::time::Duration;

/// Benchmark simulated Laravel project resolution.
fn bench_laravel_simulation(c: &mut Criterion) {
    let mut group = c.benchmark_group("realworld/laravel");
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(15));

    let deps = laravel_like_dependencies();

    group.throughput(Throughput::Elements(deps.len() as u64));

    group.bench_function("parse_deps", |b| {
        b.iter(|| {
            // Parse each dependency constraint
            let parsed: Vec<_> = deps
                .iter()
                .map(|(name, constraint)| {
                    (name.to_string(), semver::VersionReq::parse(constraint).ok())
                })
                .collect();
            black_box(parsed)
        });
    });

    group.bench_function("build_dep_tree", |b| {
        b.iter(|| {
            // Build a dependency map
            let mut dep_map: HashMap<String, Vec<String>> = HashMap::new();
            for (name, _constraint) in &deps {
                // Simulate adding transitive dependencies
                let parts: Vec<_> = name.split('/').collect();
                if parts.len() == 2 {
                    dep_map
                        .entry(parts[0].to_string())
                        .or_default()
                        .push(parts[1].to_string());
                }
            }
            black_box(dep_map)
        });
    });

    group.finish();
}

/// Benchmark simulated Symfony project resolution.
fn bench_symfony_simulation(c: &mut Criterion) {
    let mut group = c.benchmark_group("realworld/symfony");
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(15));

    let deps = symfony_like_dependencies();

    group.throughput(Throughput::Elements(deps.len() as u64));

    group.bench_function("parse_deps", |b| {
        b.iter(|| {
            let parsed: Vec<_> = deps
                .iter()
                .map(|(name, constraint)| {
                    (name.to_string(), semver::VersionReq::parse(constraint).ok())
                })
                .collect();
            black_box(parsed)
        });
    });

    group.bench_function("version_resolution", |b| {
        b.iter(|| {
            // Simulate version resolution
            let versions: Vec<semver::Version> =
                (0..100).map(|i| semver::Version::new(6, i, 0)).collect();

            let req = semver::VersionReq::parse("^6.0").unwrap();
            let matching_count = versions.iter().filter(|v| req.matches(v)).count();

            black_box(matching_count)
        });
    });

    group.finish();
}

/// Benchmark full project setup with varying sizes.
fn bench_project_setup(c: &mut Criterion) {
    let mut group = c.benchmark_group("realworld/setup");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(20));

    for (deps, files, label) in [
        (10, 100, "small"),
        (50, 500, "medium"),
        (100, 1000, "large"),
    ] {
        group.bench_with_input(
            BenchmarkId::new("project", label),
            &(deps, files),
            |b, &(num_deps, num_files)| {
                b.iter_with_setup(
                    || PhpProjectGenerator::new().unwrap(),
                    |project_gen| {
                        project_gen.generate_composer_json(num_deps).unwrap();
                        project_gen.generate_composer_lock(num_deps).unwrap();
                        project_gen.generate_files(num_files).unwrap();
                        black_box(project_gen.root().to_path_buf())
                    },
                );
            },
        );
    }

    group.finish();
}

/// Benchmark update with no changes (should be fast).
fn bench_update_no_changes(c: &mut Criterion) {
    let mut group = c.benchmark_group("realworld/update/nochange");

    // Generate identical lock files
    let lock_content = generate_composer_lock(100);

    group.bench_function("compare_identical", |b| {
        b.iter(|| {
            // Parse both (simulating reading from disk)
            let lock1: serde_json::Value = sonic_rs::from_str(&lock_content).unwrap();
            let lock2: serde_json::Value = sonic_rs::from_str(&lock_content).unwrap();

            // Compare content-hash
            let hash1 = lock1["content-hash"].as_str();
            let hash2 = lock2["content-hash"].as_str();

            let same = hash1 == hash2;
            black_box(same)
        });
    });

    group.bench_function("full_comparison", |b| {
        b.iter(|| {
            let lock1: serde_json::Value = sonic_rs::from_str(&lock_content).unwrap();
            let lock2: serde_json::Value = sonic_rs::from_str(&lock_content).unwrap();

            // Full JSON comparison
            let same = lock1 == lock2;
            black_box(same)
        });
    });

    group.finish();
}

/// Benchmark update with changes.
fn bench_update_with_changes(c: &mut Criterion) {
    let mut group = c.benchmark_group("realworld/update/changes");

    for num_changes in [5, 10, 20, 50] {
        let lock1 = generate_composer_lock(100);
        let lock2 = generate_composer_lock(100 + num_changes);

        group.bench_with_input(
            BenchmarkId::new("changed_packages", num_changes),
            &(lock1.clone(), lock2.clone()),
            |b, (l1, l2)| {
                b.iter(|| {
                    let parsed1: serde_json::Value = sonic_rs::from_str(l1).unwrap();
                    let parsed2: serde_json::Value = sonic_rs::from_str(l2).unwrap();

                    let packages1 = parsed1["packages"].as_array().unwrap();
                    let packages2 = parsed2["packages"].as_array().unwrap();

                    // Build name -> version maps with owned Strings
                    let map1: HashMap<String, String> = packages1
                        .iter()
                        .filter_map(|p| {
                            let name = p["name"].as_str()?.to_string();
                            let version = p["version"].as_str()?.to_string();
                            Some((name, version))
                        })
                        .collect();

                    let map2: HashMap<String, String> = packages2
                        .iter()
                        .filter_map(|p| {
                            let name = p["name"].as_str()?.to_string();
                            let version = p["version"].as_str()?.to_string();
                            Some((name, version))
                        })
                        .collect();

                    // Find added, removed, updated
                    let added = map2.keys().filter(|k| !map1.contains_key(*k)).count();
                    let removed = map1.keys().filter(|k| !map2.contains_key(*k)).count();
                    let updated = map1
                        .iter()
                        .filter(|(k, v)| map2.get(*k).is_some_and(|v2| *v != v2))
                        .count();

                    black_box((added, removed, updated))
                });
            },
        );
    }

    group.finish();
}

/// Benchmark cold vs warm cache scenarios.
fn bench_cache_scenarios(c: &mut Criterion) {
    let mut group = c.benchmark_group("realworld/cache_scenario");
    group.sample_size(20);

    // Cold cache: nothing cached
    group.bench_function("cold_100_lookups", |b| {
        b.iter_with_setup(
            || HashMap::<String, Vec<u8>>::new(),
            |cache| {
                let mut hits = 0;
                for i in 0..100 {
                    let key = format!("vendor/package{i}");
                    if cache.contains_key(&key) {
                        hits += 1;
                    }
                }
                black_box(hits)
            },
        );
    });

    // Warm cache: everything cached
    let mut warm_cache = HashMap::new();
    for i in 0..100 {
        let key = format!("vendor/package{i}");
        warm_cache.insert(key, vec![0u8; 4096]);
    }

    group.bench_function("warm_100_lookups", |b| {
        b.iter(|| {
            let mut hits = 0;
            for i in 0..100 {
                let key = format!("vendor/package{i}");
                if warm_cache.contains_key(&key) {
                    hits += 1;
                }
            }
            black_box(hits)
        });
    });

    // Partial cache: 50% hit rate
    let mut partial_cache = HashMap::new();
    for i in 0..50 {
        let key = format!("vendor/package{i}");
        partial_cache.insert(key, vec![0u8; 4096]);
    }

    group.bench_function("partial_100_lookups", |b| {
        b.iter(|| {
            let mut hits = 0;
            for i in 0..100 {
                let key = format!("vendor/package{i}");
                if partial_cache.contains_key(&key) {
                    hits += 1;
                }
            }
            black_box(hits)
        });
    });

    group.finish();
}

/// Benchmark composer.json parsing and validation.
fn bench_composer_json_parsing(c: &mut Criterion) {
    let mut group = c.benchmark_group("realworld/composer_json");

    for num_deps in [10, 50, 100, 500] {
        let content = generate_composer_json(num_deps);

        group.throughput(Throughput::Bytes(content.len() as u64));
        group.bench_with_input(BenchmarkId::new("parse", num_deps), &content, |b, json| {
            b.iter(|| {
                let parsed: serde_json::Value = sonic_rs::from_str(black_box(json)).unwrap();

                // Extract key fields
                let name = parsed["name"].as_str().map(|s| s.to_string());
                let require_count = parsed["require"].as_object().map(|r| r.len());
                let require_dev_count = parsed["require-dev"].as_object().map(|r| r.len());

                black_box((name, require_count, require_dev_count))
            });
        });
    }

    group.finish();
}

/// Benchmark install command simulation.
fn bench_install_simulation(c: &mut Criterion) {
    let mut group = c.benchmark_group("realworld/install");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(20));

    for num_packages in [10, 50, 100] {
        group.bench_with_input(
            BenchmarkId::new("packages", num_packages),
            &num_packages,
            |b, &n| {
                b.iter_with_setup(
                    || {
                        let project = PhpProjectGenerator::new().unwrap();
                        project.generate_composer_json(n).unwrap();
                        project.generate_composer_lock(n).unwrap();
                        project
                    },
                    |project| {
                        // Simulate install steps:
                        // 1. Read composer.json
                        let json_path = project.root().join("composer.json");
                        let json_content = std::fs::read_to_string(&json_path).unwrap();
                        let _parsed: serde_json::Value = sonic_rs::from_str(&json_content).unwrap();

                        // 2. Read composer.lock
                        let lock_path = project.root().join("composer.lock");
                        let lock_content = std::fs::read_to_string(&lock_path).unwrap();
                        let lock: serde_json::Value = sonic_rs::from_str(&lock_content).unwrap();

                        // 3. Process packages
                        let packages = lock["packages"].as_array().unwrap();
                        black_box(
                            packages
                                .iter()
                                .map(|p| {
                                    let name = p["name"].as_str().unwrap();
                                    let version = p["version"].as_str().unwrap();
                                    (name, version)
                                })
                                .count(),
                        )
                    },
                );
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_laravel_simulation,
    bench_symfony_simulation,
    bench_project_setup,
    bench_update_no_changes,
    bench_update_with_changes,
    bench_cache_scenarios,
    bench_composer_json_parsing,
    bench_install_simulation,
);

criterion_main!(benches);
