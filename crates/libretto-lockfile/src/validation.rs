//! Lock file validation and drift detection.
//!
//! Provides:
//! - Structural validation
//! - Content hash verification
//! - Manual edit detection
//! - Out-of-date warning

use crate::hash::{verify_hash, ContentHasher};
use crate::types::{ComposerLock, LockedPackage};
use ahash::AHashSet;
use std::collections::BTreeMap;

/// Validation result with warnings.
#[derive(Debug, Clone, Default)]
pub struct ValidationResult {
    /// Whether validation passed.
    pub valid: bool,
    /// Critical errors that prevent use.
    pub errors: Vec<ValidationError>,
    /// Warnings that don't prevent use.
    pub warnings: Vec<ValidationWarning>,
}

impl ValidationResult {
    /// Create a valid result.
    #[must_use]
    pub fn valid() -> Self {
        Self {
            valid: true,
            errors: Vec::new(),
            warnings: Vec::new(),
        }
    }

    /// Create an invalid result with an error.
    #[must_use]
    pub fn error(error: ValidationError) -> Self {
        Self {
            valid: false,
            errors: vec![error],
            warnings: Vec::new(),
        }
    }

    /// Add a warning.
    pub fn warn(&mut self, warning: ValidationWarning) {
        self.warnings.push(warning);
    }

    /// Add an error.
    pub fn add_error(&mut self, error: ValidationError) {
        self.valid = false;
        self.errors.push(error);
    }

    /// Check if there are any issues (errors or warnings).
    #[must_use]
    pub fn has_issues(&self) -> bool {
        !self.errors.is_empty() || !self.warnings.is_empty()
    }

    /// Merge another result into this one.
    pub fn merge(&mut self, other: ValidationResult) {
        if !other.valid {
            self.valid = false;
        }
        self.errors.extend(other.errors);
        self.warnings.extend(other.warnings);
    }
}

/// Validation error types.
#[derive(Debug, Clone)]
pub enum ValidationError {
    /// Missing required field.
    MissingField(String),
    /// Duplicate package.
    DuplicatePackage(String),
    /// Invalid package data.
    InvalidPackage { name: String, reason: String },
    /// Invalid JSON structure.
    InvalidStructure(String),
    /// Circular dependency.
    CircularDependency(Vec<String>),
    /// Content hash mismatch.
    ContentHashMismatch { expected: String, actual: String },
    /// Missing dependency.
    MissingDependency { package: String, dependency: String },
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingField(field) => write!(f, "Missing required field: {}", field),
            Self::DuplicatePackage(name) => write!(f, "Duplicate package: {}", name),
            Self::InvalidPackage { name, reason } => {
                write!(f, "Invalid package '{}': {}", name, reason)
            }
            Self::InvalidStructure(reason) => write!(f, "Invalid structure: {}", reason),
            Self::CircularDependency(cycle) => {
                write!(f, "Circular dependency: {}", cycle.join(" -> "))
            }
            Self::ContentHashMismatch { expected, actual } => {
                write!(
                    f,
                    "Content hash mismatch: expected {}, got {}",
                    expected, actual
                )
            }
            Self::MissingDependency {
                package,
                dependency,
            } => {
                write!(
                    f,
                    "Package '{}' requires '{}' which is not in lock file",
                    package, dependency
                )
            }
        }
    }
}

/// Validation warning types.
#[derive(Debug, Clone)]
pub enum ValidationWarning {
    /// Lock file may be out of date.
    OutOfDate(String),
    /// Manual edits detected.
    ManualEdit(String),
    /// Missing optional field.
    MissingOptionalField { package: String, field: String },
    /// Unusual version format.
    UnusualVersion { package: String, version: String },
    /// Development dependency in production.
    DevInProduction(String),
    /// Deprecated package.
    DeprecatedPackage(String),
    /// Missing source/dist.
    MissingInstallSource(String),
}

impl std::fmt::Display for ValidationWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OutOfDate(msg) => write!(f, "Lock file may be out of date: {}", msg),
            Self::ManualEdit(msg) => write!(f, "Manual edit detected: {}", msg),
            Self::MissingOptionalField { package, field } => {
                write!(f, "Package '{}' missing optional field: {}", package, field)
            }
            Self::UnusualVersion { package, version } => {
                write!(
                    f,
                    "Package '{}' has unusual version format: {}",
                    package, version
                )
            }
            Self::DevInProduction(name) => {
                write!(
                    f,
                    "Dev package '{}' may be in production dependencies",
                    name
                )
            }
            Self::DeprecatedPackage(name) => write!(f, "Package '{}' is deprecated", name),
            Self::MissingInstallSource(name) => {
                write!(f, "Package '{}' has no source or dist", name)
            }
        }
    }
}

