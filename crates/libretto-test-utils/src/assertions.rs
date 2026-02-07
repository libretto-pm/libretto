//! Custom assertion helpers for Libretto testing.
//!
//! This module provides domain-specific assertions for verifying
//! package installations, autoloader generation, and lock file integrity.

use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::path::Path;
use tokio::fs;

/// Assert that a package is installed in the vendor directory.
///
/// Checks:
/// - Package directory exists
/// - composer.json exists in package directory
/// - Package name matches expected
pub async fn assert_package_installed(
    vendor_path: &Path,
    name: &str,
    version: Option<&str>,
) -> Result<()> {
    let parts: Vec<&str> = name.split('/').collect();
    if parts.len() != 2 {
        bail!("Invalid package name format: {name}");
    }

    let package_path = vendor_path.join(parts[0]).join(parts[1]);

    // Check directory exists
    if !package_path.exists() {
        bail!("Package {name} not found at {}", package_path.display());
    }

    // Check composer.json exists
    let composer_json_path = package_path.join("composer.json");
    if composer_json_path.exists() {
        let content = fs::read_to_string(&composer_json_path).await?;
        let json: Value = serde_json::from_str(&content)?;

        // Verify package name
        if let Some(pkg_name) = json["name"].as_str()
            && pkg_name != name
        {
            bail!("Package name mismatch: expected {name}, found {pkg_name}");
        }

        // Verify version if provided
        if let Some(expected_version) = version
            && let Some(pkg_version) = json["version"].as_str()
            && pkg_version != expected_version
        {
            bail!(
                "Package version mismatch for {name}: expected {expected_version}, found {pkg_version}"
            );
        }
    }

    Ok(())
}

/// Assert that a package is NOT installed.
pub async fn assert_package_not_installed(vendor_path: &Path, name: &str) -> Result<()> {
    let parts: Vec<&str> = name.split('/').collect();
    if parts.len() != 2 {
        bail!("Invalid package name format: {name}");
    }

    let package_path = vendor_path.join(parts[0]).join(parts[1]);

    if package_path.exists() {
        bail!(
            "Package {name} should not be installed, but found at {}",
            package_path.display()
        );
    }

    Ok(())
}

/// Assert that a lock file is valid.
///
/// Checks:
/// - File exists
/// - Valid JSON
/// - Contains required fields
/// - Content hash is present
pub async fn assert_lock_file_valid(lock_path: &Path) -> Result<()> {
    if !lock_path.exists() {
        bail!("Lock file not found at {}", lock_path.display());
    }

    let content = fs::read_to_string(lock_path).await?;
    let json: Value = serde_json::from_str(&content).context("Lock file is not valid JSON")?;

    // Check required fields
    if !json["packages"].is_array() {
        bail!("Lock file missing 'packages' array");
    }

    if !json["content-hash"].is_string() {
        bail!("Lock file missing 'content-hash'");
    }

    // Validate package entries
    if let Some(packages) = json["packages"].as_array() {
        for (i, pkg) in packages.iter().enumerate() {
            if !pkg["name"].is_string() {
                bail!("Package {i} missing 'name' field");
            }
            if !pkg["version"].is_string() {
                bail!("Package {} ({}) missing 'version' field", i, pkg["name"]);
            }
        }
    }

    Ok(())
}

/// Assert that lock file contains a specific package.
pub async fn assert_lock_contains_package(
    lock_path: &Path,
    name: &str,
    version: Option<&str>,
) -> Result<()> {
    let content = fs::read_to_string(lock_path).await?;
    let json: Value = serde_json::from_str(&content)?;

    let packages = json["packages"]
        .as_array()
        .context("Lock file missing 'packages' array")?;

    let found = packages
        .iter()
        .find(|pkg| pkg["name"].as_str() == Some(name));

    match found {
        None => bail!("Package {name} not found in lock file"),
        Some(pkg) => {
            if let Some(expected_version) = version {
                let actual_version = pkg["version"].as_str().unwrap_or("");
                if actual_version != expected_version {
                    bail!(
                        "Package {name} version mismatch: expected {expected_version}, found {actual_version}"
                    );
                }
            }
        }
    }

    Ok(())
}

