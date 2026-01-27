//! Benchmarks for the cache system.

use bytes::Bytes;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use libretto_cache::{BloomFilter, CacheConfig, CacheEntryType, L1Cache, L1Entry, TieredCache};
use libretto_core::ContentHash;

fn bench_bloom_filter(c: &mut Criterion) {
    let mut group = c.benchmark_group("bloom_filter");

    for size in [1_000, 10_000, 100_000] {
        let mut bf = BloomFilter::new(size, 0.01);

        // Pre-populate
        for i in 0..size {
            bf.insert(&i);
        }

        group.bench_with_input(BenchmarkId::new("lookup_hit", size), &size, |b, _| {
            b.iter(|| {
                for i in 0..100 {
                    black_box(bf.may_contain(&i));
                }
            });
        });

        group.bench_with_input(BenchmarkId::new("lookup_miss", size), &size, |b, &size| {
            b.iter(|| {
                for i in size..(size + 100) {
                    black_box(bf.may_contain(&i));
                }
            });
        });

        group.bench_with_input(BenchmarkId::new("insert", size), &size, |b, _| {
            let mut bf = BloomFilter::new(size, 0.01);
            let mut i = 0usize;
            b.iter(|| {
                bf.insert(&i);
                i += 1;
            });
        });
    }

    group.finish();
}

fn bench_l1_cache(c: &mut Criterion) {
    let mut group = c.benchmark_group("l1_cache");

    let cache = L1Cache::new(256 * 1024 * 1024, None);
    let data = vec![0u8; 1024];

    // Pre-populate with some entries
    for i in 0..1000 {
        let entry = L1Entry::new(Bytes::from(data.clone()), 1024, false, [i as u8; 32]);
        cache.insert(format!("key-{i}"), entry);
    }

    group.throughput(Throughput::Elements(1));

    group.bench_function("get_hit", |b| {
        b.iter(|| {
            black_box(cache.get("key-500"));
        });
    });

    group.bench_function("get_miss", |b| {
        b.iter(|| {
            black_box(cache.get("nonexistent-key"));
        });
    });

    group.bench_function("insert", |b| {
        let mut i = 2000usize;
        b.iter(|| {
            let entry = L1Entry::new(Bytes::from(data.clone()), 1024, false, [0u8; 32]);
            cache.insert(format!("bench-key-{i}"), entry);
            i += 1;
        });
    });

    group.bench_function("contains", |b| {
        b.iter(|| {
            black_box(cache.contains("key-500"));
        });
    });

    group.finish();
}

fn bench_tiered_cache(c: &mut Criterion) {
    let mut group = c.benchmark_group("tiered_cache");
    group.sample_size(50); // Reduce sample size due to disk I/O

    let dir = tempfile::tempdir().unwrap();
    let config = CacheConfig {
        root: Some(dir.path().join("cache")),
        bloom_filter_enabled: true,
        compression_enabled: false, // Disable for pure lookup benchmark
        ..Default::default()
    };
    let cache = TieredCache::with_config(config).unwrap();

    let data = vec![0u8; 4096];

    // Pre-populate
    let mut hashes = Vec::new();
    for i in 0..100 {
        let mut d = data.clone();
        d[0] = i as u8;
        let hash = cache.put(&d, CacheEntryType::Package, None, None).unwrap();
        hashes.push(hash);
    }

    group.throughput(Throughput::Elements(1));

    group.bench_function("get_l1_hit", |b| {
        let hash = &hashes[50];
        b.iter(|| {
            black_box(cache.get(hash).unwrap());
        });
    });

    group.bench_function("contains", |b| {
        let hash = &hashes[50];
        b.iter(|| {
            black_box(cache.contains(hash));
        });
    });

    group.bench_function("contains_miss", |b| {
        let fake_hash = ContentHash::from_bytes(b"not in cache");
        b.iter(|| {
            black_box(cache.contains(&fake_hash));
        });
    });

    group.bench_function("put_small", |b| {
        let mut i = 0usize;
        b.iter(|| {
            let d = format!("small data {i}");
            black_box(
                cache
                    .put(d.as_bytes(), CacheEntryType::Metadata, None, None)
                    .unwrap(),
            );
            i += 1;
        });
    });

    group.finish();
}

fn bench_compression(c: &mut Criterion) {
    let mut group = c.benchmark_group("compression");

    // Test with different data sizes and compressibility
    let sizes = [1024, 10240, 102400];

    for size in sizes {
        // Highly compressible data (repeated pattern)
        let compressible = vec![0xABu8; size];

        // Less compressible data (random-ish)
        let mut less_compressible = vec![0u8; size];
        for (i, b) in less_compressible.iter_mut().enumerate() {
            *b = (i * 37 + i / 256) as u8;
        }

        group.throughput(Throughput::Bytes(size as u64));

        group.bench_with_input(
            BenchmarkId::new("compress_high", size),
            &compressible,
            |b, data| {
                b.iter(|| {
                    black_box(libretto_cache::compress(data, 3).unwrap());
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("compress_low", size),
            &less_compressible,
            |b, data| {
                b.iter(|| {
                    black_box(libretto_cache::compress(data, 3).unwrap());
                });
            },
        );

        let compressed = libretto_cache::compress(&compressible, 3).unwrap();
        group.bench_with_input(
            BenchmarkId::new("decompress", size),
            &compressed,
            |b, data| {
                b.iter(|| {
                    black_box(libretto_cache::decompress(data).unwrap());
                });
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_bloom_filter,
    bench_l1_cache,
    bench_tiered_cache,
    bench_compression,
);
criterion_main!(benches);
