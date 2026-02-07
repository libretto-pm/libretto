//! Core configuration types for Composer compatibility.

use crate::auth::{
    BearerToken, BitbucketOAuthCredentials, GitLabOAuthToken, GitLabToken, HttpBasicCredentials,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Preferred installation method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PreferredInstall {
    /// Install from source (VCS).
    Source,
    /// Install from distribution archive.
    #[default]
    Dist,
    /// Auto-select based on stability.
    Auto,
}

/// Store authentication credentials setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StoreAuths {
    /// Always store credentials.
    #[serde(rename = "true")]
    Always,
    /// Never store credentials.
    #[serde(rename = "false")]
    Never,
    /// Prompt user before storing.
    #[default]
    Prompt,
}

/// Binary compatibility mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BinCompat {
    /// Auto-detect compatibility mode.
    #[default]
    Auto,
    /// Full compatibility mode.
    Full,
    /// Proxy mode.
    Proxy,
    /// Symlink mode.
    Symlink,
}

/// Archive format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArchiveFormat {
    /// TAR archive.
    Tar,
    /// ZIP archive.
    #[default]
    Zip,
}

/// Platform check mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PlatformCheck {
    /// Full platform requirements check.
    #[default]
    #[serde(rename = "true")]
    Full,
    /// Check only PHP version.
    PhpOnly,
    /// Disable platform checks.
    #[serde(rename = "false")]
    Disabled,
}

/// GitHub protocol preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GitHubProtocol {
    /// HTTPS protocol.
    Https,
    /// SSH protocol.
    Ssh,
    /// Git protocol.
    Git,
}

/// Minimum stability level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stability {
    /// Development version.
    Dev,
    /// Alpha version.
    Alpha,
    /// Beta version.
    Beta,
    /// Release candidate.
    #[serde(rename = "RC")]
    Rc,
    /// Stable release.
    #[default]
    Stable,
}

/// Main Composer configuration section.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", default)]
pub struct ComposerConfig {
    /// Maximum script execution time in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_timeout: Option<u32>,

    /// Use PHP `include_path` for autoloading.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_include_path: Option<bool>,

    /// Preferred installation method.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred_install: Option<PreferredInstallConfig>,

    /// Store authentication credentials.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store_auths: Option<StoreAuths>,

    /// Show package notifications on install.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notify_on_install: Option<bool>,

    /// GitHub protocols in order of preference.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github_protocols: Option<Vec<GitHubProtocol>>,

    /// GitHub OAuth tokens by domain.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github_oauth: Option<BTreeMap<String, String>>,

    /// GitLab OAuth tokens by domain.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gitlab_oauth: Option<BTreeMap<String, GitLabOAuthToken>>,

    /// GitLab private tokens by domain.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gitlab_token: Option<BTreeMap<String, GitLabToken>>,

    /// Bitbucket OAuth credentials by domain.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bitbucket_oauth: Option<BTreeMap<String, BitbucketOAuthCredentials>>,

    /// HTTP Basic auth credentials by domain.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_basic: Option<BTreeMap<String, HttpBasicCredentials>>,

    /// Bearer tokens by domain.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bearer: Option<BTreeMap<String, BearerToken>>,

    /// Platform package overrides.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<BTreeMap<String, PlatformValue>>,

    /// Vendor directory path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vendor_dir: Option<String>,

    /// Binaries directory path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bin_dir: Option<String>,

    /// Data directory path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_dir: Option<String>,

    /// Cache directory override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_dir: Option<String>,

    /// Cache files directory.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_files_dir: Option<String>,

    /// Cache repository directory.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_repo_dir: Option<String>,

    /// Cache VCS directory.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_vcs_dir: Option<String>,

    /// Cache files TTL in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_files_ttl: Option<u32>,

    /// Maximum cache size (e.g., "300MiB").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_files_maxsize: Option<String>,

    /// Binary compatibility mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bin_compat: Option<BinCompat>,

    /// Prepend autoloader to existing autoloader stack.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prepend_autoloader: Option<bool>,

    /// Custom autoloader suffix.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub autoloader_suffix: Option<String>,

    /// Enable autoloader optimization.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub optimize_autoloader: Option<bool>,

    /// Sort packages in composer.json.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_packages: Option<bool>,

    /// Enable authoritative classmap mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classmap_authoritative: Option<bool>,