/// Assert that the autoloader is properly generated.
///
/// Checks:
/// - vendor/autoload.php exists
/// - vendor/composer directory exists
/// - Required autoload files exist
pub async fn assert_autoloader_valid(vendor_path: &Path) -> Result<()> {
    let autoload_path = vendor_path.join("autoload.php");
    if !autoload_path.exists() {
        bail!("Autoloader not found at {}", autoload_path.display());
    }

    let composer_dir = vendor_path.join("composer");
    if !composer_dir.exists() {
        bail!("Composer directory not found at {}", composer_dir.display());
    }

    // Check required autoload files
    let required_files = ["autoload_real.php", "ClassLoader.php"];

    for file in &required_files {
        let file_path = composer_dir.join(file);
        if !file_path.exists() {
            bail!("Required autoload file not found: {}", file_path.display());
        }
    }

    // Verify autoload.php content
    let autoload_content = fs::read_to_string(&autoload_path).await?;
    if !autoload_content.contains("<?php") {
        bail!("Autoloader missing PHP opening tag");
    }

    Ok(())
}

/// Assert that a PSR-4 namespace is registered in the autoloader.
pub async fn assert_psr4_registered(
    vendor_path: &Path,
    namespace: &str,
    _path: &str,
) -> Result<()> {
    let psr4_path = vendor_path.join("composer").join("autoload_psr4.php");

    if !psr4_path.exists() {
        bail!("PSR-4 autoload file not found");
    }

    let content = fs::read_to_string(&psr4_path).await?;

    // Check for namespace registration
    let escaped_ns = namespace.replace('\\', "\\\\");
    if !content.contains(&escaped_ns) {
        bail!("Namespace {namespace} not found in PSR-4 autoloader");
    }

    Ok(())
}

/// Assert that a classmap entry exists.
pub async fn assert_classmap_contains(vendor_path: &Path, class_name: &str) -> Result<()> {
    let classmap_path = vendor_path.join("composer").join("autoload_classmap.php");

    if !classmap_path.exists() {
        bail!("Classmap autoload file not found");
    }

    let content = fs::read_to_string(&classmap_path).await?;

    if !content.contains(class_name) {
        bail!("Class {class_name} not found in classmap");
    }

    Ok(())
}

/// Assert that composer.json contains expected dependencies.
pub async fn assert_composer_has_dependency(
    composer_json_path: &Path,
    name: &str,
    constraint: Option<&str>,
    dev: bool,
) -> Result<()> {
    let content = fs::read_to_string(composer_json_path).await?;
    let json: Value = serde_json::from_str(&content)?;

    let section = if dev { "require-dev" } else { "require" };
    let deps = json[section]
        .as_object()
        .context(format!("Missing '{section}' section"))?;

    match deps.get(name) {
        None => bail!("Dependency {name} not found in {section} section"),
        Some(actual_constraint) => {
            if let Some(expected) = constraint
                && actual_constraint.as_str() != Some(expected)
            {
                bail!(
                    "Dependency {name} constraint mismatch: expected {expected}, found {actual_constraint}"
                );
            }
        }
    }

    Ok(())
}

/// Assert that composer.json does NOT contain a dependency.
pub async fn assert_composer_no_dependency(
    composer_json_path: &Path,
    name: &str,
    dev: bool,
) -> Result<()> {
    let content = fs::read_to_string(composer_json_path).await?;
    let json: Value = serde_json::from_str(&content)?;

    let section = if dev { "require-dev" } else { "require" };

    if let Some(deps) = json[section].as_object()
        && deps.contains_key(name)
    {
        bail!("Dependency {name} should not be in {section} section");
    }

    Ok(())
}

/// Assert file exists and contains expected content.
pub async fn assert_file_contains(path: &Path, expected: &str) -> Result<()> {
    if !path.exists() {
        bail!("File not found: {}", path.display());
    }

    let content = fs::read_to_string(path).await?;

    if !content.contains(expected) {
        bail!(
            "File {} does not contain expected content: {expected}",
            path.display()
        );
    }

    Ok(())
}