/// Lock file validator.
#[derive(Debug, Default)]
pub struct Validator {
    /// Check for circular dependencies.
    check_circular: bool,
    /// Check for missing dependencies.
    check_dependencies: bool,
    /// Warn about missing optional fields.
    warn_missing_optional: bool,
    /// Check version formats.
    check_versions: bool,
}

impl Validator {
    /// Create a new validator with default settings.
    #[must_use]
    pub fn new() -> Self {
        Self {
            check_circular: true,
            check_dependencies: true,
            warn_missing_optional: false,
            check_versions: true,
        }
    }

    /// Create a strict validator that checks everything.
    #[must_use]
    pub fn strict() -> Self {
        Self {
            check_circular: true,
            check_dependencies: true,
            warn_missing_optional: true,
            check_versions: true,
        }
    }

    /// Enable/disable circular dependency check.
    pub fn check_circular(&mut self, enable: bool) -> &mut Self {
        self.check_circular = enable;
        self
    }

    /// Enable/disable dependency completeness check.
    pub fn check_dependencies(&mut self, enable: bool) -> &mut Self {
        self.check_dependencies = enable;
        self
    }

    /// Validate a lock file.
    #[must_use]
    pub fn validate(&self, lock: &ComposerLock) -> ValidationResult {
        let mut result = ValidationResult::valid();

        // Check required fields
        if lock.content_hash.is_empty() {
            result.add_error(ValidationError::MissingField("content-hash".to_string()));
        }

        // Check for duplicate packages
        self.check_duplicates(&mut result, &lock.packages, false);
        self.check_duplicates(&mut result, &lock.packages_dev, true);

        // Validate each package
        self.validate_packages(&mut result, &lock.packages, false);
        self.validate_packages(&mut result, &lock.packages_dev, true);

        // Check dependencies if enabled
        if self.check_dependencies {
            self.check_dependency_completeness(&mut result, lock);
        }

        // Check for circular dependencies if enabled
        if self.check_circular {
            self.check_circular_deps(&mut result, lock);
        }

        result
    }

    /// Validate against composer.json dependencies.
    pub fn validate_against_manifest(
        &self,
        lock: &ComposerLock,
        require: &BTreeMap<String, String>,
        require_dev: &BTreeMap<String, String>,
        minimum_stability: Option<&str>,
        prefer_stable: Option<bool>,
        platform: &BTreeMap<String, String>,
    ) -> ValidationResult {
        let mut result = self.validate(lock);

        // Compute expected content hash
        let expected_hash = ContentHasher::compute_content_hash(
            require,
            require_dev,
            minimum_stability,
            prefer_stable,
            if lock.prefer_lowest { Some(true) } else { None },
            platform,
            &BTreeMap::new(),
        );

        // Check content hash
        if !verify_hash(&lock.content_hash, &expected_hash) {
            result.add_error(ValidationError::ContentHashMismatch {
                expected: expected_hash,
                actual: lock.content_hash.clone(),
            });
        }

        // Check that all required packages are present
        let locked_names: AHashSet<&str> = lock.packages.iter().map(|p| p.name.as_str()).collect();
        let locked_dev_names: AHashSet<&str> =
            lock.packages_dev.iter().map(|p| p.name.as_str()).collect();

        for name in require.keys() {
            // Skip platform packages
            if is_platform_package(name) {
                continue;
            }
            if !locked_names.contains(name.as_str()) {
                result.add_error(ValidationError::MissingDependency {
                    package: "(root)".to_string(),
                    dependency: name.clone(),
                });
            }
        }

        for name in require_dev.keys() {
            if is_platform_package(name) {
                continue;
            }
            if !locked_dev_names.contains(name.as_str()) && !locked_names.contains(name.as_str()) {
                result.add_error(ValidationError::MissingDependency {
                    package: "(root-dev)".to_string(),
                    dependency: name.clone(),
                });
            }
        }

        result
    }

    fn check_duplicates(
        &self,
        result: &mut ValidationResult,
        packages: &[LockedPackage],
        _is_dev: bool,
    ) {
        let mut seen: AHashSet<&str> = AHashSet::with_capacity(packages.len());
        for pkg in packages {
            let name_lower = pkg.name.to_lowercase();
            // Check case-insensitive duplicates
            if seen.iter().any(|s| s.eq_ignore_ascii_case(&name_lower)) {
                result.add_error(ValidationError::DuplicatePackage(pkg.name.clone()));
            }
            seen.insert(&pkg.name);
        }
    }

