//! Benchmarks for lock file operations.
//!
//! Performance targets:
//! - Lock file operations <10ms for 500 packages
//! - Atomic writes with full integrity verification
//! - SIMD-accelerated hashing and comparison

use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use libretto_lockfile::{
    AtomicWriter, ContentHasher, DeterministicSerializer, IntegrityHasher, LockGenerator,
    LockedPackage, LockfileManager, PackageDistInfo, PackageSourceInfo, Validator, compute_diff,
};
use std::collections::BTreeMap;
use tempfile::TempDir;

/// Generate a lock file with N packages.
fn generate_lock_with_packages(count: usize) -> libretto_lockfile::ComposerLock {
    let mut generator = LockGenerator::new();
    generator.minimum_stability("stable").prefer_stable(true);

    // Add production packages
    for i in 0..count {
        let mut pkg = LockedPackage::new(
            format!("vendor{}/package{}", i / 100, i),
            format!("{}.0.0", i % 10),
        );
        pkg.description = Some(format!("Package {i} description"));
        pkg.license = vec!["MIT".to_string()];
        pkg.source = Some(PackageSourceInfo::git(
            format!("https://github.com/vendor{}/package{}.git", i / 100, i),
            format!("abc{i:06x}"),
        ));
        pkg.dist = Some(
            PackageDistInfo::zip(format!(
                "https://packagist.org/dl/vendor{}/package{}.zip",
                i / 100,
                i
            ))
            .with_shasum(format!("{i:040x}")),
        );

        // Add some dependencies
        if i > 0 {
            pkg.require.insert(
                format!("vendor{}/package{}", (i - 1) / 100, i - 1),
                "^1.0".to_string(),
            );
        }

        generator.add_package(pkg);
    }

    // Add dev packages (10% of total)
    for i in 0..count / 10 {
        let pkg = LockedPackage::new(format!("dev{}/test{}", i / 10, i), format!("{i}.0.0-dev"));
        generator.add_package_dev(pkg);
    }

    let mut require = BTreeMap::new();
    require.insert("vendor0/package0".to_string(), "^1.0".to_string());

    generator.generate(&require, &BTreeMap::new())
}

fn bench_serialization(c: &mut Criterion) {
    let mut group = c.benchmark_group("serialization");

    for size in &[10, 100, 500, 1000] {
        let lock = generate_lock_with_packages(*size);

        group.throughput(Throughput::Elements(*size as u64));
        group.bench_with_input(BenchmarkId::new("deterministic", size), &lock, |b, lock| {
            b.iter(|| {
                let result = DeterministicSerializer::serialize(black_box(lock)).unwrap();
                black_box(result)
            })
        });

        group.bench_with_input(BenchmarkId::new("sonic_rs", size), &lock, |b, lock| {
            b.iter(|| {
                let result = sonic_rs::to_string_pretty(black_box(lock)).unwrap();
                black_box(result)
            })
        });
    }

    group.finish();
}

fn bench_parsing(c: &mut Criterion) {
    let mut group = c.benchmark_group("parsing");

    for size in &[10, 100, 500, 1000] {
        let lock = generate_lock_with_packages(*size);
        let json = DeterministicSerializer::serialize(&lock).unwrap();

        group.throughput(Throughput::Bytes(json.len() as u64));
        group.bench_with_input(BenchmarkId::new("sonic_rs", size), &json, |b, json| {
            b.iter(|| {
                let result: libretto_lockfile::ComposerLock =
                    sonic_rs::from_str(black_box(json)).unwrap();
                black_box(result)
            })
        });
    }

    group.finish();
}

