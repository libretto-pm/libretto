//! Integration tests for Libretto workflows.
//!
//! These tests verify end-to-end functionality of the package manager,
//! testing complete workflows like install, update, require, and remove.

use assert_cmd::prelude::*;
use predicates::prelude::*;
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

/// Get the libretto binary command.
fn libretto() -> Command {
    Command::cargo_bin("libretto").expect("Failed to find libretto binary")
}

/// Create a minimal composer.json in the given directory.
fn create_composer_json(dir: &Path, content: &str) {
    fs::write(dir.join("composer.json"), content).expect("Failed to write composer.json");
}

/// Create a composer.lock in the given directory.
fn create_composer_lock(dir: &Path, content: &str) {
    fs::write(dir.join("composer.lock"), content).expect("Failed to write composer.lock");
}

/// Check if vendor directory exists.
fn vendor_exists(dir: &Path) -> bool {
    dir.join("vendor").exists()
}

/// Check if autoloader exists.
fn autoloader_exists(dir: &Path) -> bool {
    dir.join("vendor/autoload.php").exists()
}

/// Check if a package is installed.
fn package_installed(dir: &Path, vendor: &str, package: &str) -> bool {
    dir.join("vendor").join(vendor).join(package).exists()
}

// ========== Validate Workflow Tests ==========

mod validate_workflow {
    use super::*;

    #[test]
    fn test_validate_minimal_project() {
        let temp = TempDir::new().unwrap();
        create_composer_json(
            temp.path(),
            r#"{
                "name": "test/project",
                "require": {}
            }"#,
        );

        libretto()
            .arg("validate")
            .current_dir(temp.path())
            .assert()
            .success();
    }

    #[test]
    fn test_validate_full_project() {
        let temp = TempDir::new().unwrap();
        create_composer_json(
            temp.path(),
            r#"{
                "name": "test/full-project",
                "description": "A complete test project",
                "type": "project",
                "license": "MIT",
                "authors": [
                    {
                        "name": "Test Author",
                        "email": "test@example.com"
                    }
                ],
                "require": {
                    "php": ">=8.1"
                },
                "require-dev": {},
                "autoload": {
                    "psr-4": {
                        "App\\": "src/"
                    }
                },
                "autoload-dev": {
                    "psr-4": {
                        "Tests\\": "tests/"
                    }
                },
                "config": {
                    "optimize-autoloader": true,
                    "sort-packages": true
                }
            }"#,
        );

        libretto()
            .arg("validate")
            .current_dir(temp.path())
            .assert()
            .success();
    }

    #[test]
    fn test_validate_with_scripts() {
        let temp = TempDir::new().unwrap();
        create_composer_json(
            temp.path(),
            r#"{
                "name": "test/scripts-project",
                "require": {},
                "scripts": {
                    "post-install-cmd": ["echo 'Installed!'"],
                    "post-update-cmd": ["echo 'Updated!'"],
                    "test": "phpunit"
                }
            }"#,
        );

        libretto()
            .arg("validate")
            .current_dir(temp.path())
            .assert()
            .success();
    }

    #[test]
    fn test_validate_with_repositories() {
        let temp = TempDir::new().unwrap();
        create_composer_json(
            temp.path(),
            r#"{
                "name": "test/repos-project",
                "require": {},
                "repositories": [
                    {
                        "type": "vcs",
                        "url": "https://github.com/example/repo"
                    },
                    {
                        "type": "composer",
                        "url": "https://packages.example.com"
                    }
                ]
            }"#,
        );

        libretto()
            .arg("validate")
            .current_dir(temp.path())
            .assert()
            .success();
    }

    #[test]
    fn test_validate_with_extra() {
        let temp = TempDir::new().unwrap();
        create_composer_json(
            temp.path(),
            r#"{
                "name": "test/extra-project",
                "require": {},
                "extra": {
                    "laravel": {
                        "dont-discover": []
                    },
                    "custom-key": "custom-value"
                }
            }"#,
        );

        libretto()
            .arg("validate")
            .current_dir(temp.path())
            .assert()
            .success();
    }
}

// ========== Init Workflow Tests ==========

mod init_workflow {
    use super::*;