    fn validate_packages(
        &self,
        result: &mut ValidationResult,
        packages: &[LockedPackage],
        _is_dev: bool,
    ) {
        for pkg in packages {
            // Check name format
            if !pkg.name.contains('/') {
                result.add_error(ValidationError::InvalidPackage {
                    name: pkg.name.clone(),
                    reason: "Invalid package name format (expected vendor/name)".to_string(),
                });
            }

            // Check version
            if pkg.version.is_empty() {
                result.add_error(ValidationError::InvalidPackage {
                    name: pkg.name.clone(),
                    reason: "Empty version".to_string(),
                });
            }

            // Check version format
            if self.check_versions && !is_valid_version(&pkg.version) {
                result.warn(ValidationWarning::UnusualVersion {
                    package: pkg.name.clone(),
                    version: pkg.version.clone(),
                });
            }

            // Warn about missing source/dist
            if pkg.source.is_none() && pkg.dist.is_none() {
                result.warn(ValidationWarning::MissingInstallSource(pkg.name.clone()));
            }

            // Check for deprecated packages
            if pkg.abandoned.is_some() {
                result.warn(ValidationWarning::DeprecatedPackage(pkg.name.clone()));
            }

            // Warn about missing optional fields if enabled
            if self.warn_missing_optional {
                if pkg.description.is_none() {
                    result.warn(ValidationWarning::MissingOptionalField {
                        package: pkg.name.clone(),
                        field: "description".to_string(),
                    });
                }
                if pkg.license.is_empty() {
                    result.warn(ValidationWarning::MissingOptionalField {
                        package: pkg.name.clone(),
                        field: "license".to_string(),
                    });
                }
            }
        }
    }

    fn check_dependency_completeness(&self, result: &mut ValidationResult, lock: &ComposerLock) {
        // Build index of all locked packages
        let all_packages: AHashSet<&str> = lock
            .packages
            .iter()
            .chain(lock.packages_dev.iter())
            .map(|p| p.name.as_str())
            .collect();

        // Check each package's dependencies
        for pkg in lock.packages.iter().chain(lock.packages_dev.iter()) {
            for dep_name in pkg.require.keys() {
                // Skip platform packages
                if is_platform_package(dep_name) {
                    continue;
                }
                // Check if dependency is locked
                if !all_packages.contains(dep_name.as_str()) {
                    result.add_error(ValidationError::MissingDependency {
                        package: pkg.name.clone(),
                        dependency: dep_name.clone(),
                    });
                }
            }
        }
    }

    fn check_circular_deps(&self, result: &mut ValidationResult, lock: &ComposerLock) {
        // Build adjacency list
        let mut graph: AHashSet<(&str, &str)> = AHashSet::new();
        for pkg in lock.packages.iter().chain(lock.packages_dev.iter()) {
            for dep_name in pkg.require.keys() {
                if !is_platform_package(dep_name) {
                    graph.insert((pkg.name.as_str(), dep_name.as_str()));
                }
            }
        }

        // Simple cycle detection using DFS
        let all_names: Vec<&str> = lock
            .packages
            .iter()
            .chain(lock.packages_dev.iter())
            .map(|p| p.name.as_str())
            .collect();

        for start in &all_names {
            let mut visited: AHashSet<&str> = AHashSet::new();
            let mut path: Vec<&str> = Vec::new();

            if let Some(cycle) = find_cycle(&graph, start, &mut visited, &mut path) {
                result.add_error(ValidationError::CircularDependency(
                    cycle.into_iter().map(String::from).collect(),
                ));
                return; // Only report first cycle
            }
        }
    }
}

/// Find a cycle starting from the given node.
fn find_cycle<'a>(
    graph: &AHashSet<(&'a str, &'a str)>,
    current: &'a str,
    visited: &mut AHashSet<&'a str>,
    path: &mut Vec<&'a str>,
) -> Option<Vec<&'a str>> {
    if path.contains(&current) {
        // Found cycle
        let cycle_start = path.iter().position(|&n| n == current).unwrap();
        let mut cycle: Vec<&str> = path[cycle_start..].to_vec();
        cycle.push(current);
        return Some(cycle);
    }

    if visited.contains(current) {
        return None;
    }

    visited.insert(current);
    path.push(current);

    for &(from, to) in graph {
        if from == current {
            if let Some(cycle) = find_cycle(graph, to, visited, path) {
                return Some(cycle);
            }
        }
    }

    path.pop();
    None
}