    /// Enable `APCu` cache for autoloader.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub apcu_autoloader: Option<bool>,

    /// Custom GitHub Enterprise domains.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github_domains: Option<Vec<String>>,

    /// Expose hostname to GitHub for statistics.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github_expose_hostname: Option<bool>,

    /// Custom GitLab domains.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gitlab_domains: Option<Vec<String>>,

    /// Use GitHub API for repository information.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_github_api: Option<bool>,

    /// Archive format for packaging.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archive_format: Option<ArchiveFormat>,

    /// Archive output directory.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archive_dir: Option<String>,

    /// Protect web-accessible directories with .htaccess.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub htaccess_protect: Option<bool>,

    /// Lock dependencies to exact versions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lock: Option<bool>,

    /// Platform requirements check mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform_check: Option<PlatformCheck>,

    /// Require HTTPS for all downloads.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secure_http: Option<bool>,

    /// Disable TLS verification (not recommended).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disable_tls: Option<bool>,

    /// Custom CA certificate file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cafile: Option<String>,

    /// Custom CA certificate directory.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capath: Option<String>,

    /// Discard changes in vendor packages.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discard_changes: Option<DiscardChanges>,

    /// Allow plugins to run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_plugins: Option<AllowPlugins>,

    /// Audit configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audit: Option<AuditConfig>,

    /// Extra configuration for extensions.
    #[serde(flatten)]
    pub extra: BTreeMap<String, sonic_rs::Value>,
}

/// Preferred install configuration (global or per-package).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PreferredInstallConfig {
    /// Global setting.
    Global(PreferredInstall),
    /// Per-package settings.
    PerPackage(BTreeMap<String, PreferredInstall>),
}

impl Default for PreferredInstallConfig {
    fn default() -> Self {
        Self::Global(PreferredInstall::default())
    }
}

/// Platform value (version string or false to disable).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PlatformValue {
    /// Platform version.
    Version(String),
    /// Disable platform package.
    Disabled(bool),
}

/// Discard changes setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DiscardChanges {
    /// Boolean value.
    Bool(bool),
    /// Stash changes.
    #[serde(rename = "stash")]
    Stash,
}

impl Default for DiscardChanges {
    fn default() -> Self {
        Self::Bool(false)
    }
}

/// Allow plugins configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AllowPlugins {
    /// Global setting (true/false).
    Global(bool),
    /// Per-plugin settings.
    PerPlugin(BTreeMap<String, bool>),
}

impl Default for AllowPlugins {
    fn default() -> Self {
        Self::Global(true)
    }
}

// Auth credential types are defined in auth.rs - use auth::HttpBasicCredentials, etc.

/// Audit configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", default)]
pub struct AuditConfig {
    /// Ignored advisories by package name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ignored: Option<Vec<String>>,
    /// Abandoned package handling.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub abandoned: Option<AbandonedHandling>,
}

/// Abandoned package handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AbandonedHandling {
    /// Ignore abandoned packages.
    Ignore,
    /// Report abandoned packages.
    #[default]
    Report,
    /// Fail on abandoned packages.
    Fail,
}

/// Scripts configuration section.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ScriptsConfig {
    /// Pre-install command scripts.
    #[serde(rename = "pre-install-cmd", skip_serializing_if = "Option::is_none")]
    pub pre_install_cmd: Option<Scripts>,
    /// Post-install command scripts.
    #[serde(rename = "post-install-cmd", skip_serializing_if = "Option::is_none")]
    pub post_install_cmd: Option<Scripts>,
    /// Pre-update command scripts.
    #[serde(rename = "pre-update-cmd", skip_serializing_if = "Option::is_none")]
    pub pre_update_cmd: Option<Scripts>,
    /// Post-update command scripts.
    #[serde(rename = "post-update-cmd", skip_serializing_if = "Option::is_none")]
    pub post_update_cmd: Option<Scripts>,
    /// Pre-autoload dump scripts.
    #[serde(rename = "pre-autoload-dump", skip_serializing_if = "Option::is_none")]
    pub pre_autoload_dump: Option<Scripts>,
    /// Post-autoload dump scripts.
    #[serde(rename = "post-autoload-dump", skip_serializing_if = "Option::is_none")]
    pub post_autoload_dump: Option<Scripts>,
    /// Post-root package install scripts.
    #[serde(
        rename = "post-root-package-install",
        skip_serializing_if = "Option::is_none"
    )]
    pub post_root_package_install: Option<Scripts>,
    /// Post-create project command scripts.
    #[serde(
        rename = "post-create-project-cmd",
        skip_serializing_if = "Option::is_none"
    )]
    pub post_create_project_cmd: Option<Scripts>,
    /// Custom scripts.
    #[serde(flatten)]
    pub custom: BTreeMap<String, Scripts>,
}