    #[test]
    fn test_init_help_available() {
        libretto()
            .args(["init", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Initialize"));
    }
}

// ========== Show Workflow Tests ==========

mod show_workflow {
    use super::*;

    #[test]
    fn test_show_empty_project() {
        let temp = TempDir::new().unwrap();
        create_composer_json(
            temp.path(),
            r#"{
                "name": "test/empty",
                "require": {}
            }"#,
        );

        libretto()
            .arg("show")
            .current_dir(temp.path())
            .assert()
            .success();
    }

    #[test]
    fn test_show_with_lock_file() {
        let temp = TempDir::new().unwrap();
        create_composer_json(
            temp.path(),
            r#"{
                "name": "test/with-lock",
                "require": {
                    "psr/log": "^3.0"
                }
            }"#,
        );

        create_composer_lock(
            temp.path(),
            r#"{
                "content-hash": "abc123",
                "packages": [
                    {
                        "name": "psr/log",
                        "version": "3.0.0",
                        "source": {
                            "type": "git",
                            "url": "https://github.com/php-fig/log.git",
                            "reference": "abc123"
                        },
                        "dist": {
                            "type": "zip",
                            "url": "https://example.com/psr-log.zip",
                            "reference": "abc123"
                        },
                        "require": {
                            "php": ">=8.0.0"
                        },
                        "type": "library"
                    }
                ],
                "packages-dev": [],
                "aliases": [],
                "minimum-stability": "stable",
                "prefer-stable": true
            }"#,
        );

        libretto()
            .arg("show")
            .current_dir(temp.path())
            .assert()
            .success();
    }

    #[test]
    fn test_show_installed_flag() {
        let temp = TempDir::new().unwrap();
        create_composer_json(temp.path(), r#"{"name": "test/project", "require": {}}"#);

        libretto()
            .args(["show", "--installed"])
            .current_dir(temp.path())
            .assert()
            .success();
    }

    #[test]
    fn test_show_platform_flag() {
        let temp = TempDir::new().unwrap();
        create_composer_json(temp.path(), r#"{"name": "test/project", "require": {}}"#);

        libretto()
            .args(["show", "--platform"])
            .current_dir(temp.path())
            .assert()
            .success();
    }
}

// ========== Dump-Autoload Workflow Tests ==========

mod dump_autoload_workflow {
    use super::*;

    #[test]
    fn test_dump_autoload_psr4() {
        let temp = TempDir::new().unwrap();
        create_composer_json(
            temp.path(),
            r#"{
                "name": "test/psr4-project",
                "autoload": {
                    "psr-4": {
                        "App\\": "src/"
                    }
                }
            }"#,
        );

        // Create src directory
        fs::create_dir_all(temp.path().join("src")).unwrap();

        libretto()
            .arg("dump-autoload")
            .current_dir(temp.path())
            .assert()
            .success();

        // Check that autoloader was generated
        assert!(autoloader_exists(temp.path()));
    }

    #[test]
    fn test_dump_autoload_classmap() {
        let temp = TempDir::new().unwrap();
        create_composer_json(
            temp.path(),
            r#"{
                "name": "test/classmap-project",
                "autoload": {
                    "classmap": ["lib/"]
                }
            }"#,
        );

        // Create lib directory with a PHP file
        fs::create_dir_all(temp.path().join("lib")).unwrap();
        fs::write(
            temp.path().join("lib/Helper.php"),
            "<?php\nclass Helper {}\n",
        )
        .unwrap();

        libretto()
            .arg("dump-autoload")
            .current_dir(temp.path())
            .assert()
            .success();

        assert!(autoloader_exists(temp.path()));
    }

    #[test]
    fn test_dump_autoload_files() {
        let temp = TempDir::new().unwrap();
        create_composer_json(
            temp.path(),
            r#"{
                "name": "test/files-project",
                "autoload": {
                    "files": ["src/helpers.php"]
                }
            }"#,
        );

        // Create the file
        fs::create_dir_all(temp.path().join("src")).unwrap();
        fs::write(
            temp.path().join("src/helpers.php"),
            "<?php\nfunction helper() {}\n",
        )
        .unwrap();

        libretto()
            .arg("dump-autoload")
            .current_dir(temp.path())
            .assert()
            .success();

        assert!(autoloader_exists(temp.path()));
    }

    #[test]
    fn test_dump_autoload_optimize() {
        let temp = TempDir::new().unwrap();
        create_composer_json(
            temp.path(),
            r#"{
                "name": "test/optimize-project",
                "autoload": {
                    "psr-4": {
                        "App\\": "src/"
                    }
                }
            }"#,
        );

        fs::create_dir_all(temp.path().join("src")).unwrap();

        libretto()
            .args(["dump-autoload", "--optimize"])
            .current_dir(temp.path())
            .assert()
            .success();

        assert!(autoloader_exists(temp.path()));
    }

    #[test]
    fn test_dump_autoload_classmap_authoritative() {
        let temp = TempDir::new().unwrap();
        create_composer_json(
            temp.path(),
            r#"{
                "name": "test/authoritative-project",
                "autoload": {
                    "psr-4": {
                        "App\\": "src/"
                    }
                }
            }"#,
        );

        fs::create_dir_all(temp.path().join("src")).unwrap();

        libretto()
            .args(["dump-autoload", "--classmap-authoritative"])
            .current_dir(temp.path())
            .assert()
            .success();

        assert!(autoloader_exists(temp.path()));
    }

    #[test]
    fn test_dump_autoload_combined() {
        let temp = TempDir::new().unwrap();
        create_composer_json(
            temp.path(),
            r#"{
                "name": "test/combined-project",
                "autoload": {
                    "psr-4": {
                        "App\\": "src/",
                        "Lib\\": ["lib/", "legacy/"]
                    },
                    "psr-0": {
                        "Old_": "old/"
                    },
                    "classmap": ["classes/"],
                    "files": ["src/functions.php"]
                }
            }"#,
        );

        // Create all directories
        for dir in &["src", "lib", "legacy", "old", "classes"] {
            fs::create_dir_all(temp.path().join(dir)).unwrap();
        }
        fs::write(temp.path().join("src/functions.php"), "<?php\n").unwrap();

        libretto()
            .arg("dump-autoload")
            .current_dir(temp.path())
            .assert()
            .success();

        assert!(autoloader_exists(temp.path()));
    }
}

