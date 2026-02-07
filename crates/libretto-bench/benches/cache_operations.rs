//! Cache operations benchmarks.
//!
//! Benchmarks for cache and content-addressable storage operations.

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use std::collections::HashMap;
use std::time::Duration;

/// Benchmark in-memory cache operations.
fn bench_memory_cache(c: &mut Criterion) {
    let mut group = c.benchmark_group("cache/memory");

    let data = vec![0u8; 4096];

    // Pre-populate cache
    let mut cache: HashMap<String, Vec<u8>> = HashMap::new();
    for i in 0..1000 {
        cache.insert(format!("key-{i}"), data.clone());
    }

    group.throughput(Throughput::Elements(1));

    group.bench_function("get_hit", |b| {
        b.iter(|| black_box(cache.get("key-500")));
    });

    group.bench_function("get_miss", |b| {
        b.iter(|| black_box(cache.get("nonexistent")));
    });

    group.bench_function("insert", |b| {
        let mut i = 10000usize;
        b.iter(|| {
            cache.insert(format!("bench-{i}"), data.clone());
            i += 1;
        });
    });

    group.bench_function("contains", |b| {
        b.iter(|| black_box(cache.contains_key("key-500")));
    });

    group.finish();
}

/// Benchmark content-addressable storage hashing.
fn bench_content_hash(c: &mut Criterion) {
    let mut group = c.benchmark_group("cache/hash");

    let sizes = [(1024, "1KB"), (64 * 1024, "64KB"), (1024 * 1024, "1MB")];

    for (size, label) in sizes {
        let data = vec![0xABu8; size];

        group.throughput(Throughput::Bytes(size as u64));

        group.bench_with_input(BenchmarkId::new("blake3", label), &data, |b, d| {
            b.iter(|| {
                let hash = blake3::hash(black_box(d));
                black_box(hash)
            });
        });
    }

    group.finish();
}

/// Benchmark cache lookup performance across sizes.
fn bench_cache_lookup(c: &mut Criterion) {
    let mut group = c.benchmark_group("cache/lookup");

    for cache_size in [100, 1000, 10000] {
        let mut cache = HashMap::with_capacity(cache_size);
        let data = vec![0u8; 1024];

        // Populate cache
        for i in 0..cache_size {
            cache.insert(format!("key-{i}"), data.clone());
        }

        group.bench_with_input(
            BenchmarkId::new("entries", cache_size),
            &cache_size,
            |b, &n| {
                let mut idx = 0usize;
                b.iter(|| {
                    let key = format!("key-{}", idx % n);
                    idx += 1;
                    black_box(cache.get(&key))
                });
            },
        );
    }

    group.finish();
}

/// Benchmark ahash vs std hash.
fn bench_hash_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("cache/hash_algo");

    for size in [100, 1000] {
        // Standard HashMap
        group.bench_with_input(BenchmarkId::new("std_hashmap", size), &size, |b, &n| {
            b.iter(|| {
                let mut map = std::collections::HashMap::new();
                for i in 0..n {
                    map.insert(format!("key{i}"), vec![0u8; 64]);
                }
                black_box(map)
            });
        });

        // AHash HashMap
        group.bench_with_input(BenchmarkId::new("ahash_hashmap", size), &size, |b, &n| {
            b.iter(|| {
                let mut map = ahash::AHashMap::new();
                for i in 0..n {
                    map.insert(format!("key{i}"), vec![0u8; 64]);
                }
                black_box(map)
            });
        });
    }

    group.finish();
}

/// Benchmark file cache operations simulation.
fn bench_file_cache_simulation(c: &mut Criterion) {
    let mut group = c.benchmark_group("cache/file_sim");
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(10));

    for num_entries in [10, 50, 100] {
        group.bench_with_input(
            BenchmarkId::new("entries", num_entries),
            &num_entries,
            |b, &n| {
                b.iter_with_setup(
                    || {
                        let dir = tempfile::tempdir().unwrap();
                        let data: Vec<Vec<u8>> = (0..n)
                            .map(|i| {
                                let mut d = vec![0u8; 4096];
                                d[0] = i as u8;
                                d
                            })
                            .collect();
                        (dir, data)
                    },
                    |(dir, data)| {
                        // Write files (simulating cache put)
                        for d in &data {
                            let hash = blake3::hash(d);
                            let path = dir.path().join(hash.to_hex().to_string());
                            std::fs::write(&path, d).unwrap();
                        }
                        black_box(dir)
                    },
                );
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_memory_cache,
    bench_content_hash,
    bench_cache_lookup,
    bench_hash_comparison,
    bench_file_cache_simulation,
);

criterion_main!(benches);
