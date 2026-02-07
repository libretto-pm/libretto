//! VCS operations benchmarks.
//!
//! Comprehensive benchmarks for Git clone, checkout, URL parsing, and parallel operations.

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use libretto_vcs::{CloneOptions, GitProtocol, VcsManager, VcsRef, VcsType, VcsUrl};
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

/// Create a temporary git repository for benchmarking.
fn create_test_repo(num_files: usize, num_commits: usize) -> Option<(tempfile::TempDir, PathBuf)> {
    let temp_dir = tempfile::tempdir().ok()?;
    let repo_path = temp_dir.path().join("test-repo");
    std::fs::create_dir(&repo_path).ok()?;

    // Initialize git repo
    Command::new("git")
        .current_dir(&repo_path)
        .args(["init", "--initial-branch=main"])
        .output()
        .ok()?;

    Command::new("git")
        .current_dir(&repo_path)
        .args(["config", "user.email", "bench@test.com"])
        .output()
        .ok()?;

    Command::new("git")
        .current_dir(&repo_path)
        .args(["config", "user.name", "Benchmark"])
        .output()
        .ok()?;

    // Create files and commits
    for commit in 0..num_commits {
        for file in 0..num_files {
            let file_path = repo_path.join(format!("file_{file}.txt"));
            std::fs::write(
                &file_path,
                format!("Content for commit {commit}, file {file}"),
            )
            .ok()?;
        }

        Command::new("git")
            .current_dir(&repo_path)
            .args(["add", "."])
            .output()
            .ok()?;

        Command::new("git")
            .current_dir(&repo_path)
            .args(["commit", "-m", &format!("Commit {commit}")])
            .output()
            .ok()?;
    }

    // Create a bare clone for fetch operations
    let bare_path = temp_dir.path().join("bare-repo.git");
    Command::new("git")
        .args(["clone", "--bare", repo_path.to_str()?, bare_path.to_str()?])
        .output()
        .ok()?;

    Some((temp_dir, bare_path))
}

/// Benchmark `VcsType` detection.
fn bench_vcs_detection(c: &mut Criterion) {
    let mut group = c.benchmark_group("vcs/detection");

    let temp_dir = tempfile::tempdir().unwrap();
    let git_dir = temp_dir.path().join("git-project");
    std::fs::create_dir(&git_dir).unwrap();
    std::fs::create_dir(git_dir.join(".git")).unwrap();

    group.bench_function("detect_git", |b| {
        b.iter(|| black_box(VcsType::detect(&git_dir)));
    });

    let svn_dir = temp_dir.path().join("svn-project");
    std::fs::create_dir(&svn_dir).unwrap();
    std::fs::create_dir(svn_dir.join(".svn")).unwrap();

    group.bench_function("detect_svn", |b| {
        b.iter(|| black_box(VcsType::detect(&svn_dir)));
    });

    let hg_dir = temp_dir.path().join("hg-project");
    std::fs::create_dir(&hg_dir).unwrap();
    std::fs::create_dir(hg_dir.join(".hg")).unwrap();

    group.bench_function("detect_hg", |b| {
        b.iter(|| black_box(VcsType::detect(&hg_dir)));
    });

    let not_vcs = temp_dir.path().join("not-vcs");
    std::fs::create_dir(&not_vcs).unwrap();

    group.bench_function("detect_none", |b| {
        b.iter(|| black_box(VcsType::detect(&not_vcs)));
    });

    group.finish();
}

/// Benchmark `VcsRef` parsing for various ref formats.
fn bench_vcs_ref_parsing(c: &mut Criterion) {
    let mut group = c.benchmark_group("vcs/ref_parsing");

    let refs = [
        ("main", "branch"),
        ("develop", "branch"),
        ("v1.0.0", "tag"),
        ("v10.20.30", "semver_tag"),
        ("abc123def456abc123def456abc123def456abcd", "commit_sha"),
        ("feature/my-cool-feature", "feature_branch"),
        ("release/v2.0", "release_branch"),
        ("HEAD", "head"),
    ];

    for (ref_str, label) in refs {
        group.bench_with_input(BenchmarkId::new("parse", label), &ref_str, |b, r| {
            b.iter(|| black_box(VcsRef::parse(r)));
        });
    }

    group.finish();
}

/// Benchmark `VcsUrl` parsing for various URL formats.
fn bench_url_parsing(c: &mut Criterion) {
    let mut group = c.benchmark_group("vcs/url_parsing");

    let urls = [
        ("https://github.com/owner/repo.git", "https_github"),
        ("git@github.com:owner/repo.git", "ssh_github"),
        (
            "https://gitlab.com/group/subgroup/repo.git",
            "https_gitlab_nested",
        ),
        ("git@bitbucket.org:team/repo.git", "ssh_bitbucket"),
        ("https://packagist.org/packages/vendor/package", "packagist"),
        ("symfony/console", "shorthand"),
        ("vendor/package", "vendor_package"),
        ("file:///local/path/to/repo", "file_protocol"),
    ];

    for (url, label) in urls {
        group.bench_with_input(BenchmarkId::new("parse", label), &url, |b, u| {
            b.iter(|| black_box(VcsUrl::parse(u)));
        });
    }

    // Batch parse benchmark
    group.bench_function("batch_parse_10", |b| {
        let test_urls: Vec<&str> = urls.iter().map(|(u, _)| *u).take(10).collect();
        b.iter(|| {
            let results: Vec<_> = test_urls.iter().map(|u| VcsUrl::parse(u)).collect();
            black_box(results)
        });
    });

    group.finish();
}

