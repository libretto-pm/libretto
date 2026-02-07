//! Configuration loader with hierarchical merging.

use crate::auth::AuthConfig;
use crate::env::EnvConfig;
use crate::error::{ConfigError, Result};
use crate::types::{ComposerConfig, ComposerManifest, ResolvedConfig};
use libretto_platform::Platform;
use std::path::{Path, PathBuf};

/// Configuration source in hierarchy order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConfigSource {
    /// Built-in defaults.
    Defaults = 0,
    /// System-wide configuration.
    System = 1,
    /// User global configuration.
    Global = 2,
    /// Project-local configuration.
    Project = 3,
    /// Environment variables.
    Environment = 4,
    /// CLI arguments.
    Cli = 5,
}

impl ConfigSource {
    /// Get description for display.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::Defaults => "built-in defaults",
            Self::System => "system configuration",
            Self::Global => "global configuration",
            Self::Project => "project configuration",
            Self::Environment => "environment variables",
            Self::Cli => "command-line arguments",
        }
    }
}

/// Configuration loader with caching and hierarchy support.
#[derive(Debug)]
pub struct ConfigLoader {
    /// Platform information.
    platform: &'static Platform,
    /// Project directory.
    project_dir: PathBuf,
    /// Cached project manifest.
    manifest: Option<ComposerManifest>,
    /// Cached environment config.
    env_config: EnvConfig,
    /// Cached auth config.
    auth_config: AuthConfig,
}

impl ConfigLoader {
    /// Create a new configuration loader.
    #[must_use]
    pub fn new(project_dir: impl Into<PathBuf>) -> Self {
        let platform = Platform::current();
        let project_dir = project_dir.into();
        let env_config = EnvConfig::from_env();

        Self {
            platform,
            project_dir,
            manifest: None,
            env_config,
            auth_config: AuthConfig::default(),
        }
    }

    /// Get the system configuration path.
    #[must_use]
    pub fn system_config_path(&self) -> PathBuf {
        if self.platform.is_windows() {
            PathBuf::from(r"C:\ProgramData\libretto\config.json")
        } else if self.platform.is_macos() {
            PathBuf::from("/Library/Application Support/libretto/config.json")
        } else {
            PathBuf::from("/etc/libretto/config.json")
        }
    }

    /// Get the global configuration path.
    #[must_use]
    pub fn global_config_path(&self) -> PathBuf {
        self.env_config
            .home
            .clone()
            .unwrap_or_else(|| self.platform.config_dir.clone())
            .join("config.json")
    }

    /// Get the global auth.json path.
    #[must_use]
    pub fn global_auth_path(&self) -> PathBuf {
        self.env_config
            .home
            .clone()
            .unwrap_or_else(|| self.platform.config_dir.clone())
            .join("auth.json")
    }

    /// Get the project composer.json path.
    #[must_use]
    pub fn project_manifest_path(&self) -> PathBuf {
        self.env_config
            .composer
            .clone()
            .unwrap_or_else(|| self.project_dir.join("composer.json"))
    }

    /// Get the project auth.json path.
    #[must_use]
    pub fn project_auth_path(&self) -> PathBuf {
        self.project_dir.join("auth.json")
    }

    /// Load system configuration.
    fn load_system_config(&self) -> Option<ComposerConfig> {
        let path = self.system_config_path();
        self.load_config_file(&path).ok()
    }

    /// Load global configuration.
    fn load_global_config(&self) -> Option<ComposerConfig> {
        let path = self.global_config_path();
        self.load_config_file(&path).ok()
    }

    /// Load project configuration from composer.json.
    fn load_project_config(&self) -> Option<ComposerConfig> {
        let path = self.project_manifest_path();
        match self.load_manifest(&path) {
            Ok(manifest) => manifest.config,
            Err(_) => None,
        }
    }

    /// Load a configuration file.
    fn load_config_file(&self, path: &Path) -> Result<ComposerConfig> {
        let content = std::fs::read_to_string(path).map_err(|e| ConfigError::io(path, e))?;
        sonic_rs::from_str(&content).map_err(|e| ConfigError::json(path, &e))
    }