/// Script definition (single or multiple).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Scripts {
    /// Single script.
    Single(String),
    /// Multiple scripts.
    Multiple(Vec<String>),
}

impl Scripts {
    /// Get scripts as a vector.
    #[must_use]
    pub fn as_vec(&self) -> Vec<&str> {
        match self {
            Self::Single(s) => vec![s.as_str()],
            Self::Multiple(v) => v.iter().map(String::as_str).collect(),
        }
    }
}

/// Repository type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RepositoryType {
    /// Composer repository (Packagist-compatible).
    Composer,
    /// VCS repository (Git, SVN, etc.).
    Vcs,
    /// Git repository.
    Git,
    /// GitHub repository.
    Github,
    /// GitLab repository.
    Gitlab,
    /// Bitbucket repository.
    Bitbucket,
    /// SVN repository.
    Svn,
    /// Mercurial repository.
    Hg,
    /// PEAR repository.
    Pear,
    /// Local path repository.
    Path,
    /// Artifact (ZIP files) repository.
    Artifact,
    /// Inline package definition.
    Package,
}

/// Repository configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RepositoryConfig {
    /// Disable a repository (e.g., packagist.org).
    Disabled(bool),
    /// Full repository configuration.
    Config(Box<RepositoryDefinition>),
}

/// Repository definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepositoryDefinition {
    /// Repository type.
    #[serde(rename = "type")]
    pub repo_type: RepositoryType,
    /// Repository URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Repository options.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<RepositoryOptions>,
    /// Inline package definition (for type: package).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package: Option<sonic_rs::Value>,
    /// Canonical flag for repository ordering.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canonical: Option<bool>,
    /// Only packages matching these patterns.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub only: Option<Vec<String>>,
    /// Exclude packages matching these patterns.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude: Option<Vec<String>>,
}

/// Repository options.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", default)]
pub struct RepositoryOptions {
    /// SSL verification.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssl: Option<SslOptions>,
    /// HTTP options.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http: Option<HttpOptions>,
    /// Symlink packages instead of copying.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symlink: Option<bool>,
    /// Reference type for VCS repositories.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
}

/// SSL options.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SslOptions {
    /// Verify peer certificate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verify_peer: Option<bool>,
    /// Verify peer name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verify_peer_name: Option<bool>,
    /// Allow self-signed certificates.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_self_signed: Option<bool>,
    /// CA file path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cafile: Option<String>,
    /// CA path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capath: Option<String>,
}

/// HTTP options.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HttpOptions {
    /// Request timeout.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<f64>,
    /// Proxy URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxy: Option<String>,
    /// Custom headers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header: Option<Vec<String>>,
}

/// Autoload configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", default)]
pub struct AutoloadConfig {
    /// PSR-4 autoloading rules.
    #[serde(rename = "psr-4", skip_serializing_if = "Option::is_none")]
    pub psr4: Option<BTreeMap<String, AutoloadPath>>,
    /// PSR-0 autoloading rules.
    #[serde(rename = "psr-0", skip_serializing_if = "Option::is_none")]
    pub psr0: Option<BTreeMap<String, AutoloadPath>>,
    /// Classmap directories.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classmap: Option<Vec<String>>,
    /// Files to include.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<String>>,
    /// Excluded paths from classmap.
    #[serde(
        rename = "exclude-from-classmap",
        skip_serializing_if = "Option::is_none"
    )]
    pub exclude_from_classmap: Option<Vec<String>>,
}

/// Autoload path (single or multiple).
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

