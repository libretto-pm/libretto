//! Composer.json parsing and manipulation.
//!
//! This module provides types for parsing and working with composer.json files,
//! using sonic-rs for high-performance JSON parsing.

use crate::package::{Dependency, PackageName};
use crate::version::{ComposerConstraint, Stability};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;
use std::str::FromStr;

/// A complete composer.json manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposerManifest {
    /// Package name.
    #[serde(default)]
    pub name: Option<String>,

    /// Package description.
    #[serde(default)]
    pub description: Option<String>,

    /// Package version (usually not in composer.json).
    #[serde(default)]
    pub version: Option<String>,

    /// Package type.
    #[serde(rename = "type", default)]
    pub package_type: Option<String>,

    /// Keywords.
    #[serde(default)]
    pub keywords: Vec<String>,

    /// Homepage URL.
    #[serde(default)]
    pub homepage: Option<String>,

    /// License.
    #[serde(default)]
    pub license: LicenseField,

    /// Authors.
    #[serde(default)]
    pub authors: Vec<Author>,

    /// Support information.
    #[serde(default)]
    pub support: Option<Support>,

    /// Funding information.
    #[serde(default)]
    pub funding: Vec<Funding>,

    /// Dependencies.
    #[serde(default)]
    pub require: BTreeMap<String, String>,

    /// Dev dependencies.
    #[serde(rename = "require-dev", default)]
    pub require_dev: BTreeMap<String, String>,

    /// Replaced packages.
    #[serde(default)]
    pub replace: BTreeMap<String, String>,

    /// Provided packages.
    #[serde(default)]
    pub provide: BTreeMap<String, String>,

    /// Conflicting packages.
    #[serde(default)]
    pub conflict: BTreeMap<String, String>,

    /// Suggested packages.
    #[serde(default)]
    pub suggest: BTreeMap<String, String>,

    /// Autoload configuration.
    #[serde(default)]
    pub autoload: Autoload,

    /// Dev autoload configuration.
    #[serde(rename = "autoload-dev", default)]
    pub autoload_dev: Autoload,

    /// Minimum stability.
    #[serde(rename = "minimum-stability", default)]
    pub minimum_stability: Option<String>,

    /// Prefer stable.
    #[serde(rename = "prefer-stable", default)]
    pub prefer_stable: Option<bool>,

    /// Repository configurations.
    #[serde(default)]
    pub repositories: Vec<Repository>,

    /// Configuration options.
    #[serde(default)]
    pub config: ComposerConfig,

    /// Scripts.
    #[serde(default)]
    pub scripts: BTreeMap<String, ScriptValue>,

    /// Extra data.
    #[serde(default)]
    pub extra: BTreeMap<String, sonic_rs::Value>,

    /// Bin files.
    #[serde(default)]
    pub bin: Vec<String>,

    /// Archive settings.
    #[serde(default)]
    pub archive: Option<Archive>,

    /// Non-feature branches.
    #[serde(rename = "non-feature-branches", default)]
    pub non_feature_branches: Vec<String>,

    /// Abandoned notice.
    #[serde(default)]
    pub abandoned: Option<AbandonedField>,
}

impl ComposerManifest {
    /// Load a composer.json from a file path.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or parsed.
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, ManifestError> {
        let content = fs::read(path.as_ref()).map_err(|e| ManifestError::Io {
            path: path.as_ref().to_path_buf(),
            source: e,
        })?;

        Self::from_slice(&content)
    }

    /// Parse a composer.json from bytes.
    ///
    /// # Errors
    ///
    /// Returns an error if the JSON is invalid.
    pub fn from_slice(data: &[u8]) -> Result<Self, ManifestError> {
        sonic_rs::from_slice(data).map_err(ManifestError::Json)
    }

    /// Parse a composer.json from a string.
    ///
    /// # Errors
    ///
    /// Returns an error if the JSON is invalid.
    pub fn parse(s: &str) -> Result<Self, ManifestError> {
        Self::from_slice(s.as_bytes())
    }

    /// Serialize to JSON string.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization fails.
    pub fn to_json(&self) -> Result<String, ManifestError> {
        sonic_rs::to_string(self).map_err(ManifestError::Json)
    }

