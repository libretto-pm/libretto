//! Example: Audit a Laravel project for security vulnerabilities.

use libretto_audit::Auditor;
use libretto_core::{PackageId, Version};
use serde_json::Value;
use std::path::Path;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    let lock_path = Path::new("/tmp/laravel-test/composer.lock");

    if !lock_path.exists() {
        eprintln!("Error: composer.lock not found at {}", lock_path.display());
        eprintln!("Please run: cd /tmp && composer create-project laravel/laravel laravel-test");
        return Ok(());
    }

    println!("=== Libretto Security Audit ===\n");
    println!("Analyzing: {}\n", lock_path.display());

    // Parse composer.lock
    let lock_content = std::fs::read_to_string(lock_path)?;
    let lock: Value = serde_json::from_str(&lock_content)?;

    // Extract packages
    let mut packages: Vec<(PackageId, Version)> = Vec::new();

    if let Some(pkgs) = lock.get("packages").and_then(|p| p.as_array()) {
        for pkg in pkgs {
            if let (Some(name), Some(version)) = (
                pkg.get("name").and_then(|n| n.as_str()),
                pkg.get("version").and_then(|v| v.as_str()),
            ) {
                if let Some(package_id) = PackageId::parse(name) {
                    // Clean version string (remove 'v' prefix if present)
                    let version_str = version.trim_start_matches('v');
                    if let Ok(ver) = Version::parse(version_str) {
                        packages.push((package_id, ver));
                    }
                }
            }
        }
    }

    // Also check dev packages
    if let Some(pkgs) = lock.get("packages-dev").and_then(|p| p.as_array()) {
        for pkg in pkgs {
            if let (Some(name), Some(version)) = (
                pkg.get("name").and_then(|n| n.as_str()),
                pkg.get("version").and_then(|v| v.as_str()),
            ) {
                if let Some(package_id) = PackageId::parse(name) {
                    let version_str = version.trim_start_matches('v');
                    if let Ok(ver) = Version::parse(version_str) {
                        packages.push((package_id, ver));
                    }
                }
            }
        }
    }

    println!("Found {} packages to audit\n", packages.len());

    // Create auditor and run audit
    let auditor = Auditor::new()?;

    println!("Checking security advisories...\n");
    let report = auditor.audit(&packages).await?;

    // Display results
    if report.vulnerability_count() == 0 {
        println!("\x1b[32m✓ No security vulnerabilities found!\x1b[0m\n");
    } else {
        println!(
            "\x1b[31m✗ Found {} vulnerabilities in {} packages\x1b[0m\n",
            report.vulnerability_count(),
            report.vulnerable_package_count()
        );

        for (severity, vulns) in report.by_severity() {
            println!("{}{}:{}\x1b[0m", severity.color(), severity, vulns.len());
            for vuln in vulns {
                println!(
                    "  • {} {} - {}",
                    vuln.package, vuln.affected_versions, vuln.title
                );
                println!("    Advisory: {}", vuln.advisory_id);
                if let Some(ref fixed) = vuln.fixed_version {
                    println!("    Fixed in: {fixed}");
                }
                for url in &vuln.references {
                    println!("    Reference: {url}");
                }
                println!();
            }
        }
    }

    // Test integrity verification with a sample file
    println!("=== Integrity Verification Demo ===\n");

    let autoload_path = Path::new("/tmp/laravel-test/vendor/autoload.php");
    if autoload_path.exists() {
        println!("Computing hashes for vendor/autoload.php...");

        let hashes = libretto_audit::hash_file_all(autoload_path).await?;
        for hash in hashes {
            println!("  {:?}: {}", hash.algorithm, hash.value);
        }
        println!();
    }

    // Summary
    println!("=== Audit Summary ===");
    println!("Packages scanned: {}", packages.len());
    println!("Vulnerabilities found: {}", report.vulnerability_count());
    println!("Has critical issues: {}", report.has_critical());
    println!("Audit passed: {}", report.passes());

    // Exit code based on audit result
    if !report.passes() {
        std::process::exit(1);
    }

    Ok(())
}
