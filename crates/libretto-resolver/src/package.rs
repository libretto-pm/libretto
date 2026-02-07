//! Package types and identifiers for the resolver.
//!
//! This module defines the core package types used throughout the resolver:
//! - `PackageName`: A validated Composer package name (vendor/name format)
//! - `PackageId`: A package with a specific version
//! - `Dependency`: A dependency requirement with constraint

use crate::version::{ComposerConstraint, ComposerVersion, Stability};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smallvec::SmallVec;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::str::FromStr;
use std::sync::Arc;

/// A validated Composer package name in vendor/name format.
///
/// Package names must:
/// - Contain exactly one `/` separator
/// - Have non-empty vendor and name parts
/// - Be lowercase (automatically normalized)
#[derive(Clone)]
pub struct PackageName {
    /// The full name (vendor/name).
    full: Arc<str>,
    /// Index of the `/` separator.
    separator_idx: usize,
}

impl PackageName {
    /// Create a new package name from vendor and name parts.
    ///
    /// # Panics
    ///
    /// Panics if vendor or name is empty.
    #[must_use]
    pub fn new(vendor: &str, name: &str) -> Self {
        assert!(!vendor.is_empty(), "vendor cannot be empty");
        assert!(!name.is_empty(), "name cannot be empty");

        let vendor = vendor.to_ascii_lowercase();
        let name = name.to_ascii_lowercase();
        let full = format!("{vendor}/{name}");
        let separator_idx = vendor.len();

        Self {
            full: Arc::from(full),
            separator_idx,
        }
    }

    /// Parse a package name from a string.
    ///
    /// Returns `None` if the string is not a valid package name.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().to_ascii_lowercase();
        let separator_idx = s.find('/')?;

        // Must have non-empty parts
        if separator_idx == 0 || separator_idx == s.len() - 1 {
            return None;
        }

        // Must have exactly one /
        if s[separator_idx + 1..].contains('/') {
            return None;
        }

        Some(Self {
            full: Arc::from(s),
            separator_idx,
        })
    }

    /// Get the vendor part.
    #[must_use]
    #[inline]
    pub fn vendor(&self) -> &str {
        &self.full[..self.separator_idx]
    }

    /// Get the name part.
    #[must_use]
    #[inline]
    pub fn name(&self) -> &str {
        &self.full[self.separator_idx + 1..]
    }

    /// Get the full name.
    #[must_use]
    #[inline]
    pub fn as_str(&self) -> &str {
        &self.full
    }

    /// Check if this is a platform package (php, ext-*, lib-*).
    #[must_use]
    pub fn is_platform(&self) -> bool {
        libretto_core::is_platform_package_name(self.as_str())
    }
}

impl fmt::Debug for PackageName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("PackageName").field(&self.full).finish()
    }
}

impl fmt::Display for PackageName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.full)
    }
}

impl PartialEq for PackageName {
    fn eq(&self, other: &Self) -> bool {
        self.full == other.full
    }
}

impl Eq for PackageName {}

impl Hash for PackageName {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.full.hash(state);
    }
}

impl PartialOrd for PackageName {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PackageName {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.full.cmp(&other.full)
    }
}

impl FromStr for PackageName {
    type Err = PackageNameError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s).ok_or_else(|| PackageNameError(s.to_string()))
    }
}

impl Serialize for PackageName {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.full)
    }
}

impl<'de> Deserialize<'de> for PackageName {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::parse(&s)
            .ok_or_else(|| serde::de::Error::custom(format!("invalid package name: {s}")))
    }
}

/// Error when parsing an invalid package name.
#[derive(Debug, Clone, thiserror::Error)]
#[error("invalid package name: {0}")]
pub struct PackageNameError(pub String);

/// A dependency requirement.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Dependency {
    /// The package name.
    pub name: PackageName,
    /// Version constraint.
    pub constraint: ComposerConstraint,
}

impl Dependency {
    /// Create a new dependency.
    #[must_use]
    pub const fn new(name: PackageName, constraint: ComposerConstraint) -> Self {
        Self { name, constraint }
    }
}

impl fmt::Display for Dependency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.name, self.constraint)
    }
}

