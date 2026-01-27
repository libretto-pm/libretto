//! Lock file data structures - Composer compatible.
//!
//! This module defines all data structures needed for composer.lock files,
//! with full compatibility with Composer's JSON schema.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Complete composer.lock file structure.
///
/// Fields are ordered for deterministic JSON output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposerLock {
    /// Usage warning.
    #[serde(rename = "_readme")]
    pub readme: Vec<String>,

    /// Hash of composer.json dependencies for drift detection.
    #[serde(rename = "content-hash")]
    pub content_hash: String,

    /// Installed production packages.
    pub packages: Vec<LockedPackage>,

    /// Installed development packages.
    #[serde(rename = "packages-dev")]
    pub packages_dev: Vec<LockedPackage>,

    /// Package aliases.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<PackageAlias>,

    /// Minimum stability setting.
    #[serde(rename = "minimum-stability")]
    pub minimum_stability: String,

    /// Per-package stability flags.
    #[serde(rename = "stability-flags")]
    pub stability_flags: BTreeMap<String, u8>,

    /// Prefer stable versions.
    #[serde(rename = "prefer-stable")]
    pub prefer_stable: bool,

    /// Prefer lowest versions.
    #[serde(rename = "prefer-lowest")]
    pub prefer_lowest: bool,

    /// PHP and extension versions.
    pub platform: BTreeMap<String, String>,

    /// Dev platform requirements.
    #[serde(rename = "platform-dev")]
    pub platform_dev: BTreeMap<String, String>,

    /// Plugin API version.
    #[serde(rename = "plugin-api-version")]
    pub plugin_api_version: String,
}

impl Default for ComposerLock {
    fn default() -> Self {
        Self {
            readme: vec![
                "This file locks the dependencies of your project to a known state".to_string(),
                "Read more about it at https://getcomposer.org/doc/01-basic-usage.md#installing-dependencies".to_string(),
                "This file is @generated automatically".to_string(),
            ],
            content_hash: String::new(),
            packages: Vec::new(),
            packages_dev: Vec::new(),
            aliases: Vec::new(),
            minimum_stability: "stable".to_string(),
            stability_flags: BTreeMap::new(),
            prefer_stable: false,
            prefer_lowest: false,
            platform: BTreeMap::new(),
            platform_dev: BTreeMap::new(),
            plugin_api_version: "2.6.0".to_string(),
        }
    }
}

/// A locked package with full metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LockedPackage {
    /// Package name (vendor/name).
    pub name: String,

    /// Exact locked version.
    pub version: String,

    /// Source repository information.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<PackageSourceInfo>,

    /// Distribution archive information.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dist: Option<PackageDistInfo>,

    /// Production dependencies.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub require: BTreeMap<String, String>,

    /// Development dependencies.
    #[serde(
        rename = "require-dev",
        default,
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub require_dev: BTreeMap<String, String>,

    /// Package type.
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub package_type: Option<String>,

    /// Extra metadata.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, sonic_rs::Value>,

    /// Autoload configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autoload: Option<AutoloadConfig>,

    /// Notification URL for downloads.
    #[serde(
        rename = "notification-url",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub notification_url: Option<String>,

    /// License(s).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub license: Vec<String>,

    /// Authors.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authors: Vec<PackageAuthor>,

    /// Package description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Homepage URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,

    /// Keywords.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keywords: Vec<String>,

    /// Support information.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub support: Option<SupportInfo>,

    /// Funding information.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub funding: Vec<FundingInfo>,

    /// Installation timestamp (ISO 8601).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time: Option<String>,

    /// Replaced packages.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub replace: BTreeMap<String, String>,

    /// Provided packages.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub provide: BTreeMap<String, String>,

    /// Conflicting packages.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub conflict: BTreeMap<String, String>,

    /// Suggested packages.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub suggest: BTreeMap<String, String>,

    /// Binary files.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bin: Vec<String>,

    /// Dev autoload configuration.
    #[serde(
        rename = "autoload-dev",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub autoload_dev: Option<AutoloadConfig>,

    /// Default branch flag.
    #[serde(
        rename = "default-branch",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub default_branch: Option<bool>,

    /// Archive configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive: Option<ArchiveConfig>,

    /// Abandoned notice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abandoned: Option<AbandonedValue>,

    /// Installation source preference.
    #[serde(
        rename = "installation-source",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub installation_source: Option<String>,
}

