//! Benchmarks for integrity verification.

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use libretto_audit::{HashAlgorithm, hash_file};
use std::io::Write;
use tempfile::NamedTempFile;

fn create_test_file(size: usize) -> NamedTempFile {
    let mut file = NamedTempFile::new().unwrap();
    let data = vec![0u8; size];
    file.write_all(&data).unwrap();
    file.flush().unwrap();
    file
}

fn bench_hash_algorithms(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().unwrap();

    let mut group = c.benchmark_group("hash_algorithms");

    // Test different file sizes
    for size in &[1024, 10_240, 102_400, 1_024_000] {
        group.throughput(Throughput::Bytes(*size as u64));

        // SHA-256
        group.bench_with_input(BenchmarkId::new("sha256", size), size, |b, &size| {
            let file = create_test_file(size);
            b.to_async(&runtime).iter(|| async {
                black_box(hash_file(file.path(), HashAlgorithm::Sha256).await.unwrap())
            });
        });

        // SHA-1
        group.bench_with_input(BenchmarkId::new("sha1", size), size, |b, &size| {
            let file = create_test_file(size);
            b.to_async(&runtime).iter(|| async {
                black_box(hash_file(file.path(), HashAlgorithm::Sha1).await.unwrap())
            });
        });

        // BLAKE3 (should be fastest)
        group.bench_with_input(BenchmarkId::new("blake3", size), size, |b, &size| {
            let file = create_test_file(size);
            b.to_async(&runtime).iter(|| async {
                black_box(hash_file(file.path(), HashAlgorithm::Blake3).await.unwrap())
            });
        });
    }

    group.finish();
}

fn bench_constant_time_comparison(c: &mut Criterion) {
    use libretto_audit::{Hash, HashAlgorithm};

    let hash = Hash::new(
        HashAlgorithm::Sha256,
        "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9".to_string(),
    );

    let correct = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";
    let incorrect = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde8";

    c.bench_function("verify_correct_hash", |b| {
        b.iter(|| {
            hash.verify(correct).unwrap();
            black_box(())
        });
    });

    c.bench_function("verify_incorrect_hash", |b| {
        b.iter(|| black_box(hash.verify(incorrect).unwrap_err()));
    });
}

criterion_group!(
    benches,
    bench_hash_algorithms,
    bench_constant_time_comparison
);
criterion_main!(benches);
