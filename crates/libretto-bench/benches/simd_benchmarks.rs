//! SIMD benchmarks.
//!
//! Benchmarks comparing SIMD-accelerated operations vs scalar implementations.

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use std::time::Duration;

/// Benchmark SIMD-accelerated hashing (BLAKE3).
fn bench_simd_hashing(c: &mut Criterion) {
    let mut group = c.benchmark_group("simd/hashing");

    let sizes = [
        (1024, "1KB"),
        (64 * 1024, "64KB"),
        (1024 * 1024, "1MB"),
        (16 * 1024 * 1024, "16MB"),
    ];

    for (size, label) in sizes {
        let data = vec![0xABu8; size];

        group.throughput(Throughput::Bytes(size as u64));

        // BLAKE3 uses SIMD automatically when available
        group.bench_with_input(BenchmarkId::new("blake3", label), &data, |b, d| {
            b.iter(|| {
                let hash = blake3::hash(black_box(d));
                black_box(hash)
            });
        });

        // SHA256 for comparison (less SIMD optimization)
        group.bench_with_input(BenchmarkId::new("sha256", label), &data, |b, d| {
            b.iter(|| {
                use sha2::{Digest, Sha256};
                let mut hasher = Sha256::new();
                hasher.update(black_box(d));
                black_box(hasher.finalize())
            });
        });
    }

    group.finish();
}

/// Benchmark JSON parsing with sonic-rs (SIMD) vs `serde_json`.
fn bench_simd_json(c: &mut Criterion) {
    let mut group = c.benchmark_group("simd/json");
    group.measurement_time(Duration::from_secs(10));

    for num_deps in [10, 100, 500] {
        let json_content = libretto_bench::fixtures::generate_composer_lock(num_deps);

        group.throughput(Throughput::Bytes(json_content.len() as u64));

        // sonic-rs uses SIMD for parsing
        group.bench_with_input(
            BenchmarkId::new("sonic_rs", num_deps),
            &json_content,
            |b, content| {
                b.iter(|| {
                    let parsed: serde_json::Value = sonic_rs::from_str(black_box(content)).unwrap();
                    black_box(parsed)
                });
            },
        );

        // serde_json for comparison
        group.bench_with_input(
            BenchmarkId::new("serde_json", num_deps),
            &json_content,
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

/// Benchmark memchr (SIMD string searching).
fn bench_simd_string_search(c: &mut Criterion) {
    let mut group = c.benchmark_group("simd/string_search");

    // Large text with multiple occurrences
    let text = "vendor/package".repeat(10000) + "MARKER" + &"vendor/package".repeat(10000);
    let bytes = text.as_bytes();

    group.throughput(Throughput::Bytes(bytes.len() as u64));

    // memchr uses SIMD
    group.bench_function("memchr_find", |b| {
        b.iter(|| {
            let pos = memchr::memchr(b'M', black_box(bytes));
            black_box(pos)
        });
    });

    // Standard library for comparison
    group.bench_function("std_find", |b| {
        b.iter(|| {
            let pos = bytes.iter().position(|&b| b == b'M');
            black_box(pos)
        });
    });

    // Find substring
    group.bench_function("memchr_memmem", |b| {
        let needle = b"MARKER";
        b.iter(|| {
            let finder = memchr::memmem::find(black_box(bytes), needle);
            black_box(finder)
        });
    });

    group.finish();
}

/// Benchmark version string comparison.
fn bench_version_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("simd/version");

    let versions: Vec<String> = (0..1000)
        .map(|i| format!("{}.{}.{}", i / 100, (i / 10) % 10, i % 10))
        .collect();

    group.throughput(Throughput::Elements(versions.len() as u64));

    group.bench_function("parse_and_compare", |b| {
        b.iter(|| {
            let mut parsed: Vec<_> = versions
                .iter()
                .filter_map(|v| semver::Version::parse(v).ok())
                .collect();
            parsed.sort();
            black_box(parsed)
        });
    });

    group.bench_function("string_sort", |b| {
        b.iter(|| {
            let mut cloned = versions.clone();
            cloned.sort();
            black_box(cloned)
        });
    });

    group.finish();
}

/// Benchmark data compression (zstd uses SIMD).
fn bench_simd_compression(c: &mut Criterion) {
    let mut group = c.benchmark_group("simd/compression");
    group.sample_size(20);

    // Compressible data (JSON-like content)
    let json = libretto_bench::fixtures::generate_composer_lock(100);
    let data = json.as_bytes();

    group.throughput(Throughput::Bytes(data.len() as u64));

    // zstd compression levels
    for level in [1, 3, 9] {
        group.bench_with_input(
            BenchmarkId::new("zstd_compress", level),
            &(data, level),
            |b, (d, lvl)| {
                b.iter(|| {
                    let compressed =
                        zstd::encode_all(std::io::Cursor::new(black_box(*d)), *lvl).unwrap();
                    black_box(compressed)
                });
            },
        );
    }

    // Prepare compressed data for decompression benchmark
    let compressed = zstd::encode_all(std::io::Cursor::new(data), 3).unwrap();

    group.bench_function("zstd_decompress", |b| {
        b.iter(|| {
            let decompressed =
                zstd::decode_all(std::io::Cursor::new(black_box(&compressed))).unwrap();
            black_box(decompressed)
        });
    });

    group.finish();
}

/// Benchmark hex encoding/decoding.
fn bench_hex_encoding(c: &mut Criterion) {
    let mut group = c.benchmark_group("simd/hex");

    let hash_bytes = [0xABu8; 32]; // SHA256-like hash
    let hex_string = hex::encode(hash_bytes);

    group.bench_function("encode_32bytes", |b| {
        b.iter(|| {
            let encoded = hex::encode(black_box(&hash_bytes));
            black_box(encoded)
        });
    });

    group.bench_function("decode_64chars", |b| {
        b.iter(|| {
            let decoded = hex::decode(black_box(&hex_string)).unwrap();
            black_box(decoded)
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_simd_hashing,
    bench_simd_json,
    bench_simd_string_search,
    bench_version_comparison,
    bench_simd_compression,
    bench_hex_encoding,
);

criterion_main!(benches);