/// Detect drift between lock file and composer.json.
#[derive(Debug)]
pub struct DriftDetector;

impl DriftDetector {
    /// Check if lock file is out of date.
    #[must_use]
    pub fn check_drift(
        lock: &ComposerLock,
        require: &BTreeMap<String, String>,
        require_dev: &BTreeMap<String, String>,
        minimum_stability: Option<&str>,
        prefer_stable: Option<bool>,
        platform: &BTreeMap<String, String>,
    ) -> DriftResult {
        let expected_hash = ContentHasher::compute_content_hash(
            require,
            require_dev,
            minimum_stability,
            prefer_stable,
            if lock.prefer_lowest { Some(true) } else { None },
            platform,
            &BTreeMap::new(),
        );

        let hash_matches = verify_hash(&lock.content_hash, &expected_hash);

        DriftResult {
            is_current: hash_matches,
            expected_hash,
            actual_hash: lock.content_hash.clone(),
            added_deps: find_missing_in_lock(require, &lock.packages),
            removed_deps: find_extra_in_lock(require, &lock.packages),
            added_dev_deps: find_missing_in_lock(require_dev, &lock.packages_dev),
            removed_dev_deps: find_extra_in_lock(require_dev, &lock.packages_dev),
        }
    }
}

/// Result of drift detection.
#[derive(Debug)]
pub struct DriftResult {
    /// Whether lock file is current.
    pub is_current: bool,
    /// Expected content hash.
    pub expected_hash: String,
    /// Actual content hash in lock file.
    pub actual_hash: String,
    /// Dependencies in composer.json but not locked.
    pub added_deps: Vec<String>,
    /// Dependencies locked but not in composer.json.
    pub removed_deps: Vec<String>,
    /// Dev dependencies in composer.json but not locked.
    pub added_dev_deps: Vec<String>,
    /// Dev dependencies locked but not in composer.json.
    pub removed_dev_deps: Vec<String>,
}

impl DriftResult {
    /// Check if there are any changes.
    #[must_use]
    pub fn has_changes(&self) -> bool {
        !self.is_current
            || !self.added_deps.is_empty()
            || !self.removed_deps.is_empty()
            || !self.added_dev_deps.is_empty()
            || !self.removed_dev_deps.is_empty()
    }

    /// Generate human-readable summary.
    #[must_use]
    pub fn summary(&self) -> String {
        if self.is_current && !self.has_changes() {
            return "Lock file is up to date".to_string();
        }

        let mut parts = Vec::new();

        if !self.is_current {
            parts.push("Content hash mismatch".to_string());
        }
        if !self.added_deps.is_empty() {
            parts.push(format!(
                "{} new dependencies: {}",
                self.added_deps.len(),
                self.added_deps.join(", ")
            ));
        }
        if !self.removed_deps.is_empty() {
            parts.push(format!(
                "{} removed dependencies: {}",
                self.removed_deps.len(),
                self.removed_deps.join(", ")
            ));
        }
        if !self.added_dev_deps.is_empty() {
            parts.push(format!(
                "{} new dev dependencies: {}",
                self.added_dev_deps.len(),
                self.added_dev_deps.join(", ")
            ));
        }
        if !self.removed_dev_deps.is_empty() {
            parts.push(format!(
                "{} removed dev dependencies: {}",
                self.removed_dev_deps.len(),
                self.removed_dev_deps.join(", ")
            ));
        }

        parts.join("; ")
    }
}

/// Find dependencies in composer.json but not in lock file.
fn find_missing_in_lock(
    require: &BTreeMap<String, String>,
    packages: &[LockedPackage],
) -> Vec<String> {
    let locked: AHashSet<&str> = packages.iter().map(|p| p.name.as_str()).collect();
    require
        .keys()
        .filter(|name| !is_platform_package(name) && !locked.contains(name.as_str()))
        .cloned()
        .collect()
}

/// Find packages in lock file but not in composer.json.
fn find_extra_in_lock(
    require: &BTreeMap<String, String>,
    packages: &[LockedPackage],
) -> Vec<String> {
    // Note: This only checks direct dependencies, not transitive
    packages
        .iter()
        .filter(|pkg| !require.contains_key(&pkg.name))
        .map(|pkg| pkg.name.clone())
        .collect()
}

/// Detect manual edits in lock file.
#[derive(Debug)]
pub struct ManualEditDetector;