    /// Serialize to pretty JSON string.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization fails.
    pub fn to_json_pretty(&self) -> Result<String, ManifestError> {
        sonic_rs::to_string_pretty(self).map_err(ManifestError::Json)
    }

    /// Get the package name.
    #[must_use]
    pub fn package_name(&self) -> Option<PackageName> {
        self.name.as_ref().and_then(|n| PackageName::parse(n))
    }

    /// Get the minimum stability.
    #[must_use]
    pub fn min_stability(&self) -> Stability {
        self.minimum_stability
            .as_ref()
            .and_then(|s| Stability::parse(s))
            .unwrap_or(Stability::Stable)
    }

    /// Get dependencies as parsed Dependency structs.
    #[must_use]
    pub fn dependencies(&self) -> Vec<Dependency> {
        self.require
            .iter()
            .filter_map(|(name, constraint)| {
                let pkg_name = PackageName::parse(name)?;
                let constraint = ComposerConstraint::parse(constraint)?;
                Some(Dependency::new(pkg_name, constraint))
            })
            .collect()
    }

    /// Get dev dependencies as parsed Dependency structs.
    #[must_use]
    pub fn dev_dependencies(&self) -> Vec<Dependency> {
        self.require_dev
            .iter()
            .filter_map(|(name, constraint)| {
                let pkg_name = PackageName::parse(name)?;
                let constraint = ComposerConstraint::parse(constraint)?;
                Some(Dependency::new(pkg_name, constraint))
            })
            .collect()
    }

    /// Get all dependencies (including dev).
    #[must_use]
    pub fn all_dependencies(&self) -> Vec<Dependency> {
        let mut deps = self.dependencies();
        deps.extend(self.dev_dependencies());
        deps
    }

    /// Get platform requirements (php, ext-*, lib-*).
    #[must_use]
    pub fn platform_requirements(&self) -> Vec<(String, String)> {
        self.require
            .iter()
            .filter(|(name, _)| is_platform_package(name))
            .map(|(n, c)| (n.clone(), c.clone()))
            .collect()
    }

    /// Add a dependency.
    pub fn add_dependency(&mut self, name: &str, constraint: &str) {
        self.require
            .insert(name.to_string(), constraint.to_string());
    }

    /// Add a dev dependency.
    pub fn add_dev_dependency(&mut self, name: &str, constraint: &str) {
        self.require_dev
            .insert(name.to_string(), constraint.to_string());
    }

    /// Remove a dependency.
    pub fn remove_dependency(&mut self, name: &str) -> Option<String> {
        self.require.remove(name)
    }

    /// Remove a dev dependency.
    pub fn remove_dev_dependency(&mut self, name: &str) -> Option<String> {
        self.require_dev.remove(name)
    }
}

impl FromStr for ComposerManifest {
    type Err = ManifestError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_slice(s.as_bytes())
    }
}

impl Default for ComposerManifest {
    fn default() -> Self {
        Self {
            name: None,
            description: None,
            version: None,
            package_type: None,
            keywords: Vec::new(),
            homepage: None,
            license: LicenseField::None,
            authors: Vec::new(),
            support: None,
            funding: Vec::new(),
            require: BTreeMap::new(),
            require_dev: BTreeMap::new(),
            replace: BTreeMap::new(),
            provide: BTreeMap::new(),
            conflict: BTreeMap::new(),
            suggest: BTreeMap::new(),
            autoload: Autoload::default(),
            autoload_dev: Autoload::default(),
            minimum_stability: None,
            prefer_stable: None,
            repositories: Vec::new(),
            config: ComposerConfig::default(),
            scripts: BTreeMap::new(),
            extra: BTreeMap::new(),
            bin: Vec::new(),
            archive: None,
            non_feature_branches: Vec::new(),
            abandoned: None,
        }
    }
}

/// License field (can be string or array).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(untagged)]
pub enum LicenseField {
    /// No license specified.
    #[default]
    None,
    /// Single license.
    Single(String),
    /// Multiple licenses (e.g., dual licensing).
    Multiple(Vec<String>),
}