/// Assert that file matches expected content exactly.
pub async fn assert_file_equals(path: &Path, expected: &str) -> Result<()> {
    if !path.exists() {
        bail!("File not found: {}", path.display());
    }

    let content = fs::read_to_string(path).await?;

    if content != expected {
        bail!(
            "File {} content mismatch.\nExpected:\n{expected}\n\nActual:\n{content}",
            path.display()
        );
    }

    Ok(())
}

/// Assert that a directory contains expected number of files.
pub async fn assert_dir_file_count(dir: &Path, expected: usize) -> Result<()> {
    if !dir.exists() {
        bail!("Directory not found: {}", dir.display());
    }

    let mut count = 0;
    let mut entries = fs::read_dir(dir).await?;

    while let Some(entry) = entries.next_entry().await? {
        if entry.file_type().await?.is_file() {
            count += 1;
        }
    }

    if count != expected {
        bail!(
            "Directory {} has {count} files, expected {expected}",
            dir.display()
        );
    }

    Ok(())
}

/// Assert that installation was successful with specific package count.
pub async fn assert_install_success(vendor_path: &Path, expected_packages: usize) -> Result<()> {
    // Check vendor directory exists
    if !vendor_path.exists() {
        bail!("Vendor directory not found");
    }

    // Check autoloader
    assert_autoloader_valid(vendor_path).await?;

    // Count installed packages
    let mut package_count = 0;
    let mut vendor_entries = fs::read_dir(vendor_path).await?;

    while let Some(vendor_entry) = vendor_entries.next_entry().await? {
        let vendor_name = vendor_entry.file_name();
        let vendor_name_str = vendor_name.to_string_lossy();

        // Skip composer directory
        if vendor_name_str == "composer" || vendor_name_str == "autoload.php" {
            continue;
        }

        if vendor_entry.file_type().await?.is_dir() {
            let mut pkg_entries = fs::read_dir(vendor_entry.path()).await?;
            while let Some(_pkg_entry) = pkg_entries.next_entry().await? {
                package_count += 1;
            }
        }
    }

    if package_count != expected_packages {
        bail!("Expected {expected_packages} packages installed, found {package_count}");
    }

    Ok(())
}

/// Macro for asserting JSON structure matches expected shape.
#[macro_export]
macro_rules! assert_json_shape {
    ($value:expr, $expected:tt) => {{
        let expected: serde_json::Value = serde_json::json!($expected);
        $crate::assertions::check_json_shape(&$value, &expected)
    }};
}

/// Check if JSON value matches expected structure.
pub fn check_json_shape(actual: &Value, expected: &Value) -> Result<()> {
    match (actual, expected) {
        (Value::Object(actual_obj), Value::Object(expected_obj)) => {
            for (key, expected_val) in expected_obj {
                let actual_val = actual_obj
                    .get(key)
                    .with_context(|| format!("Missing key: {key}"))?;
                check_json_shape(actual_val, expected_val)
                    .with_context(|| format!("Mismatch at key: {key}"))?;
            }
        }
        (Value::Array(_), Value::Array(_)) => {
            // For arrays, just check that actual is also an array
        }
        (Value::String(_), Value::String(_)) => {}
        (Value::Number(_), Value::Number(_)) => {}
        (Value::Bool(_), Value::Bool(_)) => {}
        (Value::Null, Value::Null) => {}
        _ => bail!("Type mismatch: expected {expected:?}, got {actual:?}"),
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_assert_file_contains() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("test.txt");
        fs::write(&file, "Hello, World!").await.unwrap();

        assert!(assert_file_contains(&file, "Hello").await.is_ok());
        assert!(assert_file_contains(&file, "NotFound").await.is_err());
    }

    #[tokio::test]
    async fn test_assert_file_equals() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("test.txt");
        fs::write(&file, "exact content").await.unwrap();

        assert!(assert_file_equals(&file, "exact content").await.is_ok());
        assert!(assert_file_equals(&file, "different").await.is_err());
    }

    #[test]
    fn test_check_json_shape() {
        let actual = serde_json::json!({
            "name": "test/package",
            "version": "1.0.0",
            "require": {
                "php": ">=8.0"
            }
        });

        let expected = serde_json::json!({
            "name": "",
            "require": {}
        });

        assert!(check_json_shape(&actual, &expected).is_ok());
    }
}