    /// Load a composer.json manifest.
    fn load_manifest(&self, path: &Path) -> Result<ComposerManifest> {
        let content = std::fs::read_to_string(path).map_err(|e| ConfigError::io(path, e))?;
        sonic_rs::from_str(&content).map_err(|e| ConfigError::json(path, &e))
    }

    /// Load and cache the project manifest.
    ///
    /// # Errors
    /// Returns error if manifest cannot be loaded.
    pub fn load_project_manifest(&mut self) -> Result<&ComposerManifest> {
        if self.manifest.is_none() {
            let path = self.project_manifest_path();
            self.manifest = Some(self.load_manifest(&path)?);
        }
        Ok(self.manifest.as_ref().expect("manifest was just set"))
    }

    /// Load all authentication configurations.
    ///
    /// # Errors
    /// Returns error if auth config is invalid.
    pub fn load_auth(&mut self) -> Result<&AuthConfig> {
        // Start with global auth
        let global_path = self.global_auth_path();
        if global_path.exists() {
            self.auth_config = AuthConfig::load(&global_path)?;
        }

        // Merge project auth (overrides global)
        let project_path = self.project_auth_path();
        if project_path.exists() {
            let project_auth = AuthConfig::load(&project_path)?;
            self.auth_config.merge(&project_auth);
        }

        // Merge environment auth (highest priority)
        if let Some(env_auth) = self.env_config.parse_auth()? {
            self.auth_config.merge(&env_auth);
        }

        Ok(&self.auth_config)
    }

    /// Build resolved configuration by merging all sources.
    ///
    /// # Errors
    /// Returns error if configuration is invalid.
    pub fn resolve(&self) -> Result<ResolvedConfig> {
        let mut resolved = ResolvedConfig {
            project_dir: self.project_dir.clone(),
            home_dir: self
                .env_config
                .home
                .clone()
                .unwrap_or_else(|| self.platform.config_dir.clone()),
            ..Default::default()
        };

        // Layer 1: System config
        if let Some(system) = self.load_system_config() {
            self.apply_config(&mut resolved, &system);
        }

        // Layer 2: Global config
        if let Some(global) = self.load_global_config() {
            self.apply_config(&mut resolved, &global);
        }

        // Layer 3: Project config
        if let Some(project) = self.load_project_config() {
            self.apply_config(&mut resolved, &project);
        }

        // Layer 4: Environment variables
        self.apply_env(&mut resolved);

        Ok(resolved)
    }