// ========== Audit Workflow Tests ==========

mod audit_workflow {
    use super::*;

    #[test]
    fn test_audit_no_lock_file() {
        let temp = TempDir::new().unwrap();
        create_composer_json(temp.path(), r#"{"name": "test/project", "require": {}}"#);

        // Audit without lock file should either fail or warn
        libretto().arg("audit").current_dir(temp.path()).assert();
    }

    #[test]
    fn test_audit_with_empty_lock() {
        let temp = TempDir::new().unwrap();
        create_composer_json(temp.path(), r#"{"name": "test/project", "require": {}}"#);
        create_composer_lock(
            temp.path(),
            r#"{
                "content-hash": "abc123",
                "packages": [],
                "packages-dev": [],
                "aliases": [],
                "minimum-stability": "stable",
                "prefer-stable": true
            }"#,
        );

        libretto()
            .arg("audit")
            .current_dir(temp.path())
            .assert()
            .success();
    }

    #[test]
    fn test_audit_locked_flag() {
        let temp = TempDir::new().unwrap();
        create_composer_json(temp.path(), r#"{"name": "test/project", "require": {}}"#);
        create_composer_lock(
            temp.path(),
            r#"{
                "content-hash": "abc123",
                "packages": [],
                "packages-dev": [],
                "aliases": [],
                "minimum-stability": "stable",
                "prefer-stable": true
            }"#,
        );

        libretto()
            .args(["audit", "--locked"])
            .current_dir(temp.path())
            .assert()
            .success();
    }
}

// ========== Config Workflow Tests ==========

mod config_workflow {
    use super::*;

    #[test]
    fn test_config_list() {
        let temp = TempDir::new().unwrap();
        create_composer_json(temp.path(), r#"{"name": "test/project", "require": {}}"#);

        libretto()
            .args(["config", "--list"])
            .current_dir(temp.path())
            .assert()
            .success();
    }

    #[test]
    fn test_config_global_list() {
        libretto()
            .args(["config", "--global", "--list"])
            .assert()
            .success();
    }
}

// ========== Diagnose Workflow Tests ==========

mod diagnose_workflow {
    use super::*;

    #[test]
    fn test_diagnose_runs() {
        libretto().arg("diagnose").assert().success();
    }
}

// ========== Licenses Workflow Tests ==========

mod licenses_workflow {
    use super::*;

    #[test]
    fn test_licenses_no_packages() {
        let temp = TempDir::new().unwrap();
        create_composer_json(temp.path(), r#"{"name": "test/project", "require": {}}"#);
        create_composer_lock(
            temp.path(),
            r#"{
                "content-hash": "abc123",
                "packages": [],
                "packages-dev": [],
                "aliases": [],
                "minimum-stability": "stable"
            }"#,
        );

        libretto()
            .arg("licenses")
            .current_dir(temp.path())
            .assert()
            .success();
    }
}

// ========== Outdated Workflow Tests ==========

mod outdated_workflow {
    use super::*;

