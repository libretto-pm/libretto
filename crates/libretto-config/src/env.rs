//! Environment variable configuration support.

use crate::error::{ConfigError, Result};
use crate::types::ComposerConfig;
use std::path::PathBuf;

/// Well-known Composer environment variables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ComposerEnvVar {
    /// `COMPOSER_HOME` - global configuration directory.
    Home,
    /// `COMPOSER_CACHE_DIR` - cache directory.
    CacheDir,
    /// `COMPOSER_PROCESS_TIMEOUT` - process timeout in seconds.
    ProcessTimeout,
    /// `COMPOSER_ALLOW_SUPERUSER` - allow running as root.
    AllowSuperuser,
    /// `COMPOSER_AUTH` - inline auth.json content.
    Auth,
    /// `COMPOSER_DISABLE_NETWORK` - disable network access.
    DisableNetwork,
    /// `COMPOSER_NO_INTERACTION` - non-interactive mode.
    NoInteraction,
    /// `COMPOSER_VENDOR_DIR` - vendor directory.
    VendorDir,
    /// `COMPOSER_BIN_DIR` - binaries directory.
    BinDir,
    /// `COMPOSER_HTACCESS_PROTECT` - protect vendor with .htaccess.
    HtaccessProtect,
    /// `COMPOSER_MEMORY_LIMIT` - memory limit override.
    MemoryLimit,
    /// `COMPOSER_MIRROR_PATH_REPOS` - mirror path repositories.
    MirrorPathRepos,
    /// `COMPOSER_ROOT_VERSION` - root package version.
    RootVersion,
    /// `COMPOSER_DISABLE_XDEBUG_WARN` - disable Xdebug warning.
    DisableXdebugWarn,
    /// `COMPOSER_FUND` - enable funding messages.
    Fund,
    /// `COMPOSER_AUDIT_ABANDONED` - abandoned package handling.
    AuditAbandoned,
    /// COMPOSER - path to composer.json.
    Composer,
    /// `COMPOSER_ORIGINAL_INIS` - original PHP ini files.
    OriginalInis,
    /// `COMPOSER_RUNTIME_ENV` - runtime environment.
    RuntimeEnv,
    /// `HTTP_PROXY` - HTTP proxy.
    HttpProxy,
    /// `HTTPS_PROXY` - HTTPS proxy.
    HttpsProxy,
    /// `NO_PROXY` - no-proxy hosts.
    NoProxy,
    /// `http_proxy` - HTTP proxy (lowercase).
    HttpProxyLower,
    /// `https_proxy` - HTTPS proxy (lowercase).
    HttpsProxyLower,
    /// `no_proxy` - no-proxy hosts (lowercase).
    NoProxyLower,
    /// `CGI_HTTP_PROXY` - CGI HTTP proxy.
    CgiHttpProxy,
}