    /// Apply composer config to resolved config.
    fn apply_config(&self, resolved: &mut ResolvedConfig, config: &ComposerConfig) {
        if let Some(timeout) = config.process_timeout {
            resolved.process_timeout = timeout;
        }
        if let Some(use_include) = config.use_include_path {
            resolved.use_include_path = use_include;
        }
        if let Some(ref preferred) = config.preferred_install {
            resolved.preferred_install = preferred.clone();
        }
        if let Some(store) = config.store_auths {
            resolved.store_auths = store;
        }
        if let Some(notify) = config.notify_on_install {
            resolved.notify_on_install = notify;
        }
        if let Some(ref protocols) = config.github_protocols {
            resolved.github_protocols = protocols.clone();
        }
        if let Some(ref vendor) = config.vendor_dir {
            resolved.vendor_dir = self.resolve_path(vendor);
        }
        if let Some(ref bin) = config.bin_dir {
            resolved.bin_dir = self.resolve_path(bin);
        }
        if let Some(ref data) = config.data_dir {
            resolved.data_dir = self.resolve_path(data);
        }
        if let Some(ref cache) = config.cache_dir {
            resolved.cache_dir = self.resolve_path(cache);
        }
        if let Some(ref cache_files) = config.cache_files_dir {
            resolved.cache_files_dir = self.resolve_path(cache_files);
        }
        if let Some(ref cache_repo) = config.cache_repo_dir {
            resolved.cache_repo_dir = self.resolve_path(cache_repo);
        }
        if let Some(ref cache_vcs) = config.cache_vcs_dir {
            resolved.cache_vcs_dir = self.resolve_path(cache_vcs);
        }
        if let Some(ttl) = config.cache_files_ttl {
            resolved.cache_files_ttl = ttl;
        }
        if let Some(ref maxsize) = config.cache_files_maxsize
            && let Ok(bytes) = crate::env::parse_byte_size(maxsize)
        {
            resolved.cache_files_maxsize = bytes;
        }
        if let Some(compat) = config.bin_compat {
            resolved.bin_compat = compat;
        }
        if let Some(prepend) = config.prepend_autoloader {
            resolved.prepend_autoloader = prepend;
        }
        if let Some(ref suffix) = config.autoloader_suffix {
            resolved.autoloader_suffix = Some(suffix.clone());
        }
        if let Some(optimize) = config.optimize_autoloader {
            resolved.optimize_autoloader = optimize;
        }
        if let Some(sort) = config.sort_packages {
            resolved.sort_packages = sort;
        }
        if let Some(authoritative) = config.classmap_authoritative {
            resolved.classmap_authoritative = authoritative;
        }
        if let Some(apcu) = config.apcu_autoloader {
            resolved.apcu_autoloader = apcu;
        }
        if let Some(ref domains) = config.github_domains {
            resolved.github_domains = domains.clone();
        }
        if let Some(expose) = config.github_expose_hostname {
            resolved.github_expose_hostname = expose;
        }
        if let Some(ref domains) = config.gitlab_domains {
            resolved.gitlab_domains = domains.clone();
        }
        if let Some(use_api) = config.use_github_api {
            resolved.use_github_api = use_api;
        }
        if let Some(format) = config.archive_format {
            resolved.archive_format = format;
        }
        if let Some(ref dir) = config.archive_dir {
            resolved.archive_dir = self.resolve_path(dir);
        }
        if let Some(protect) = config.htaccess_protect {
            resolved.htaccess_protect = protect;
        }
        if let Some(lock) = config.lock {
            resolved.lock = lock;
        }
        if let Some(check) = config.platform_check {
            resolved.platform_check = check;
        }
        if let Some(secure) = config.secure_http {
            resolved.secure_http = secure;
        }
        if let Some(disable) = config.disable_tls {
            resolved.disable_tls = disable;
        }
        if let Some(ref cafile) = config.cafile {
            resolved.cafile = Some(self.resolve_path(cafile));
        }
        if let Some(ref capath) = config.capath {
            resolved.capath = Some(self.resolve_path(capath));
        }
        if let Some(discard) = config.discard_changes {
            resolved.discard_changes = discard;
        }
        if let Some(ref allow) = config.allow_plugins {
            resolved.allow_plugins = allow.clone();
        }
        if let Some(ref platform) = config.platform {
            for (k, v) in platform {
                resolved.platform.insert(k.clone(), v.clone());
            }
        }
    }

    /// Apply environment variables to resolved config.
    fn apply_env(&self, resolved: &mut ResolvedConfig) {
        if let Some(ref home) = self.env_config.home {
            resolved.home_dir = home.clone();
        }
        if let Some(ref cache) = self.env_config.cache_dir {
            resolved.cache_dir = cache.clone();
            resolved.cache_files_dir = cache.join("files");
            resolved.cache_repo_dir = cache.join("repo");
            resolved.cache_vcs_dir = cache.join("vcs");
        }
        if let Some(timeout) = self.env_config.process_timeout {
            resolved.process_timeout = timeout;
        }
        resolved.allow_superuser = self.env_config.allow_superuser;
        resolved.offline = self.env_config.disable_network;
        if let Some(ref vendor) = self.env_config.vendor_dir {
            resolved.vendor_dir = vendor.clone();
        }
        if let Some(ref bin) = self.env_config.bin_dir {
            resolved.bin_dir = bin.clone();
        }
        if let Some(protect) = self.env_config.htaccess_protect {
            resolved.htaccess_protect = protect;
        }
        resolved.http_proxy = self.env_config.http_proxy.clone();
        resolved.https_proxy = self.env_config.https_proxy.clone();
        resolved.no_proxy = self.env_config.no_proxy.clone();
    }