impl LicenseField {
    /// Get licenses as a vector.
    #[must_use]
    pub fn as_vec(&self) -> Vec<&str> {
        match self {
            Self::None => Vec::new(),
            Self::Single(s) => vec![s.as_str()],
            Self::Multiple(v) => v.iter().map(String::as_str).collect(),
        }
    }
}

/// Author information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Author {
    /// Name.
    pub name: String,
    /// Email.
    #[serde(default)]
    pub email: Option<String>,
    /// Homepage.
    #[serde(default)]
    pub homepage: Option<String>,
    /// Role.
    #[serde(default)]
    pub role: Option<String>,
}

/// Support information.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Support {
    /// Email.
    #[serde(default)]
    pub email: Option<String>,
    /// Issues URL.
    #[serde(default)]
    pub issues: Option<String>,
    /// Forum URL.
    #[serde(default)]
    pub forum: Option<String>,
    /// Wiki URL.
    #[serde(default)]
    pub wiki: Option<String>,
    /// IRC channel.
    #[serde(default)]
    pub irc: Option<String>,
    /// Source URL.
    #[serde(default)]
    pub source: Option<String>,
    /// Documentation URL.
    #[serde(default)]
    pub docs: Option<String>,
    /// RSS feed.
    #[serde(default)]
    pub rss: Option<String>,
    /// Chat URL.
    #[serde(default)]
    pub chat: Option<String>,
    /// Security contact.
    #[serde(default)]
    pub security: Option<String>,
}

/// Funding information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Funding {
    /// Funding type.
    #[serde(rename = "type")]
    pub funding_type: String,
    /// Funding URL.
    pub url: String,
}

/// Autoload configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Autoload {
    /// PSR-4 autoloading.
    #[serde(rename = "psr-4", default)]
    pub psr4: BTreeMap<String, AutoloadPath>,
    /// PSR-0 autoloading (legacy).
    #[serde(rename = "psr-0", default)]
    pub psr0: BTreeMap<String, AutoloadPath>,
    /// Classmap autoloading.
    #[serde(default)]
    pub classmap: Vec<String>,
    /// Files to include.
    #[serde(default)]
    pub files: Vec<String>,
    /// Excluded paths.
    #[serde(rename = "exclude-from-classmap", default)]
    pub exclude_from_classmap: Vec<String>,
}

/// Autoload path (can be string or array).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AutoloadPath {
    /// Single path.
    Single(String),
    /// Multiple paths.
    Multiple(Vec<String>),
}

impl AutoloadPath {
    /// Get paths as a vector.
    #[must_use]
    pub fn as_vec(&self) -> Vec<&str> {
        match self {
            Self::Single(s) => vec![s.as_str()],
            Self::Multiple(v) => v.iter().map(String::as_str).collect(),
        }
    }
}

/// Repository configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Repository {
    /// Composer repository.
    #[serde(rename = "composer")]
    Composer {
        /// Repository URL.
        url: String,
        /// Options.
        #[serde(default)]
        options: BTreeMap<String, sonic_rs::Value>,
    },
    /// VCS repository.
    #[serde(rename = "vcs")]
    Vcs {
        /// Repository URL.
        url: String,
    },
    /// Package repository.
    #[serde(rename = "package")]
    Package {
        /// Package definition.
        package: sonic_rs::Value,
    },
    /// Path repository.
    #[serde(rename = "path")]
    Path {
        /// Path to local directory.
        url: String,
        /// Options.
        #[serde(default)]
        options: BTreeMap<String, sonic_rs::Value>,
    },
    /// Artifact repository.
    #[serde(rename = "artifact")]
    Artifact {
        /// Path to artifacts.
        url: String,
    },
}