impl ComposerEnvVar {
    /// Get the environment variable name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Home => "COMPOSER_HOME",
            Self::CacheDir => "COMPOSER_CACHE_DIR",
            Self::ProcessTimeout => "COMPOSER_PROCESS_TIMEOUT",
            Self::AllowSuperuser => "COMPOSER_ALLOW_SUPERUSER",
            Self::Auth => "COMPOSER_AUTH",
            Self::DisableNetwork => "COMPOSER_DISABLE_NETWORK",
            Self::NoInteraction => "COMPOSER_NO_INTERACTION",
            Self::VendorDir => "COMPOSER_VENDOR_DIR",
            Self::BinDir => "COMPOSER_BIN_DIR",
            Self::HtaccessProtect => "COMPOSER_HTACCESS_PROTECT",
            Self::MemoryLimit => "COMPOSER_MEMORY_LIMIT",
            Self::MirrorPathRepos => "COMPOSER_MIRROR_PATH_REPOS",
            Self::RootVersion => "COMPOSER_ROOT_VERSION",
            Self::DisableXdebugWarn => "COMPOSER_DISABLE_XDEBUG_WARN",
            Self::Fund => "COMPOSER_FUND",
            Self::AuditAbandoned => "COMPOSER_AUDIT_ABANDONED",
            Self::Composer => "COMPOSER",
            Self::OriginalInis => "COMPOSER_ORIGINAL_INIS",
            Self::RuntimeEnv => "COMPOSER_RUNTIME_ENV",
            Self::HttpProxy => "HTTP_PROXY",
            Self::HttpsProxy => "HTTPS_PROXY",
            Self::NoProxy => "NO_PROXY",
            Self::HttpProxyLower => "http_proxy",
            Self::HttpsProxyLower => "https_proxy",
            Self::NoProxyLower => "no_proxy",
            Self::CgiHttpProxy => "CGI_HTTP_PROXY",
        }
    }

    /// Get the value from environment.
    #[must_use]
    pub fn get(self) -> Option<String> {
        std::env::var(self.as_str()).ok()
    }

    /// Check if the variable is set.
    #[must_use]
    pub fn is_set(self) -> bool {
        std::env::var(self.as_str()).is_ok()
    }

    /// Get as boolean (1/true/yes/on = true).
    #[must_use]
    pub fn as_bool(self) -> Option<bool> {
        self.get()
            .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "yes" | "on"))
    }

    /// Get as u32.
    #[must_use]
    pub fn as_u32(self) -> Option<u32> {
        self.get().and_then(|v| v.parse().ok())
    }

    /// Get as path.
    #[must_use]
    pub fn as_path(self) -> Option<PathBuf> {
        self.get().map(PathBuf::from)
    }
}

/// Environment configuration reader.
#[derive(Debug, Default)]
pub struct EnvConfig {
    /// `COMPOSER_HOME` directory.
    pub home: Option<PathBuf>,
    /// `COMPOSER_CACHE_DIR` directory.
    pub cache_dir: Option<PathBuf>,
    /// Process timeout in seconds.
    pub process_timeout: Option<u32>,
    /// Allow running as root.
    pub allow_superuser: bool,
    /// Inline auth.json content.
    pub auth: Option<String>,
    /// Disable network access.
    pub disable_network: bool,
    /// Non-interactive mode.
    pub no_interaction: bool,
    /// Vendor directory.
    pub vendor_dir: Option<PathBuf>,
    /// Binaries directory.
    pub bin_dir: Option<PathBuf>,
    /// Protect vendor with .htaccess.
    pub htaccess_protect: Option<bool>,
    /// Memory limit.
    pub memory_limit: Option<String>,
    /// Mirror path repositories.
    pub mirror_path_repos: bool,
    /// Root package version.
    pub root_version: Option<String>,
    /// Disable Xdebug warning.
    pub disable_xdebug_warn: bool,
    /// Enable funding messages.
    pub fund: Option<bool>,
    /// Abandoned package handling.
    pub audit_abandoned: Option<String>,
    /// Path to composer.json.
    pub composer: Option<PathBuf>,
    /// HTTP proxy.
    pub http_proxy: Option<String>,
    /// HTTPS proxy.
    pub https_proxy: Option<String>,
    /// No-proxy hosts.
    pub no_proxy: Option<String>,
}