impl LockedPackage {
    /// Create a new locked package with required fields.
    #[must_use]
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            source: None,
            dist: None,
            require: BTreeMap::new(),
            require_dev: BTreeMap::new(),
            package_type: None,
            extra: BTreeMap::new(),
            autoload: None,
            notification_url: None,
            license: Vec::new(),
            authors: Vec::new(),
            description: None,
            homepage: None,
            keywords: Vec::new(),
            support: None,
            funding: Vec::new(),
            time: None,
            replace: BTreeMap::new(),
            provide: BTreeMap::new(),
            conflict: BTreeMap::new(),
            suggest: BTreeMap::new(),
            bin: Vec::new(),
            autoload_dev: None,
            default_branch: None,
            archive: None,
            abandoned: None,
            installation_source: None,
        }
    }

    /// Set source information.
    #[must_use]
    pub fn with_source(mut self, source: PackageSourceInfo) -> Self {
        self.source = Some(source);
        self
    }

    /// Set dist information.
    #[must_use]
    pub fn with_dist(mut self, dist: PackageDistInfo) -> Self {
        self.dist = Some(dist);
        self
    }

    /// Set installation time to current timestamp.
    #[must_use]
    pub fn with_current_time(mut self) -> Self {
        self.time = Some(
            chrono::Utc::now()
                .format("%Y-%m-%dT%H:%M:%S+00:00")
                .to_string(),
        );
        self
    }
}

impl Ord for LockedPackage {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.name.to_lowercase().cmp(&other.name.to_lowercase())
    }
}

impl PartialOrd for LockedPackage {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Package source (VCS) information.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PackageSourceInfo {
    /// Source type (git, svn, hg).
    #[serde(rename = "type")]
    pub source_type: String,

    /// Repository URL.
    pub url: String,

    /// Reference (commit hash, tag, branch).
    pub reference: String,
}

impl PackageSourceInfo {
    /// Create Git source.
    #[must_use]
    pub fn git(url: impl Into<String>, reference: impl Into<String>) -> Self {
        Self {
            source_type: "git".to_string(),
            url: url.into(),
            reference: reference.into(),
        }
    }

    /// Create SVN source.
    #[must_use]
    pub fn svn(url: impl Into<String>, reference: impl Into<String>) -> Self {
        Self {
            source_type: "svn".to_string(),
            url: url.into(),
            reference: reference.into(),
        }
    }
}

/// Package distribution (archive) information.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PackageDistInfo {
    /// Distribution type (zip, tar).
    #[serde(rename = "type")]
    pub dist_type: String,

    /// Download URL.
    pub url: String,

    /// Reference (usually same as source reference).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,

    /// SHA-1 checksum.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shasum: Option<String>,
}

impl PackageDistInfo {
    /// Create ZIP distribution.
    #[must_use]
    pub fn zip(url: impl Into<String>) -> Self {
        Self {
            dist_type: "zip".to_string(),
            url: url.into(),
            reference: None,
            shasum: None,
        }
    }

    /// Set reference.
    #[must_use]
    pub fn with_reference(mut self, reference: impl Into<String>) -> Self {
        self.reference = Some(reference.into());
        self
    }

    /// Set checksum.
    #[must_use]
    pub fn with_shasum(mut self, shasum: impl Into<String>) -> Self {
        self.shasum = Some(shasum.into());
        self
    }
}

/// Autoload configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AutoloadConfig {
    /// PSR-4 autoloading.
    #[serde(rename = "psr-4", default, skip_serializing_if = "BTreeMap::is_empty")]
    pub psr4: BTreeMap<String, AutoloadPath>,

    /// PSR-0 autoloading (legacy).
    #[serde(rename = "psr-0", default, skip_serializing_if = "BTreeMap::is_empty")]
    pub psr0: BTreeMap<String, AutoloadPath>,

    /// Classmap paths.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub classmap: Vec<String>,

    /// Files to include.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,

    /// Excluded paths from classmap.
    #[serde(
        rename = "exclude-from-classmap",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub exclude_from_classmap: Vec<String>,
}

impl Default for AutoloadConfig {
    fn default() -> Self {
        Self {
            psr4: BTreeMap::new(),
            psr0: BTreeMap::new(),
            classmap: Vec::new(),
            files: Vec::new(),
            exclude_from_classmap: Vec::new(),
        }
    }
}

/// Autoload path (can be string or array of strings).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum AutoloadPath {
    /// Single path.
    Single(String),
    /// Multiple paths.
    Multiple(Vec<String>),
}