/// Package metadata for a specific version.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageVersion {
    /// Package name.
    pub name: PackageName,
    /// Version.
    pub version: ComposerVersion,
    /// Dependencies (require).
    pub dependencies: SmallVec<[Dependency; 8]>,
    /// Dev dependencies (require-dev).
    pub dev_dependencies: SmallVec<[Dependency; 4]>,
    /// Packages this version replaces.
    pub replaces: SmallVec<[Dependency; 2]>,
    /// Virtual packages this version provides.
    pub provides: SmallVec<[Dependency; 2]>,
    /// Packages this conflicts with.
    pub conflicts: SmallVec<[Dependency; 2]>,
    /// Suggested packages.
    pub suggests: SmallVec<[Dependency; 2]>,
    /// Stability of this version.
    pub stability: Stability,
    /// Distribution URL.
    pub dist_url: Option<Arc<str>>,
    /// Distribution type (zip, tar, etc.).
    pub dist_type: Option<Arc<str>>,
    /// Distribution checksum.
    pub dist_shasum: Option<Arc<str>>,
    /// Source URL (git, etc.).
    pub source_url: Option<Arc<str>>,
    /// Source type.
    pub source_type: Option<Arc<str>>,
    /// Source reference (commit, tag).
    pub source_reference: Option<Arc<str>>,
    // Full metadata for lock file
    /// Package type (library, project, etc.).
    pub package_type: Option<Arc<str>>,
    /// Description.
    pub description: Option<Arc<str>>,
    /// Homepage URL.
    pub homepage: Option<Arc<str>>,
    /// Licenses.
    pub license: Option<Vec<String>>,
    /// Authors (JSON value).
    pub authors: Option<sonic_rs::Value>,
    /// Keywords.
    pub keywords: Option<Vec<String>>,
    /// Release time.
    pub time: Option<Arc<str>>,
    /// Autoload configuration (JSON value).
    pub autoload: Option<sonic_rs::Value>,
    /// Autoload-dev configuration (JSON value).
    pub autoload_dev: Option<sonic_rs::Value>,
    /// Extra metadata (JSON value).
    pub extra: Option<sonic_rs::Value>,
    /// Support links (JSON value).
    pub support: Option<sonic_rs::Value>,
    /// Funding links (JSON value).
    pub funding: Option<sonic_rs::Value>,
    /// Notification URL.
    pub notification_url: Option<Arc<str>>,
    /// Binary files.
    pub bin: Option<Vec<String>>,
}

impl PackageVersion {
    /// Create a new package version with minimal data.
    #[must_use]
    pub fn new(name: PackageName, version: ComposerVersion) -> Self {
        let stability = version.stability;
        Self {
            name,
            version,
            dependencies: SmallVec::new(),
            dev_dependencies: SmallVec::new(),
            replaces: SmallVec::new(),
            provides: SmallVec::new(),
            conflicts: SmallVec::new(),
            suggests: SmallVec::new(),
            stability,
            dist_url: None,
            dist_type: None,
            dist_shasum: None,
            source_url: None,
            source_type: None,
            source_reference: None,
            package_type: None,
            description: None,
            homepage: None,
            license: None,
            authors: None,
            keywords: None,
            time: None,
            autoload: None,
            autoload_dev: None,
            extra: None,
            support: None,
            funding: None,
            notification_url: None,
            bin: None,
        }
    }

    /// Add a dependency.
    pub fn add_dependency(&mut self, dep: Dependency) {
        self.dependencies.push(dep);
    }

    /// Add a dev dependency.
    pub fn add_dev_dependency(&mut self, dep: Dependency) {
        self.dev_dependencies.push(dep);
    }

    /// Add a replace declaration.
    pub fn add_replace(&mut self, dep: Dependency) {
        self.replaces.push(dep);
    }

    /// Add a provide declaration.
    pub fn add_provide(&mut self, dep: Dependency) {
        self.provides.push(dep);
    }

    /// Add a conflict declaration.
    pub fn add_conflict(&mut self, dep: Dependency) {
        self.conflicts.push(dep);
    }
}

/// Package entry for the package index.
#[derive(Debug, Clone)]
pub struct PackageEntry {
    /// Package name.
    pub name: PackageName,
    /// Available versions, sorted by version descending.
    pub versions: Vec<PackageVersion>,
}

impl PackageEntry {
    /// Create a new package entry.
    #[must_use]
    pub const fn new(name: PackageName) -> Self {
        Self {
            name,
            versions: Vec::new(),
        }
    }

    /// Add a version to this package.
    pub fn add_version(&mut self, version: PackageVersion) {
        self.versions.push(version);
    }

    /// Sort versions in descending order (highest first).
    pub fn sort_versions(&mut self) {
        self.versions.sort_by(|a, b| b.version.cmp(&a.version));
    }

    /// Get versions matching a constraint.
    #[must_use]
    pub fn matching_versions(&self, constraint: &ComposerConstraint) -> Vec<&PackageVersion> {
        self.versions
            .iter()
            .filter(|v| constraint.matches(&v.version))
            .collect()
    }