/// Composer configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ComposerConfig {
    /// Process timeout.
    #[serde(rename = "process-timeout", default)]
    pub process_timeout: Option<u64>,
    /// Use include path.
    #[serde(rename = "use-include-path", default)]
    pub use_include_path: Option<bool>,
    /// Preferred install method.
    #[serde(rename = "preferred-install", default)]
    pub preferred_install: Option<PreferredInstall>,
    /// Notify on install.
    #[serde(rename = "notify-on-install", default)]
    pub notify_on_install: Option<bool>,
    /// GitHub protocols.
    #[serde(rename = "github-protocols", default)]
    pub github_protocols: Vec<String>,
    /// GitHub OAuth tokens.
    #[serde(rename = "github-oauth", default)]
    pub github_oauth: BTreeMap<String, String>,
    /// GitLab OAuth tokens.
    #[serde(rename = "gitlab-oauth", default)]
    pub gitlab_oauth: BTreeMap<String, String>,
    /// GitLab tokens.
    #[serde(rename = "gitlab-token", default)]
    pub gitlab_token: BTreeMap<String, String>,
    /// HTTP basic auth.
    #[serde(rename = "http-basic", default)]
    pub http_basic: BTreeMap<String, HttpBasicAuth>,
    /// Bearer tokens.
    #[serde(default)]
    pub bearer: BTreeMap<String, String>,
    /// Platform overrides.
    #[serde(default)]
    pub platform: BTreeMap<String, String>,
    /// Vendor directory.
    #[serde(rename = "vendor-dir", default)]
    pub vendor_dir: Option<String>,
    /// Bin directory.
    #[serde(rename = "bin-dir", default)]
    pub bin_dir: Option<String>,
    /// Data directory.
    #[serde(rename = "data-dir", default)]
    pub data_dir: Option<String>,
    /// Cache directory.
    #[serde(rename = "cache-dir", default)]
    pub cache_dir: Option<String>,
    /// Optimize autoloader.
    #[serde(rename = "optimize-autoloader", default)]
    pub optimize_autoloader: Option<bool>,
    /// Sort packages.
    #[serde(rename = "sort-packages", default)]
    pub sort_packages: Option<bool>,
    /// Classmap authoritative.
    #[serde(rename = "classmap-authoritative", default)]
    pub classmap_authoritative: Option<bool>,
    /// `APCu` autoloader.
    #[serde(rename = "apcu-autoloader", default)]
    pub apcu_autoloader: Option<bool>,
    /// Autoloader suffix.
    #[serde(rename = "autoloader-suffix", default)]
    pub autoloader_suffix: Option<String>,
    /// Secure HTTP.
    #[serde(rename = "secure-http", default)]
    pub secure_http: Option<bool>,
    /// Discard changes.
    #[serde(rename = "discard-changes", default)]
    pub discard_changes: Option<DiscardChanges>,
    /// Allow plugins.
    #[serde(rename = "allow-plugins", default)]
    pub allow_plugins: Option<AllowPlugins>,
}

/// HTTP basic auth credentials.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpBasicAuth {
    /// Username.
    pub username: String,
    /// Password.
    pub password: String,
}

/// Preferred install method.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PreferredInstall {
    /// Global setting.
    Global(String),
    /// Per-package settings.
    PerPackage(BTreeMap<String, String>),
}

/// Discard changes setting.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DiscardChanges {
    /// Boolean value.
    Bool(bool),
    /// String value (stash, etc.).
    String(String),
}

/// Allow plugins setting.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AllowPlugins {
    /// Allow all or none.
    Bool(bool),
    /// Per-plugin settings.
    PerPlugin(BTreeMap<String, bool>),
}

/// Script value (can be string or array).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ScriptValue {
    /// Single command.
    Single(String),
    /// Multiple commands.
    Multiple(Vec<String>),
}

impl ScriptValue {
    /// Get commands as a vector.
    #[must_use]
    pub fn as_vec(&self) -> Vec<&str> {
        match self {
            Self::Single(s) => vec![s.as_str()],
            Self::Multiple(v) => v.iter().map(String::as_str).collect(),
        }
    }
}

/// Archive configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Archive {
    /// Name format.
    #[serde(default)]
    pub name: Option<String>,
    /// Excluded files.
    #[serde(default)]
    pub exclude: Vec<String>,
}

/// Abandoned field (can be bool or string).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AbandonedField {
    /// Simply abandoned.
    Bool(bool),
    /// Abandoned with replacement suggestion.
    Replacement(String),
}