impl ManualEditDetector {
    /// Check for signs of manual editing.
    #[must_use]
    pub fn detect(lock: &ComposerLock) -> Vec<String> {
        let mut signs = Vec::new();

        // Check readme modification
        let expected_readme = [
            "This file locks the dependencies of your project to a known state",
            "Read more about it at https://getcomposer.org/doc/01-basic-usage.md#installing-dependencies",
            "This file is @generated automatically",
        ];

        if lock.readme.len() != 3 {
            signs.push("Readme section has unexpected number of lines".to_string());
        } else {
            for (i, expected) in expected_readme.iter().enumerate() {
                if lock.readme.get(i).map(String::as_str) != Some(*expected) {
                    signs.push(format!("Readme line {} differs from expected", i + 1));
                    break;
                }
            }
        }

        // Check for unsorted packages
        let mut sorted_packages = lock.packages.clone();
        sorted_packages.sort();
        if lock.packages != sorted_packages {
            signs.push("Packages are not sorted alphabetically".to_string());
        }

        let mut sorted_dev = lock.packages_dev.clone();
        sorted_dev.sort();
        if lock.packages_dev != sorted_dev {
            signs.push("Dev packages are not sorted alphabetically".to_string());
        }

        // Check for unusual stability values
        for (name, &flag) in &lock.stability_flags {
            if ![0, 5, 10, 15, 20].contains(&flag) {
                signs.push(format!(
                    "Package '{}' has unusual stability flag: {}",
                    name, flag
                ));
            }
        }

        signs
    }
}

/// Check if a package is a platform package.
fn is_platform_package(name: &str) -> bool {
    name == "php"
        || name.starts_with("php-")
        || name.starts_with("ext-")
        || name.starts_with("lib-")
        || name == "composer"
        || name == "composer-plugin-api"
        || name == "composer-runtime-api"
}

/// Check if version format is valid.
fn is_valid_version(version: &str) -> bool {
    // Accept common formats
    let v = version.trim_start_matches('v');

    // Semver-like
    if v.split('.')
        .all(|part| part.chars().take_while(|c| c.is_ascii_digit()).count() > 0)
    {
        return true;
    }

    // Dev versions
    if v.starts_with("dev-") {
        return true;
    }

    // Branch aliases
    if v.contains("x-dev") {
        return true;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validator_basic() {
        let lock = ComposerLock::default();
        let result = Validator::new().validate(&lock);
        // Empty content hash should be an error
        assert!(!result.valid);
    }

    #[test]
    fn test_validator_with_packages() {
        let mut lock = ComposerLock::default();
        lock.content_hash = "abc123".to_string();
        lock.packages
            .push(LockedPackage::new("vendor/pkg", "1.0.0"));

        let result = Validator::new().validate(&lock);
        assert!(result.valid);
    }

    #[test]
    fn test_duplicate_detection() {
        let mut lock = ComposerLock::default();
        lock.content_hash = "abc123".to_string();
        lock.packages
            .push(LockedPackage::new("vendor/pkg", "1.0.0"));
        lock.packages
            .push(LockedPackage::new("vendor/pkg", "2.0.0"));

        let result = Validator::new().validate(&lock);
        assert!(!result.valid);
        assert!(result
            .errors
            .iter()
            .any(|e| matches!(e, ValidationError::DuplicatePackage(_))));
    }

    #[test]
    fn test_drift_detection() {
        let mut lock = ComposerLock::default();
        lock.content_hash = "old_hash".to_string();

        let mut require = BTreeMap::new();
        require.insert("vendor/pkg".to_string(), "^1.0".to_string());

        let result = DriftDetector::check_drift(
            &lock,
            &require,
            &BTreeMap::new(),
            None,
            None,
            &BTreeMap::new(),
        );

        assert!(!result.is_current);
        assert!(!result.added_deps.is_empty());
    }

    #[test]
    fn test_manual_edit_detector() {
        let mut lock = ComposerLock::default();
        lock.readme = vec!["Modified readme".to_string()];

        let signs = ManualEditDetector::detect(&lock);
        assert!(!signs.is_empty());
    }

    #[test]
    fn test_is_platform_package() {
        assert!(is_platform_package("php"));
        assert!(is_platform_package("ext-json"));
        assert!(is_platform_package("lib-curl"));
        assert!(!is_platform_package("vendor/pkg"));
    }

    #[test]
    fn test_is_valid_version() {
        assert!(is_valid_version("1.0.0"));
        assert!(is_valid_version("v1.0.0"));
        assert!(is_valid_version("1.0.0-beta1"));
        assert!(is_valid_version("dev-master"));
        assert!(is_valid_version("1.0.x-dev"));
    }
}