    /// Get the highest version matching a constraint.
    #[must_use]
    pub fn highest_matching(&self, constraint: &ComposerConstraint) -> Option<&PackageVersion> {
        self.versions
            .iter()
            .find(|v| constraint.matches(&v.version))
    }

    /// Get the lowest version matching a constraint.
    #[must_use]
    pub fn lowest_matching(&self, constraint: &ComposerConstraint) -> Option<&PackageVersion> {
        self.versions
            .iter()
            .rev()
            .find(|v| constraint.matches(&v.version))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod package_name {
        use super::*;

        #[test]
        fn parse_valid() {
            let name = PackageName::parse("symfony/console").unwrap();
            assert_eq!(name.vendor(), "symfony");
            assert_eq!(name.name(), "console");
            assert_eq!(name.as_str(), "symfony/console");
        }

        #[test]
        fn parse_normalizes_case() {
            let name = PackageName::parse("Symfony/Console").unwrap();
            assert_eq!(name.as_str(), "symfony/console");
        }

        #[test]
        fn parse_invalid() {
            assert!(PackageName::parse("invalid").is_none());
            assert!(PackageName::parse("/name").is_none());
            assert!(PackageName::parse("vendor/").is_none());
            assert!(PackageName::parse("vendor/name/extra").is_none());
        }

        #[test]
        fn new_creates_valid() {
            let name = PackageName::new("vendor", "name");
            assert_eq!(name.as_str(), "vendor/name");
        }

        #[test]
        #[should_panic(expected = "vendor cannot be empty")]
        fn new_panics_empty_vendor() {
            let _ = PackageName::new("", "name");
        }

        #[test]
        fn is_platform() {
            // "php" alone is not vendor/name format, so parse returns None.
            // Platform packages like "php" are handled at the dependency filtering level.
            assert!(PackageName::parse("php").is_none());
            assert!(
                !PackageName::parse("php-open-source-saver/jwt-auth")
                    .unwrap()
                    .is_platform()
            );
        }
    }

    mod dependency {
        use super::*;

        #[test]
        fn create_dependency() {
            let dep = Dependency::new(
                PackageName::new("symfony", "console"),
                ComposerConstraint::parse("^5.0").unwrap(),
            );
            assert_eq!(dep.name.as_str(), "symfony/console");
        }
    }

    mod package_version {
        use super::*;

        #[test]
        fn create_and_add_deps() {
            let mut pkg = PackageVersion::new(
                PackageName::new("test", "pkg"),
                ComposerVersion::parse("1.0.0").unwrap(),
            );

            pkg.add_dependency(Dependency::new(
                PackageName::new("dep", "one"),
                ComposerConstraint::parse("^1.0").unwrap(),
            ));

            assert_eq!(pkg.dependencies.len(), 1);
        }
    }

    mod package_entry {
        use super::*;

        #[test]
        fn sort_versions() {
            let mut entry = PackageEntry::new(PackageName::new("test", "pkg"));

            entry.add_version(PackageVersion::new(
                PackageName::new("test", "pkg"),
                ComposerVersion::parse("1.0.0").unwrap(),
            ));
            entry.add_version(PackageVersion::new(
                PackageName::new("test", "pkg"),
                ComposerVersion::parse("2.0.0").unwrap(),
            ));
            entry.add_version(PackageVersion::new(
                PackageName::new("test", "pkg"),
                ComposerVersion::parse("1.5.0").unwrap(),
            ));

            entry.sort_versions();

            assert_eq!(entry.versions[0].version.major, 2);
            assert_eq!(entry.versions[1].version.major, 1);
            assert_eq!(entry.versions[1].version.minor, 5);
            assert_eq!(entry.versions[2].version.major, 1);
            assert_eq!(entry.versions[2].version.minor, 0);
        }

        #[test]
        fn matching_versions() {
            let mut entry = PackageEntry::new(PackageName::new("test", "pkg"));

            entry.add_version(PackageVersion::new(
                PackageName::new("test", "pkg"),
                ComposerVersion::parse("1.0.0").unwrap(),
            ));
            entry.add_version(PackageVersion::new(
                PackageName::new("test", "pkg"),
                ComposerVersion::parse("2.0.0").unwrap(),
            ));

            let constraint = ComposerConstraint::parse("^1.0").unwrap();
            let matching = entry.matching_versions(&constraint);

            assert_eq!(matching.len(), 1);
            assert_eq!(matching[0].version.major, 1);
        }
    }
}