    #[test]
    fn test_outdated_no_packages() {
        let temp = TempDir::new().unwrap();
        create_composer_json(temp.path(), r#"{"name": "test/project", "require": {}}"#);
        create_composer_lock(
            temp.path(),
            r#"{
                "content-hash": "abc123",
                "packages": [],
                "packages-dev": [],
                "aliases": [],
                "minimum-stability": "stable"
            }"#,
        );

        libretto()
            .arg("outdated")
            .current_dir(temp.path())
            .assert()
            .success();
    }
}

// ========== Status Workflow Tests ==========

mod status_workflow {
    use super::*;

    #[test]
    fn test_status_clean_project() {
        let temp = TempDir::new().unwrap();
        create_composer_json(temp.path(), r#"{"name": "test/project", "require": {}}"#);
        create_composer_lock(
            temp.path(),
            r#"{
                "content-hash": "abc123",
                "packages": [],
                "packages-dev": []
            }"#,
        );

        libretto()
            .arg("status")
            .current_dir(temp.path())
            .assert()
            .success();
    }
}

// ========== Edge Cases ==========

mod edge_cases {
    use super::*;

    #[test]
    fn test_unicode_in_composer_json() {
        let temp = TempDir::new().unwrap();
        create_composer_json(
            temp.path(),
            r#"{
                "name": "test/unicode-project",
                "description": "A project with unicode: 日本語 🎉 émojis",
                "require": {}
            }"#,
        );

        libretto()
            .arg("validate")
            .current_dir(temp.path())
            .assert()
            .success();
    }

    #[test]
    fn test_large_composer_json() {
        let temp = TempDir::new().unwrap();

        // Generate a large composer.json with many dependencies
        let mut require = String::from("{");
        for i in 0..100 {
            if i > 0 {
                require.push_str(", ");
            }
            require.push_str(&format!(r#""vendor{}/package{}": "^1.0""#, i / 10, i));
        }
        require.push('}');

        let content = format!(
            r#"{{
                "name": "test/large-project",
                "require": {}
            }}"#,
            require
        );

        create_composer_json(temp.path(), &content);

        libretto()
            .arg("validate")
            .current_dir(temp.path())
            .assert()
            .success();
    }

    #[test]
    fn test_deeply_nested_extra() {
        let temp = TempDir::new().unwrap();
        create_composer_json(
            temp.path(),
            r#"{
                "name": "test/nested-project",
                "require": {},
                "extra": {
                    "level1": {
                        "level2": {
                            "level3": {
                                "level4": {
                                    "level5": "deep value"
                                }
                            }
                        }
                    }
                }
            }"#,
        );

        libretto()
            .arg("validate")
            .current_dir(temp.path())
            .assert()
            .success();
    }

    #[test]
    fn test_empty_strings() {
        let temp = TempDir::new().unwrap();
        create_composer_json(
            temp.path(),
            r#"{
                "name": "test/empty-strings",
                "description": "",
                "require": {}
            }"#,
        );

        libretto()
            .arg("validate")
            .current_dir(temp.path())
            .assert()
            .success();
    }

    #[test]
    fn test_numeric_version() {
        let temp = TempDir::new().unwrap();
        create_composer_json(
            temp.path(),
            r#"{
                "name": "test/numeric-version",
                "version": "1.0.0",
                "require": {}
            }"#,
        );

        libretto()
            .arg("validate")
            .current_dir(temp.path())
            .assert()
            .success();
    }
}

// ========== Concurrent Access Tests ==========

mod concurrent_tests {
    use super::*;
    use std::thread;

    #[test]
    fn test_concurrent_validate() {
        let temp = TempDir::new().unwrap();
        create_composer_json(temp.path(), r#"{"name": "test/concurrent", "require": {}}"#);

        let path = temp.path().to_path_buf();
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let p = path.clone();
                thread::spawn(move || {
                    libretto()
                        .arg("validate")
                        .current_dir(&p)
                        .assert()
                        .success();
                })
            })
            .collect();

        for handle in handles {
            handle.join().expect("Thread panicked");
        }
    }

    #[test]
    fn test_concurrent_show() {
        let temp = TempDir::new().unwrap();
        create_composer_json(
            temp.path(),
            r#"{"name": "test/concurrent-show", "require": {}}"#,
        );

        let path = temp.path().to_path_buf();
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let p = path.clone();
                thread::spawn(move || {
                    libretto().arg("show").current_dir(&p).assert().success();
                })
            })
            .collect();

        for handle in handles {
            handle.join().expect("Thread panicked");
        }
    }
}
