//! Memory benchmarks.
//!
//! Benchmarks for memory usage tracking and allocator comparison.

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use libretto_bench::{current_rss_bytes, peak_memory_bytes};
use std::time::Duration;

/// Benchmark memory tracking overhead.
fn bench_memory_tracking(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory/tracking");

    group.bench_function("peak_memory", |b| {
        b.iter(|| black_box(peak_memory_bytes()));
    });

    group.bench_function("current_rss", |b| {
        b.iter(|| black_box(current_rss_bytes()));
    });

    group.finish();
}

/// Benchmark allocation patterns.
fn bench_allocation_patterns(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory/allocation");
    group.measurement_time(Duration::from_secs(10));

    // Small allocations
    group.bench_function("small_vec_1kb", |b| {
        b.iter(|| {
            let v: Vec<u8> = vec![0; 1024];
            black_box(v)
        });
    });

    // Medium allocations
    group.bench_function("medium_vec_1mb", |b| {
        b.iter(|| {
            let v: Vec<u8> = vec![0; 1024 * 1024];
            black_box(v)
        });
    });

    // Large allocations
    group.bench_function("large_vec_16mb", |b| {
        b.iter(|| {
            let v: Vec<u8> = vec![0; 16 * 1024 * 1024];
            black_box(v)
        });
    });

    group.finish();
}

/// Benchmark string allocation patterns (common in package management).
fn bench_string_allocations(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory/strings");

    // Package name like allocations
    group.bench_function("package_names_100", |b| {
        b.iter(|| {
            let names: Vec<String> = (0..100)
                .map(|i| format!("vendor{}/package{}", i / 10, i))
                .collect();
            black_box(names)
        });
    });

    // Version constraint allocations
    group.bench_function("constraints_100", |b| {
        b.iter(|| {
            let constraints: Vec<String> = (0..100)
                .map(|i| format!("^{}.{}.{}", i / 100, (i / 10) % 10, i % 10))
                .collect();
            black_box(constraints)
        });
    });

    group.finish();
}

/// Benchmark `HashMap` allocation (common for dependency maps).
fn bench_hashmap_allocations(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory/hashmap");

    for size in [100, 1000, 10000] {
        group.bench_with_input(BenchmarkId::new("std_hashmap", size), &size, |b, &n| {
            b.iter(|| {
                let mut map = std::collections::HashMap::with_capacity(n);
                for i in 0..n {
                    map.insert(format!("key{i}"), format!("value{i}"));
                }
                black_box(map)
            });
        });

        group.bench_with_input(BenchmarkId::new("ahash_hashmap", size), &size, |b, &n| {
            b.iter(|| {
                let mut map = ahash::AHashMap::with_capacity(n);
                for i in 0..n {
                    map.insert(format!("key{i}"), format!("value{i}"));
                }
                black_box(map)
            });
        });
    }

    group.finish();
}

/// Benchmark `BTreeMap` vs `HashMap` for sorted output (lockfiles need determinism).
fn bench_btreemap_vs_hashmap(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory/map_comparison");

    for size in [100, 1000] {
        group.bench_with_input(BenchmarkId::new("btreemap_insert", size), &size, |b, &n| {
            b.iter(|| {
                let mut map = std::collections::BTreeMap::new();
                for i in 0..n {
                    map.insert(format!("key{i}"), format!("value{i}"));
                }
                black_box(map)
            });
        });

        group.bench_with_input(
            BenchmarkId::new("hashmap_insert_and_sort", size),
            &size,
            |b, &n| {
                b.iter(|| {
                    let mut map = std::collections::HashMap::new();
                    for i in 0..n {
                        map.insert(format!("key{i}"), format!("value{i}"));
                    }
                    let mut pairs: Vec<_> = map.into_iter().collect();
                    pairs.sort_by(|a, b| a.0.cmp(&b.0));
                    black_box(pairs)
                });
            },
        );
    }

    group.finish();
}

/// Benchmark memory during JSON parsing.
fn bench_json_memory(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory/json");
    group.sample_size(20);

    let json_content = libretto_bench::fixtures::generate_composer_lock(500);

    group.bench_function("parse_large_json", |b| {
        b.iter(|| {
            let parsed: serde_json::Value = serde_json::from_str(&json_content).unwrap();
            black_box(parsed)
        });
    });

    group.bench_function("parse_and_stringify", |b| {
        b.iter(|| {
            let parsed: serde_json::Value = serde_json::from_str(&json_content).unwrap();
            let output = serde_json::to_string(&parsed).unwrap();
            black_box(output)
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_memory_tracking,
    bench_allocation_patterns,
    bench_string_allocations,
    bench_hashmap_allocations,
    bench_btreemap_vs_hashmap,
    bench_json_memory,
);

criterion_main!(benches);