/// Full composer.json manifest.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", default)]
pub struct ComposerManifest {
    /// Package name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Package description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Package type.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub package_type: Option<String>,
    /// Keywords.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keywords: Option<Vec<String>>,
    /// Homepage URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    /// License.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<License>,
    /// Authors.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authors: Option<Vec<Author>>,
    /// Support information.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub support: Option<SupportInfo>,
    /// Funding links.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub funding: Option<Vec<FundingLink>>,
    /// Required dependencies.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub require: Option<BTreeMap<String, String>>,
    /// Development dependencies.
    #[serde(rename = "require-dev", skip_serializing_if = "Option::is_none")]
    pub require_dev: Option<BTreeMap<String, String>>,
    /// Conflicting packages.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conflict: Option<BTreeMap<String, String>>,
    /// Replaced packages.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replace: Option<BTreeMap<String, String>>,
    /// Provided packages.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provide: Option<BTreeMap<String, String>>,
    /// Suggested packages.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggest: Option<BTreeMap<String, String>>,
    /// Autoload configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub autoload: Option<AutoloadConfig>,
    /// Development autoload configuration.
    #[serde(rename = "autoload-dev", skip_serializing_if = "Option::is_none")]
    pub autoload_dev: Option<AutoloadConfig>,
    /// Minimum stability.
    #[serde(rename = "minimum-stability", skip_serializing_if = "Option::is_none")]
    pub minimum_stability: Option<Stability>,
    /// Prefer stable versions.
    #[serde(rename = "prefer-stable", skip_serializing_if = "Option::is_none")]
    pub prefer_stable: Option<bool>,
    /// Repositories.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repositories: Option<Repositories>,
    /// Configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<ComposerConfig>,
    /// Scripts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scripts: Option<ScriptsConfig>,
    /// Extra metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<sonic_rs::Value>,
    /// Binary files.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bin: Option<Vec<String>>,
    /// Archive configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archive: Option<ArchiveConfig>,
    /// Non-feature branches pattern.
    #[serde(
        rename = "non-feature-branches",
        skip_serializing_if = "Option::is_none"
    )]
    pub non_feature_branches: Option<Vec<String>>,
    /// Package version (usually auto-detected).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Time of release.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<String>,
    /// Abandoned status.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub abandoned: Option<AbandonedStatus>,
}

/// Repositories configuration (array or object).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Repositories {
    /// Array of repositories.
    Array(Vec<RepositoryConfig>),
    /// Object with named repositories.
    Object(BTreeMap<String, RepositoryConfig>),
}

impl Default for Repositories {
    fn default() -> Self {
        Self::Array(Vec::new())
    }
}

/// License (single or multiple).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum License {
    /// Single license.
    Single(String),
    /// Multiple licenses (disjunction).
    Multiple(Vec<String>),
}

/// Author information.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Author {
    /// Author name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Author email.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    /// Author homepage.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    /// Author role.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
}

/// Support information.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SupportInfo {
    /// Email support.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    /// Issue tracker URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issues: Option<String>,
    /// Forum URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forum: Option<String>,
    /// Wiki URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wiki: Option<String>,
    /// IRC channel.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub irc: Option<String>,
    /// Source repository URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Documentation URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docs: Option<String>,
    /// RSS feed URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rss: Option<String>,
    /// Chat URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat: Option<String>,
    /// Security policy URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub security: Option<String>,
}

/// Funding link.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FundingLink {
    /// Funding type (e.g., "github", "patreon", "opencollective").
    #[serde(rename = "type")]
    pub funding_type: String,
    /// Funding URL.
    pub url: String,
}

/// Archive configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ArchiveConfig {
    /// Name of the archive.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Excluded paths.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude: Option<Vec<String>>,
}

/// Abandoned status.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AbandonedStatus {
    /// Boolean abandoned status.
    Bool(bool),
    /// Replacement package.
    Replacement(String),
}