/// Error when parsing a manifest.
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    /// IO error.
    #[error("failed to read {path}: {source}")]
    Io {
        /// Path that failed.
        path: std::path::PathBuf,
        /// IO error.
        source: io::Error,
    },
    /// JSON parsing error.
    #[error("JSON parse error: {0}")]
    Json(#[from] sonic_rs::Error),
}

/// Check if a package is a platform package.
fn is_platform_package(name: &str) -> bool {
    libretto_core::is_platform_package_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_manifest() {
        let json = r#"{
            "name": "vendor/package",
            "description": "A test package",
            "require": {
                "php": "^8.0",
                "symfony/console": "^6.0"
            },
            "require-dev": {
                "phpunit/phpunit": "^10.0"
            }
        }"#;

        let manifest = ComposerManifest::from_str(json).unwrap();

        assert_eq!(manifest.name.as_deref(), Some("vendor/package"));
        assert_eq!(manifest.description.as_deref(), Some("A test package"));
        assert_eq!(manifest.require.len(), 2);
        assert_eq!(manifest.require_dev.len(), 1);
    }

    #[test]
    fn test_package_name() {
        let json = r#"{"name": "vendor/package"}"#;
        let manifest = ComposerManifest::from_str(json).unwrap();

        let name = manifest.package_name().unwrap();
        assert_eq!(name.vendor(), "vendor");
        assert_eq!(name.name(), "package");
    }

    #[test]
    fn test_dependencies() {
        let json = r#"{
            "require": {
                "symfony/console": "^6.0",
                "psr/log": "^3.0"
            }
        }"#;

        let manifest = ComposerManifest::from_str(json).unwrap();
        let deps = manifest.dependencies();

        assert_eq!(deps.len(), 2);
    }

    #[test]
    fn test_platform_requirements() {
        let json = r#"{
            "require": {
                "php": "^8.0",
                "ext-json": "*",
                "php-open-source-saver/jwt-auth": "^2.0",
                "symfony/console": "^6.0"
            }
        }"#;

        let manifest = ComposerManifest::from_str(json).unwrap();
        let platform = manifest.platform_requirements();

        assert_eq!(platform.len(), 2);
        assert!(platform.iter().any(|(n, _)| n == "php"));
        assert!(platform.iter().any(|(n, _)| n == "ext-json"));
        assert!(
            !platform
                .iter()
                .any(|(n, _)| n == "php-open-source-saver/jwt-auth")
        );
    }

    #[test]
    fn test_min_stability() {
        let json = r#"{"minimum-stability": "dev"}"#;
        let manifest = ComposerManifest::from_str(json).unwrap();

        assert_eq!(manifest.min_stability(), Stability::Dev);
    }

    #[test]
    fn test_autoload() {
        let json = r#"{
            "autoload": {
                "psr-4": {
                    "Vendor\\Package\\": "src/"
                },
                "files": ["src/helpers.php"]
            }
        }"#;

        let manifest = ComposerManifest::from_str(json).unwrap();

        assert!(manifest.autoload.psr4.contains_key("Vendor\\Package\\"));
        assert_eq!(manifest.autoload.files.len(), 1);
    }

    #[test]
    fn test_repositories() {
        let json = r#"{
            "repositories": [
                {"type": "vcs", "url": "https://github.com/example/repo"},
                {"type": "composer", "url": "https://packagist.org"}
            ]
        }"#;

        let manifest = ComposerManifest::from_str(json).unwrap();

        assert_eq!(manifest.repositories.len(), 2);
    }

    #[test]
    fn test_add_remove_dependency() {
        let mut manifest = ComposerManifest::default();

        manifest.add_dependency("vendor/pkg", "^1.0");
        assert!(manifest.require.contains_key("vendor/pkg"));

        manifest.remove_dependency("vendor/pkg");
        assert!(!manifest.require.contains_key("vendor/pkg"));
    }

    #[test]
    fn test_serialize() {
        let mut manifest = ComposerManifest {
            name: Some("test/package".to_string()),
            ..Default::default()
        };
        manifest.add_dependency("dep/one", "^1.0");

        let json = manifest.to_json_pretty().unwrap();
        assert!(json.contains("test/package"));
        assert!(json.contains("dep/one"));
    }
}