impl AutoloadPath {
    /// Get paths as slice.
    #[must_use]
    pub fn as_vec(&self) -> Vec<&str> {
        match self {
            Self::Single(s) => vec![s.as_str()],
            Self::Multiple(v) => v.iter().map(String::as_str).collect(),
        }
    }
}

/// Package author.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PackageAuthor {
    /// Author name.
    pub name: String,

    /// Email address.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,

    /// Homepage URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,

    /// Role.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
}

impl PackageAuthor {
    /// Create author with name only.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            email: None,
            homepage: None,
            role: None,
        }
    }

    /// Set email.
    #[must_use]
    pub fn with_email(mut self, email: impl Into<String>) -> Self {
        self.email = Some(email.into());
        self
    }
}

/// Support information.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SupportInfo {
    /// Email.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,

    /// Issues URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issues: Option<String>,

    /// Forum URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forum: Option<String>,

    /// Wiki URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wiki: Option<String>,

    /// IRC channel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub irc: Option<String>,

    /// Source URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,

    /// Documentation URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docs: Option<String>,

    /// RSS feed URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rss: Option<String>,

    /// Chat URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat: Option<String>,

    /// Security contact URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub security: Option<String>,
}

/// Funding information.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FundingInfo {
    /// Funding type (github, patreon, etc.).
    #[serde(rename = "type")]
    pub funding_type: String,

    /// Funding URL.
    pub url: String,
}

/// Package alias.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PackageAlias {
    /// Package name.
    pub package: String,

    /// Original version.
    pub version: String,

    /// Aliased version.
    pub alias: String,

    /// Normalized aliased version.
    pub alias_normalized: String,
}

/// Archive configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArchiveConfig {
    /// Archive name format.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Excluded paths.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,
}

/// Abandoned value (can be bool or replacement package name).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum AbandonedValue {
    /// Simply abandoned.
    Bool(bool),
    /// Abandoned with replacement.
    Replacement(String),
}

/// Stability flag values matching Composer's internal representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum StabilityFlag {
    /// Stable releases.
    Stable = 0,
    /// Release candidates.
    Rc = 5,
    /// Beta releases.
    Beta = 10,
    /// Alpha releases.
    Alpha = 15,
    /// Development versions.
    Dev = 20,
}

impl StabilityFlag {
    /// Parse from string.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "stable" => Some(Self::Stable),
            "rc" => Some(Self::Rc),
            "beta" => Some(Self::Beta),
            "alpha" => Some(Self::Alpha),
            "dev" => Some(Self::Dev),
            _ => None,
        }
    }

    /// Convert to numeric value.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    /// Convert from numeric value.
    #[must_use]
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Stable),
            5 => Some(Self::Rc),
            10 => Some(Self::Beta),
            15 => Some(Self::Alpha),
            20 => Some(Self::Dev),
            _ => None,
        }
    }

    /// Get string representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Rc => "RC",
            Self::Beta => "beta",
            Self::Alpha => "alpha",
            Self::Dev => "dev",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_locked_package_ordering() {
        let mut packages = vec![
            LockedPackage::new("symfony/console", "6.0.0"),
            LockedPackage::new("psr/log", "3.0.0"),
            LockedPackage::new("Monolog/monolog", "3.0.0"),
        ];
        packages.sort();
        assert_eq!(packages[0].name, "Monolog/monolog");
        assert_eq!(packages[1].name, "psr/log");
        assert_eq!(packages[2].name, "symfony/console");
    }

    #[test]
    fn test_stability_flags() {
        assert_eq!(StabilityFlag::parse("dev"), Some(StabilityFlag::Dev));
        assert_eq!(StabilityFlag::Dev.as_u8(), 20);
        assert_eq!(StabilityFlag::from_u8(20), Some(StabilityFlag::Dev));
    }

    #[test]
    fn test_source_info() {
        let source = PackageSourceInfo::git("https://github.com/foo/bar", "abc123");
        assert_eq!(source.source_type, "git");
        assert_eq!(source.reference, "abc123");
    }

    #[test]
    fn test_dist_info() {
        let dist = PackageDistInfo::zip("https://example.com/pkg.zip")
            .with_reference("v1.0.0")
            .with_shasum("abc123");
        assert_eq!(dist.dist_type, "zip");
        assert_eq!(dist.shasum, Some("abc123".to_string()));
    }
}