/// Resolved configuration with all sources merged.
#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    /// Process timeout in seconds.
    pub process_timeout: u32,
    /// Use PHP `include_path`.
    pub use_include_path: bool,
    /// Preferred installation method.
    pub preferred_install: PreferredInstallConfig,
    /// Store authentication credentials.
    pub store_auths: StoreAuths,
    /// Show package notifications.
    pub notify_on_install: bool,
    /// GitHub protocols.
    pub github_protocols: Vec<GitHubProtocol>,
    /// Vendor directory.
    pub vendor_dir: PathBuf,
    /// Binaries directory.
    pub bin_dir: PathBuf,
    /// Data directory.
    pub data_dir: PathBuf,
    /// Cache directory.
    pub cache_dir: PathBuf,
    /// Cache files directory.
    pub cache_files_dir: PathBuf,
    /// Cache repository directory.
    pub cache_repo_dir: PathBuf,
    /// Cache VCS directory.
    pub cache_vcs_dir: PathBuf,
    /// Cache files TTL in seconds.
    pub cache_files_ttl: u32,
    /// Maximum cache size in bytes.
    pub cache_files_maxsize: u64,
    /// Binary compatibility mode.
    pub bin_compat: BinCompat,
    /// Prepend autoloader.
    pub prepend_autoloader: bool,
    /// Autoloader suffix.
    pub autoloader_suffix: Option<String>,
    /// Optimize autoloader.
    pub optimize_autoloader: bool,
    /// Sort packages.
    pub sort_packages: bool,
    /// Authoritative classmap.
    pub classmap_authoritative: bool,
    /// `APCu` autoloader.
    pub apcu_autoloader: bool,
    /// GitHub domains.
    pub github_domains: Vec<String>,
    /// Expose hostname to GitHub.
    pub github_expose_hostname: bool,
    /// GitLab domains.
    pub gitlab_domains: Vec<String>,
    /// Use GitHub API.
    pub use_github_api: bool,
    /// Archive format.
    pub archive_format: ArchiveFormat,
    /// Archive directory.
    pub archive_dir: PathBuf,
    /// Protect with .htaccess.
    pub htaccess_protect: bool,
    /// Lock file enabled.
    pub lock: bool,
    /// Platform check mode.
    pub platform_check: PlatformCheck,
    /// Require HTTPS.
    pub secure_http: bool,
    /// Disable TLS.
    pub disable_tls: bool,
    /// Custom CA file.
    pub cafile: Option<PathBuf>,
    /// Custom CA path.
    pub capath: Option<PathBuf>,
    /// Discard changes.
    pub discard_changes: DiscardChanges,
    /// Allow plugins.
    pub allow_plugins: AllowPlugins,
    /// Platform overrides.
    pub platform: BTreeMap<String, PlatformValue>,
    /// Home directory (`COMPOSER_HOME`).
    pub home_dir: PathBuf,
    /// Project root directory.
    pub project_dir: PathBuf,
    /// Allow running as root.
    pub allow_superuser: bool,
    /// Offline mode.
    pub offline: bool,
    /// HTTP proxy.
    pub http_proxy: Option<String>,
    /// HTTPS proxy.
    pub https_proxy: Option<String>,
    /// No proxy hosts.
    pub no_proxy: Option<String>,
}

impl Default for ResolvedConfig {
    fn default() -> Self {
        let platform = libretto_platform::Platform::current();
        let home = platform.config_dir.clone();
        let cache = platform.cache_dir.clone();

        Self {
            process_timeout: 300,
            use_include_path: false,
            preferred_install: PreferredInstallConfig::default(),
            store_auths: StoreAuths::default(),
            notify_on_install: true,
            github_protocols: vec![
                GitHubProtocol::Https,
                GitHubProtocol::Ssh,
                GitHubProtocol::Git,
            ],
            vendor_dir: PathBuf::from("vendor"),
            bin_dir: PathBuf::from("vendor/bin"),
            data_dir: home.join("data"),
            cache_dir: cache.clone(),
            cache_files_dir: cache.join("files"),
            cache_repo_dir: cache.join("repo"),
            cache_vcs_dir: cache.join("vcs"),
            cache_files_ttl: 15_552_000,            // 6 months
            cache_files_maxsize: 300 * 1024 * 1024, // 300 MiB
            bin_compat: BinCompat::default(),
            prepend_autoloader: true,
            autoloader_suffix: None,
            optimize_autoloader: false,
            sort_packages: false,
            classmap_authoritative: false,
            apcu_autoloader: false,
            github_domains: vec!["github.com".to_string()],
            github_expose_hostname: true,
            gitlab_domains: vec!["gitlab.com".to_string()],
            use_github_api: true,
            archive_format: ArchiveFormat::default(),
            archive_dir: PathBuf::from("."),
            htaccess_protect: true,
            lock: true,
            platform_check: PlatformCheck::default(),
            secure_http: true,
            disable_tls: false,
            cafile: None,
            capath: None,
            discard_changes: DiscardChanges::default(),
            allow_plugins: AllowPlugins::default(),
            platform: BTreeMap::new(),
            home_dir: home,
            project_dir: PathBuf::from("."),
            allow_superuser: false,
            offline: false,
            http_proxy: None,
            https_proxy: None,
            no_proxy: None,
        }
    }
}
