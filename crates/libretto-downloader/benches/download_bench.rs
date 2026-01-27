//! Benchmarks for the downloader crate.

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use libretto_downloader::{
    checksum::{bytes_to_hex, MultiHasher},
    config::DownloadConfig,
    throttle::BandwidthThrottler,
};

fn bench_multi_hasher(c: &mut Criterion) {
    let mut group = c.benchmark_group("hashing");

    // Test data sizes
    let sizes = [1024, 64 * 1024, 1024 * 1024, 16 * 1024 * 1024];

    for size in sizes {
        let data = vec![0u8; size];

        group.throughput(Throughput::Bytes(size as u64));

        group.bench_function(format!("blake3_{size}"), |b| {
            b.iter(|| {
                let mut hasher = MultiHasher::new();
                hasher.update(black_box(&data));
                black_box(hasher.finalize())
            })
        });

        group.bench_function(format!("all_hashes_{size}"), |b| {
            b.iter(|| {
                let mut hasher = MultiHasher::all();
                hasher.update(black_box(&data));
                black_box(hasher.finalize())
            })
        });
    }

    group.finish();
}

fn bench_hex_conversion(c: &mut Criterion) {
    let mut group = c.benchmark_group("hex");

    let hash = [0xab_u8; 32];

    group.bench_function("bytes_to_hex_32", |b| {
        b.iter(|| black_box(bytes_to_hex(black_box(&hash))))
    });

    group.finish();
}

fn bench_config(c: &mut Criterion) {
    let mut group = c.benchmark_group("config");

    group.bench_function("default_config", |b| {
        b.iter(|| black_box(DownloadConfig::default()))
    });

    group.bench_function("config_builder", |b| {
        b.iter(|| {
            black_box(
                DownloadConfig::builder()
                    .max_concurrent(50)
                    .bandwidth_limit(Some(1_000_000))
                    .show_progress(false)
                    .build(),
            )
        })
    });

    group.finish();
}

fn bench_throttler(c: &mut Criterion) {
    let mut group = c.benchmark_group("throttler");

    group.bench_function("create_unlimited", |b| {
        b.iter(|| black_box(BandwidthThrottler::unlimited()))
    });

    group.bench_function("create_limited", |b| {
        b.iter(|| black_box(BandwidthThrottler::new(Some(1_000_000))))
    });

    let throttler = BandwidthThrottler::unlimited();
    group.bench_function("try_acquire_unlimited", |b| {
        b.iter(|| black_box(throttler.try_acquire(1024)))
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_multi_hasher,
    bench_hex_conversion,
    bench_config,
    bench_throttler,
);

criterion_main!(benches);