impl EnvConfig {
    /// Read configuration from environment variables.
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            home: ComposerEnvVar::Home.as_path(),
            cache_dir: ComposerEnvVar::CacheDir.as_path(),
            process_timeout: ComposerEnvVar::ProcessTimeout.as_u32(),
            allow_superuser: ComposerEnvVar::AllowSuperuser.as_bool().unwrap_or(false),
            auth: ComposerEnvVar::Auth.get(),
            disable_network: ComposerEnvVar::DisableNetwork.as_bool().unwrap_or(false),
            no_interaction: ComposerEnvVar::NoInteraction.as_bool().unwrap_or(false),
            vendor_dir: ComposerEnvVar::VendorDir.as_path(),
            bin_dir: ComposerEnvVar::BinDir.as_path(),
            htaccess_protect: ComposerEnvVar::HtaccessProtect.as_bool(),
            memory_limit: ComposerEnvVar::MemoryLimit.get(),
            mirror_path_repos: ComposerEnvVar::MirrorPathRepos.as_bool().unwrap_or(false),
            root_version: ComposerEnvVar::RootVersion.get(),
            disable_xdebug_warn: ComposerEnvVar::DisableXdebugWarn.as_bool().unwrap_or(false),
            fund: ComposerEnvVar::Fund.as_bool(),
            audit_abandoned: ComposerEnvVar::AuditAbandoned.get(),
            composer: ComposerEnvVar::Composer.as_path(),
            http_proxy: Self::get_http_proxy(),
            https_proxy: Self::get_https_proxy(),
            no_proxy: Self::get_no_proxy(),
        }
    }

    /// Get HTTP proxy from environment (tries multiple variables).
    fn get_http_proxy() -> Option<String> {
        ComposerEnvVar::HttpProxy
            .get()
            .or_else(|| ComposerEnvVar::HttpProxyLower.get())
            .or_else(|| ComposerEnvVar::CgiHttpProxy.get())
    }

    /// Get HTTPS proxy from environment.
    fn get_https_proxy() -> Option<String> {
        ComposerEnvVar::HttpsProxy
            .get()
            .or_else(|| ComposerEnvVar::HttpsProxyLower.get())
    }

    /// Get no-proxy hosts from environment.
    fn get_no_proxy() -> Option<String> {
        ComposerEnvVar::NoProxy
            .get()
            .or_else(|| ComposerEnvVar::NoProxyLower.get())
    }

    /// Check if running in non-interactive mode.
    #[must_use]
    pub const fn is_non_interactive(&self) -> bool {
        self.no_interaction
    }

    /// Check if network is disabled.
    #[must_use]
    pub const fn is_offline(&self) -> bool {
        self.disable_network
    }

    /// Parse inline auth JSON.
    ///
    /// # Errors
    /// Returns error if auth JSON is invalid.
    pub fn parse_auth(&self) -> Result<Option<crate::auth::AuthConfig>> {
        match &self.auth {
            Some(json) => {
                let config: crate::auth::AuthConfig =
                    sonic_rs::from_str(json).map_err(|e| ConfigError::EnvError {
                        var: "COMPOSER_AUTH".to_string(),
                        message: format!("invalid JSON: {e}"),
                    })?;
                Ok(Some(config))
            }
            None => Ok(None),
        }
    }

    /// Apply environment overrides to composer config.
    pub fn apply_to(&self, config: &mut ComposerConfig) {
        if let Some(timeout) = self.process_timeout {
            config.process_timeout = Some(timeout);
        }
        if let Some(ref vendor) = self.vendor_dir {
            config.vendor_dir = Some(vendor.to_string_lossy().to_string());
        }
        if let Some(ref bin) = self.bin_dir {
            config.bin_dir = Some(bin.to_string_lossy().to_string());
        }
        if let Some(protect) = self.htaccess_protect {
            config.htaccess_protect = Some(protect);
        }
    }
}

/// Parse byte size string (e.g., "300MiB", "1G", "500M").
///
/// # Errors
/// Returns error if the size string is invalid.
pub fn parse_byte_size(s: &str) -> Result<u64> {
    let s = s.trim();

    // Handle numeric-only input
    if let Ok(bytes) = s.parse::<u64>() {
        return Ok(bytes);
    }

    // Find where the number ends
    let num_end = s
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .count();
    if num_end == 0 {
        return Err(ConfigError::invalid_value(
            "size",
            "invalid byte size",
            "use format like '300MiB' or '1G'",
        ));
    }

    let (num_str, unit) = s.split_at(num_end);
    let num: f64 = num_str
        .parse()
        .map_err(|_| ConfigError::invalid_value("size", "invalid number", "use a valid number"))?;

    let unit = unit.trim().to_lowercase();
    let multiplier: u64 = match unit.as_str() {
        "" | "b" => 1,
        "k" | "kb" | "kib" => 1024,
        "m" | "mb" | "mib" => 1024 * 1024,
        "g" | "gb" | "gib" => 1024 * 1024 * 1024,
        "t" | "tb" | "tib" => 1024 * 1024 * 1024 * 1024,
        _ => {
            return Err(ConfigError::invalid_value(
                "size",
                format!("unknown unit: {unit}"),
                "use B, K, M, G, or T",
            ));
        }
    };

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok((num * multiplier as f64) as u64)
}