    /// Resolve a path relative to project directory.
    fn resolve_path(&self, path: &str) -> PathBuf {
        let path = PathBuf::from(path);
        if path.is_absolute() {
            path
        } else {
            self.project_dir.join(path)
        }
    }

    /// Get environment configuration.
    #[must_use]
    pub const fn env(&self) -> &EnvConfig {
        &self.env_config
    }

    /// Get project directory.
    #[must_use]
    pub fn project_dir(&self) -> &Path {
        &self.project_dir
    }

    /// Check if project manifest exists.
    #[must_use]
    pub fn has_manifest(&self) -> bool {
        self.project_manifest_path().exists()
    }
}

/// CLI configuration overrides.
#[derive(Debug, Clone, Default)]
pub struct CliOverrides {
    /// Working directory override.
    pub working_dir: Option<PathBuf>,
    /// Disable cache.
    pub no_cache: bool,
    /// Disable network.
    pub offline: bool,
    /// Disable plugins.
    pub no_plugins: bool,
    /// Non-interactive mode.
    pub no_interaction: bool,
    /// Prefer source installation.
    pub prefer_source: bool,
    /// Prefer dist installation.
    pub prefer_dist: bool,
    /// Optimize autoloader.
    pub optimize_autoloader: bool,
    /// Authoritative classmap.
    pub classmap_authoritative: bool,
    /// `APCu` autoloader.
    pub apcu_autoloader: bool,
    /// Ignore platform requirements.
    pub ignore_platform_reqs: bool,
    /// Dev mode.
    pub dev: bool,
    /// No dev mode.
    pub no_dev: bool,
    /// Verbose output.
    pub verbose: u8,
    /// Quiet output.
    pub quiet: bool,
    /// ANSI output.
    pub ansi: Option<bool>,
}

impl CliOverrides {
    /// Apply CLI overrides to resolved config.
    pub fn apply_to(&self, resolved: &mut ResolvedConfig) {
        if let Some(ref dir) = self.working_dir {
            resolved.project_dir = dir.clone();
        }
        if self.offline {
            resolved.offline = true;
        }
        if self.no_plugins {
            resolved.allow_plugins = crate::types::AllowPlugins::Global(false);
        }
        if self.prefer_source {
            resolved.preferred_install = crate::types::PreferredInstallConfig::Global(
                crate::types::PreferredInstall::Source,
            );
        }
        if self.prefer_dist {
            resolved.preferred_install =
                crate::types::PreferredInstallConfig::Global(crate::types::PreferredInstall::Dist);
        }
        if self.optimize_autoloader {
            resolved.optimize_autoloader = true;
        }
        if self.classmap_authoritative {
            resolved.classmap_authoritative = true;
        }
        if self.apcu_autoloader {
            resolved.apcu_autoloader = true;
        }
        if self.ignore_platform_reqs {
            resolved.platform_check = crate::types::PlatformCheck::Disabled;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_loader_paths() {
        let loader = ConfigLoader::new("/tmp/test-project");
        assert!(loader.project_manifest_path().ends_with("composer.json"));
        assert!(loader.project_auth_path().ends_with("auth.json"));
    }

    #[test]
    fn resolved_config_defaults() {
        let config = ResolvedConfig::default();
        assert_eq!(config.process_timeout, 300);
        assert!(config.secure_http);
        assert!(!config.disable_tls);
    }

    #[test]
    fn cli_overrides_apply() {
        let mut config = ResolvedConfig::default();
        let overrides = CliOverrides {
            offline: true,
            optimize_autoloader: true,
            ..Default::default()
        };
        overrides.apply_to(&mut config);
        assert!(config.offline);
        assert!(config.optimize_autoloader);
    }
}