fn bench_hashing(c: &mut Criterion) {
    let mut group = c.benchmark_group("hashing");

    // Content hash (MD5-based for Composer compatibility)
    let mut require = BTreeMap::new();
    for i in 0..100 {
        require.insert(format!("vendor{}/package{}", i / 10, i), "^1.0".to_string());
    }

    group.bench_function("content_hash_100_deps", |b| {
        b.iter(|| {
            let hash = ContentHasher::compute_content_hash(
                black_box(&require),
                black_box(&BTreeMap::new()),
                Some("stable"),
                Some(true),
                None,
                black_box(&BTreeMap::new()),
                &BTreeMap::new(),
            );
            black_box(hash)
        });
    });

    // Integrity hash (BLAKE3)
    let data: Vec<u8> = (0..1024 * 1024).map(|i| (i % 256) as u8).collect();

    group.throughput(Throughput::Bytes(data.len() as u64));
    group.bench_function("blake3_1mb", |b| {
        b.iter(|| {
            let hash = IntegrityHasher::hash_bytes(black_box(&data));
            black_box(hash)
        })
    });

    group.finish();
}

fn bench_diff(c: &mut Criterion) {
    let mut group = c.benchmark_group("diff");

    for size in &[100, 500] {
        let old_lock = generate_lock_with_packages(*size);

        // Create a modified version
        let mut new_lock = old_lock.clone();
        // Upgrade some packages
        for pkg in new_lock.packages.iter_mut().take(10) {
            pkg.version = "2.0.0".to_string();
        }
        // Add some packages
        for i in 0..5 {
            new_lock.packages.push(LockedPackage::new(
                format!("new/package{i}"),
                "1.0.0".to_string(),
            ));
        }
        // Remove some packages
        new_lock.packages.truncate(new_lock.packages.len() - 5);

        group.bench_with_input(
            BenchmarkId::new("compute_diff", size),
            &(&old_lock, &new_lock),
            |b, (old, new)| {
                b.iter(|| {
                    let diff = compute_diff(black_box(old), black_box(new));
                    black_box(diff)
                })
            },
        );
    }

    group.finish();
}

fn bench_validation(c: &mut Criterion) {
    let mut group = c.benchmark_group("validation");

    for size in &[100, 500] {
        let lock = generate_lock_with_packages(*size);
        let validator = Validator::new();

        group.bench_with_input(BenchmarkId::new("validate", size), &lock, |b, lock| {
            b.iter(|| {
                let result = validator.validate(black_box(lock));
                black_box(result)
            })
        });

        let validator_strict = Validator::strict();
        group.bench_with_input(
            BenchmarkId::new("validate_strict", size),
            &lock,
            |b, lock| {
                b.iter(|| {
                    let result = validator_strict.validate(black_box(lock));
                    black_box(result)
                })
            },
        );
    }

    group.finish();
}

fn bench_atomic_write(c: &mut Criterion) {
    let mut group = c.benchmark_group("atomic_write");

    for size in &[100, 500] {
        let lock = generate_lock_with_packages(*size);
        let json = DeterministicSerializer::serialize(&lock).unwrap();

        group.throughput(Throughput::Bytes(json.len() as u64));
        group.bench_with_input(BenchmarkId::new("write", size), &json, |b, json| {
            let dir = TempDir::new().unwrap();
            let path = dir.path().join("composer.lock");

            b.iter(|| {
                let mut writer = AtomicWriter::new(&path).unwrap();
                writer.content(black_box(json.as_bytes()));
                writer.no_backup();
                let result = writer.commit().unwrap();
                black_box(result)
            })
        });
    }

    group.finish();
}

fn bench_full_workflow(c: &mut Criterion) {
    let mut group = c.benchmark_group("full_workflow");

    for size in &[100, 500] {
        // Generate, serialize, write, read, validate, diff
        group.bench_with_input(
            BenchmarkId::new("generate_write_read", size),
            size,
            |b, &size| {
                let dir = TempDir::new().unwrap();
                let path = dir.path().join("composer.lock");
                let manager = LockfileManager::new(&path).unwrap();

                b.iter(|| {
                    // Generate
                    let lock = generate_lock_with_packages(size);

                    // Write
                    manager.write(black_box(&lock)).unwrap();

                    // Read
                    let loaded = manager.read().unwrap();

                    // Validate
                    let _result = Validator::new().validate(&loaded);

                    black_box(loaded)
                })
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_serialization,
    bench_parsing,
    bench_hashing,
    bench_diff,
    bench_validation,
    bench_atomic_write,
    bench_full_workflow,
);
criterion_main!(benches);
