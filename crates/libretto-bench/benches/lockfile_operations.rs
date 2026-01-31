//! Lock file operations benchmarks.
//!
//! Benchmarks for parsing, generating, and diffing composer.lock files.

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use libretto_bench::fixtures::{generate_composer_json, generate_composer_lock};
use sonic_rs::{JsonContainerTrait, JsonValueTrait};
use std::collections::HashSet;

/// Benchmark composer.lock parsing with sonic-rs (using sonic_rs::Value for full SIMD benefits).
fn bench_lockfile_parse_sonic(c: &mut Criterion) {
    let mut group = c.benchmark_group("lockfile/parse/sonic_rs");

    for num_packages in [10, 100, 500, 1000] {
        let lock_content = generate_composer_lock(num_packages);
        let bytes = lock_content.as_bytes();

        group.throughput(Throughput::Bytes(bytes.len() as u64));
        group.bench_with_input(
            BenchmarkId::new("packages", num_packages),
            &lock_content,
            |b, content| {
                b.iter(|| {
                    // Use sonic_rs::Value for full SIMD acceleration
                    let parsed: sonic_rs::Value = sonic_rs::from_str(black_box(content)).unwrap();
                    black_box(parsed)
                });
            },
        );
    }

    group.finish();
}

/// Benchmark composer.lock parsing with serde_json (for comparison).
fn bench_lockfile_parse_serde(c: &mut Criterion) {
    let mut group = c.benchmark_group("lockfile/parse/serde_json");

    for num_packages in [10, 100, 500, 1000] {
        let lock_content = generate_composer_lock(num_packages);
        let bytes = lock_content.as_bytes();

        group.throughput(Throughput::Bytes(bytes.len() as u64));
        group.bench_with_input(
            BenchmarkId::new("packages", num_packages),
            &lock_content,
            |b, content| {
                b.iter(|| {
                    let parsed: serde_json::Value =
                        serde_json::from_str(black_box(content)).unwrap();
                    black_box(parsed)
                });
            },
        );
    }

    group.finish();
}

/// Benchmark content-hash computation.
fn bench_content_hash(c: &mut Criterion) {
    let mut group = c.benchmark_group("lockfile/content_hash");

    for num_deps in [10, 50, 100, 500] {
        let json_content = generate_composer_json(num_deps);
        let bytes = json_content.as_bytes();

        group.throughput(Throughput::Bytes(bytes.len() as u64));
        group.bench_with_input(
            BenchmarkId::new("deps", num_deps),
            &json_content,
            |b, content| {
                b.iter(|| {
                    let hash = blake3::hash(black_box(content.as_bytes()));
                    black_box(hash)
                });
            },
        );
    }

    group.finish();
}

/// Benchmark lock file serialization with sonic_rs (using sonic_rs::Value for consistency).
fn bench_lockfile_serialize(c: &mut Criterion) {
    let mut group = c.benchmark_group("lockfile/serialize");

    for num_packages in [10, 100, 500, 1000] {
        let lock_content = generate_composer_lock(num_packages);
        // Parse with sonic_rs::Value for full SIMD benefits in serialization
        let parsed: sonic_rs::Value = sonic_rs::from_str(&lock_content).unwrap();

        group.throughput(Throughput::Elements(num_packages as u64));
        group.bench_with_input(
            BenchmarkId::new("packages", num_packages),
            &parsed,
            |b, value| {
                b.iter(|| {
                    let output = sonic_rs::to_string(black_box(value)).unwrap();
                    black_box(output)
                });
            },
        );
    }

    group.finish();
}

/// Benchmark lock file diff computation using sonic_rs for SIMD-accelerated access.
fn bench_lockfile_diff(c: &mut Criterion) {
    let mut group = c.benchmark_group("lockfile/diff");
    group.sample_size(50);

    for num_packages in [100, 500] {
        let lock1 = generate_composer_lock(num_packages);
        let lock2 = generate_composer_lock(num_packages + 10);

        // Use sonic_rs::Value for faster JSON access
        let parsed1: sonic_rs::Value = sonic_rs::from_str(&lock1).unwrap();
        let parsed2: sonic_rs::Value = sonic_rs::from_str(&lock2).unwrap();

        group.bench_with_input(
            BenchmarkId::new("packages", num_packages),
            &(parsed1.clone(), parsed2.clone()),
            |b, (v1, v2)| {
                b.iter(|| {
                    let packages1 = v1
                        .get("packages")
                        .and_then(|p: &sonic_rs::Value| p.as_array())
                        .unwrap();
                    let packages2 = v2
                        .get("packages")
                        .and_then(|p: &sonic_rs::Value| p.as_array())
                        .unwrap();

                    let names1: HashSet<String> = packages1
                        .iter()
                        .filter_map(|p: &sonic_rs::Value| {
                            p.get("name")
                                .and_then(|n: &sonic_rs::Value| n.as_str())
                                .map(String::from)
                        })
                        .collect();
                    let names2: HashSet<String> = packages2
                        .iter()
                        .filter_map(|p: &sonic_rs::Value| {
                            p.get("name")
                                .and_then(|n: &sonic_rs::Value| n.as_str())
                                .map(String::from)
                        })
                        .collect();

                    let added: Vec<_> = names2.difference(&names1).cloned().collect();
                    let removed: Vec<_> = names1.difference(&names2).cloned().collect();

                    black_box((added, removed))
                });
            },
        );
    }

    group.finish();
}

/// Benchmark deterministic JSON output using sonic_rs for SIMD-accelerated serialization.
fn bench_deterministic_output(c: &mut Criterion) {
    let mut group = c.benchmark_group("lockfile/deterministic");

    for num_packages in [100, 500] {
        let lock_content = generate_composer_lock(num_packages);
        // Use sonic_rs::Value for consistent SIMD benefits
        let parsed: sonic_rs::Value = sonic_rs::from_str(&lock_content).unwrap();

        group.bench_with_input(
            BenchmarkId::new("packages", num_packages),
            &parsed,
            |b, value| {
                b.iter(|| {
                    // Use sonic_rs::to_string_pretty for SIMD-accelerated pretty printing
                    let output = sonic_rs::to_string_pretty(black_box(value)).unwrap();
                    black_box(output)
                });
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_lockfile_parse_sonic,
    bench_lockfile_parse_serde,
    bench_content_hash,
    bench_lockfile_serialize,
    bench_lockfile_diff,
    bench_deterministic_output,
);

criterion_main!(benches);