/// Benchmark `CloneOptions` construction.
fn bench_clone_options(c: &mut Criterion) {
    let mut group = c.benchmark_group("vcs/clone_options");

    group.bench_function("default", |b| {
        b.iter(|| black_box(CloneOptions::default()));
    });

    group.bench_function("full_options", |b| {
        b.iter(|| {
            let opts = CloneOptions {
                recursive: true,
                lfs: false,
                timeout_secs: Some(60),
                ..Default::default()
            };
            black_box(opts)
        });
    });

    group.finish();
}

/// Benchmark `VcsManager` creation.
fn bench_manager_creation(c: &mut Criterion) {
    let mut group = c.benchmark_group("vcs/manager");

    group.bench_function("create_default", |b| {
        b.iter(|| black_box(VcsManager::new()));
    });

    group.bench_function("create_with_cache", |b| {
        b.iter_with_setup(
            || tempfile::tempdir().unwrap(),
            |cache_dir| {
                let _ = black_box(VcsManager::with_cache(cache_dir.path().to_path_buf()));
            },
        );
    });

    group.finish();
}

/// Benchmark local clone operation.
fn bench_local_clone(c: &mut Criterion) {
    let mut group = c.benchmark_group("vcs/clone");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(30));

    if let Some((_temp_dir, bare_path)) = create_test_repo(10, 5) {
        group.bench_function("local_shallow", |b| {
            b.iter_with_setup(
                || tempfile::tempdir().unwrap(),
                |dest_dir| {
                    let manager = VcsManager::new();
                    let result = manager.clone(
                        bare_path.to_str().unwrap(),
                        &dest_dir.path().join("clone"),
                        None,
                    );
                    let _ = black_box(result);
                },
            );
        });
    }

    group.finish();
}

/// Benchmark checkout operation.
fn bench_checkout(c: &mut Criterion) {
    let mut group = c.benchmark_group("vcs/checkout");
    group.sample_size(20);

    if let Some((temp_dir, _bare_path)) = create_test_repo(5, 10) {
        let repo_path = temp_dir.path().join("test-repo");

        group.bench_function("checkout_ref", |b| {
            b.iter(|| {
                // Checkout HEAD~5
                let output = Command::new("git")
                    .current_dir(&repo_path)
                    .args(["checkout", "HEAD~5", "--quiet"])
                    .output();

                // Return to main
                let _ = Command::new("git")
                    .current_dir(&repo_path)
                    .args(["checkout", "main", "--quiet", "--force"])
                    .output();

                black_box(output)
            });
        });
    }

    group.finish();
}

/// Benchmark URL protocol detection.
fn bench_protocol_detection(c: &mut Criterion) {
    let mut group = c.benchmark_group("vcs/protocol");

    let urls_with_protocols = [
        ("https://github.com/owner/repo.git", GitProtocol::Https),
        ("git@github.com:owner/repo.git", GitProtocol::Ssh),
        ("git://github.com/owner/repo.git", GitProtocol::Git),
        ("file:///local/repo", GitProtocol::File),
    ];

    for (url, expected_protocol) in urls_with_protocols {
        group.bench_with_input(
            BenchmarkId::new("detect", format!("{expected_protocol:?}")),
            &url,
            |b, u| {
                b.iter(|| {
                    let parsed = VcsUrl::parse(u).unwrap();
                    black_box(parsed.protocol)
                });
            },
        );
    }

    group.finish();
}

/// Benchmark repository status checking.
fn bench_repo_status(c: &mut Criterion) {
    let mut group = c.benchmark_group("vcs/status");
    group.sample_size(20);

    if let Some((temp_dir, _bare_path)) = create_test_repo(5, 3) {
        let repo_path = temp_dir.path().join("test-repo");

        group.bench_function("git_status", |b| {
            b.iter(|| {
                let output = Command::new("git")
                    .current_dir(&repo_path)
                    .args(["status", "--porcelain"])
                    .output();
                black_box(output)
            });
        });

        group.bench_function("git_rev_parse", |b| {
            b.iter(|| {
                let output = Command::new("git")
                    .current_dir(&repo_path)
                    .args(["rev-parse", "HEAD"])
                    .output();
                black_box(output)
            });
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_vcs_detection,
    bench_vcs_ref_parsing,
    bench_url_parsing,
    bench_clone_options,
    bench_manager_creation,
    bench_local_clone,
    bench_checkout,
    bench_protocol_detection,
    bench_repo_status,
);

criterion_main!(benches);