/// Parse duration string (e.g., "300", "5m", "1h").
///
/// # Errors
/// Returns error if the duration string is invalid.
pub fn parse_duration_secs(s: &str) -> Result<u32> {
    let s = s.trim();

    // Handle numeric-only input (assume seconds)
    if let Ok(secs) = s.parse::<u32>() {
        return Ok(secs);
    }

    // Find where the number ends
    let num_end = s
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .count();
    if num_end == 0 {
        return Err(ConfigError::invalid_value(
            "duration",
            "invalid duration",
            "use format like '300' or '5m'",
        ));
    }

    let (num_str, unit) = s.split_at(num_end);
    let num: f64 = num_str.parse().map_err(|_| {
        ConfigError::invalid_value("duration", "invalid number", "use a valid number")
    })?;

    let unit = unit.trim().to_lowercase();
    let multiplier: u32 = match unit.as_str() {
        "" | "s" | "sec" | "secs" | "second" | "seconds" => 1,
        "m" | "min" | "mins" | "minute" | "minutes" => 60,
        "h" | "hr" | "hrs" | "hour" | "hours" => 3600,
        "d" | "day" | "days" => 86400,
        _ => {
            return Err(ConfigError::invalid_value(
                "duration",
                format!("unknown unit: {unit}"),
                "use s, m, h, or d",
            ));
        }
    };

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok((num * f64::from(multiplier)) as u32)
}

/// Check if running as root/superuser.
#[must_use]
pub fn is_superuser() -> bool {
    #[cfg(unix)]
    {
        // Use nix crate which is already available via libretto-platform
        nix::unistd::geteuid().is_root()
    }
    #[cfg(windows)]
    {
        // On Windows, check for elevated privileges
        // This is a simplified check
        std::env::var("USERNAME").is_ok_and(|u| u.eq_ignore_ascii_case("Administrator"))
    }
    #[cfg(not(any(unix, windows)))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_byte_size_numeric() {
        assert_eq!(parse_byte_size("1024").unwrap(), 1024);
        assert_eq!(parse_byte_size("0").unwrap(), 0);
    }

    #[test]
    fn parse_byte_size_units() {
        assert_eq!(parse_byte_size("1K").unwrap(), 1024);
        assert_eq!(parse_byte_size("1M").unwrap(), 1024 * 1024);
        assert_eq!(parse_byte_size("1G").unwrap(), 1024 * 1024 * 1024);
        assert_eq!(parse_byte_size("300MiB").unwrap(), 300 * 1024 * 1024);
        assert_eq!(
            parse_byte_size("1.5G").unwrap(),
            (1.5 * 1024.0 * 1024.0 * 1024.0) as u64
        );
    }

    #[test]
    fn parse_duration_numeric() {
        assert_eq!(parse_duration_secs("300").unwrap(), 300);
        assert_eq!(parse_duration_secs("0").unwrap(), 0);
    }

    #[test]
    fn parse_duration_units() {
        assert_eq!(parse_duration_secs("5m").unwrap(), 300);
        assert_eq!(parse_duration_secs("1h").unwrap(), 3600);
        assert_eq!(parse_duration_secs("1d").unwrap(), 86400);
        assert_eq!(parse_duration_secs("1.5h").unwrap(), 5400);
    }

    #[test]
    fn env_var_names() {
        assert_eq!(ComposerEnvVar::Home.as_str(), "COMPOSER_HOME");
        assert_eq!(
            ComposerEnvVar::ProcessTimeout.as_str(),
            "COMPOSER_PROCESS_TIMEOUT"
        );
    }
}
