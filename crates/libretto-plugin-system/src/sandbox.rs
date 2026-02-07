//! Plugin sandboxing and security.
//!
//! This module provides security features for plugins:
//! - File system access restrictions
//! - Network call monitoring
//! - Timeout enforcement
//! - Memory limits

use crate::error::{PluginError, Result};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::time::timeout;
use tracing::warn;

/// Default timeout for plugin operations.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Default memory limit (256 MB).
pub const DEFAULT_MEMORY_LIMIT: usize = 256 * 1024 * 1024;

/// Sandbox configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxConfig {
    /// Enable sandbox (default: true).
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// Default timeout for operations.
    #[serde(default = "default_timeout", with = "duration_serde")]
    pub timeout: Duration,

    /// Memory limit in bytes.
    #[serde(default = "default_memory_limit")]
    pub memory_limit: usize,

    /// Allowed file system paths (read access).
    #[serde(default)]
    pub allowed_read_paths: Vec<PathBuf>,

    /// Allowed file system paths (write access).
    #[serde(default)]
    pub allowed_write_paths: Vec<PathBuf>,

    /// Allowed network hosts.
    #[serde(default)]
    pub allowed_hosts: Vec<String>,

    /// Block all network access.
    #[serde(default)]
    pub block_network: bool,

    /// Allow executing external commands.
    #[serde(default)]
    pub allow_exec: bool,

    /// Allowed commands (if `allow_exec` is true).
    #[serde(default)]
    pub allowed_commands: Vec<String>,

    /// Plugin-specific overrides.
    #[serde(default)]
    pub plugin_overrides: std::collections::HashMap<String, SandboxOverride>,
}

const fn default_enabled() -> bool {
    true
}

const fn default_timeout() -> Duration {
    DEFAULT_TIMEOUT
}

const fn default_memory_limit() -> usize {
    DEFAULT_MEMORY_LIMIT
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            timeout: DEFAULT_TIMEOUT,
            memory_limit: DEFAULT_MEMORY_LIMIT,
            allowed_read_paths: Vec::new(),
            allowed_write_paths: Vec::new(),
            allowed_hosts: Vec::new(),
            block_network: false,
            allow_exec: false,
            allowed_commands: Vec::new(),
            plugin_overrides: std::collections::HashMap::new(),
        }
    }
}

/// Per-plugin sandbox overrides.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SandboxOverride {
    /// Override timeout.
    #[serde(default, with = "option_duration_serde")]
    pub timeout: Option<Duration>,

    /// Override memory limit.
    pub memory_limit: Option<usize>,

    /// Additional allowed read paths.
    #[serde(default)]
    pub additional_read_paths: Vec<PathBuf>,

    /// Additional allowed write paths.
    #[serde(default)]
    pub additional_write_paths: Vec<PathBuf>,

    /// Additional allowed hosts.
    #[serde(default)]
    pub additional_hosts: Vec<String>,

    /// Disable sandbox for this plugin (use with caution).
    #[serde(default)]
    pub disable: bool,
}

/// Sandbox violation record.
#[derive(Debug, Clone)]
pub struct SandboxViolation {
    /// Plugin that caused the violation.
    pub plugin_id: String,
    /// Type of violation.
    pub violation_type: ViolationType,
    /// Details about the violation.
    pub details: String,
    /// Timestamp.
    pub timestamp: Instant,
}

/// Type of sandbox violation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViolationType {
    /// Attempted to access a restricted path.
    FileSystemAccess,
    /// Attempted to write to a restricted path.
    FileSystemWrite,
    /// Attempted to connect to a blocked host.
    NetworkAccess,
    /// Operation timed out.
    Timeout,
    /// Exceeded memory limit.
    MemoryLimit,
    /// Attempted to execute a blocked command.
    CommandExecution,
}

impl std::fmt::Display for ViolationType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FileSystemAccess => write!(f, "filesystem_access"),
            Self::FileSystemWrite => write!(f, "filesystem_write"),
            Self::NetworkAccess => write!(f, "network_access"),
            Self::Timeout => write!(f, "timeout"),
            Self::MemoryLimit => write!(f, "memory_limit"),
            Self::CommandExecution => write!(f, "command_execution"),
        }
    }
}

/// Plugin sandbox for security enforcement.
#[derive(Debug)]
pub struct Sandbox {
    /// Configuration.
    config: SandboxConfig,
    /// Recorded violations.
    violations: DashMap<String, Vec<SandboxViolation>>,
    /// Active operations tracking.
    active_operations: DashMap<String, Instant>,
    /// Network access log.
    network_log: DashMap<String, Vec<NetworkAccess>>,
    /// File access log.
    file_log: DashMap<String, Vec<FileAccess>>,
}

/// Network access record.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct NetworkAccess {
    /// Target host.
    pub host: String,
    /// Port.
    pub port: u16,
    /// Timestamp.
    pub timestamp: Instant,
    /// Whether it was allowed.
    pub allowed: bool,
}

/// File access record.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct FileAccess {
    /// Path accessed.
    pub path: PathBuf,
    /// Access type (read/write).
    pub access_type: FileAccessType,
    /// Timestamp.
    pub timestamp: Instant,
    /// Whether it was allowed.
    pub allowed: bool,
}

/// File access type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileAccessType {
    Read,
    Write,
    Execute,
}

impl Sandbox {
    /// Create a new sandbox with the given configuration.
    #[must_use]
    pub fn new(config: SandboxConfig) -> Self {
        Self {
            config,
            violations: DashMap::new(),
            active_operations: DashMap::new(),
            network_log: DashMap::new(),
            file_log: DashMap::new(),
        }
    }

    /// Execute a future with timeout enforcement.
    ///
    /// # Errors
    /// Returns error if the operation times out.
    pub async fn execute_with_timeout<F, T>(&self, duration: Duration, future: F) -> Result<T>
    where
        F: Future<Output = Result<T>>,
    {
        timeout(duration, future)
            .await
            .map_err(|_| PluginError::Timeout {
                plugin: String::new(),
                seconds: duration.as_secs(),
            })?
    }

    /// Check if a file path is allowed for reading.
    #[must_use]
    pub fn is_read_allowed(&self, plugin_id: &str, path: &Path) -> bool {
        if !self.config.enabled {
            return true;
        }

        // Check plugin-specific override
        if let Some(override_config) = self.config.plugin_overrides.get(plugin_id) {
            if override_config.disable {
                return true;
            }

            // Check additional paths
            for allowed in &override_config.additional_read_paths {
                if path.starts_with(allowed) {
                    return true;
                }
            }
        }

        // Check global allowed paths
        for allowed in &self.config.allowed_read_paths {
            if path.starts_with(allowed) {
                return true;
            }
        }

        // Default: allow reads within project/vendor directories
        // (this should be configured properly in real usage)
        false
    }

    /// Check if a file path is allowed for writing.
    #[must_use]
    pub fn is_write_allowed(&self, plugin_id: &str, path: &Path) -> bool {
        if !self.config.enabled {
            return true;
        }

        // Check plugin-specific override
        if let Some(override_config) = self.config.plugin_overrides.get(plugin_id) {
            if override_config.disable {
                return true;
            }

            // Check additional paths
            for allowed in &override_config.additional_write_paths {
                if path.starts_with(allowed) {
                    return true;
                }
            }
        }

        // Check global allowed paths
        for allowed in &self.config.allowed_write_paths {
            if path.starts_with(allowed) {
                return true;
            }
        }

        false
    }

    /// Check if network access to a host is allowed.
    #[must_use]
    pub fn is_network_allowed(&self, plugin_id: &str, host: &str) -> bool {
        if !self.config.enabled {
            return true;
        }

        if self.config.block_network {
            return false;
        }

        // Check plugin-specific override
        if let Some(override_config) = self.config.plugin_overrides.get(plugin_id) {
            if override_config.disable {
                return true;
            }

            // Check additional hosts
            for allowed in &override_config.additional_hosts {
                if Self::host_matches(host, allowed) {
                    return true;
                }
            }
        }

        // Check global allowed hosts
        for allowed in &self.config.allowed_hosts {
            if Self::host_matches(host, allowed) {
                return true;
            }
        }

        // If no hosts are configured, allow all (unless block_network is true)
        self.config.allowed_hosts.is_empty()
    }

    /// Check if a command is allowed to execute.
    #[must_use]
    pub fn is_command_allowed(&self, plugin_id: &str, command: &str) -> bool {
        if !self.config.enabled {
            return true;
        }

        if !self.config.allow_exec {
            return false;
        }

        // Check plugin-specific override
        if let Some(override_config) = self.config.plugin_overrides.get(plugin_id)
            && override_config.disable
        {
            return true;
        }

        // Check allowed commands
        for allowed in &self.config.allowed_commands {
            if command.starts_with(allowed) {
                return true;
            }
        }

        // If no commands are configured but exec is allowed, allow all
        self.config.allowed_commands.is_empty()
    }

    /// Record a file access.
    pub fn record_file_access(
        &self,
        plugin_id: &str,
        path: &Path,
        access_type: FileAccessType,
        allowed: bool,
    ) {
        let access = FileAccess {
            path: path.to_path_buf(),
            access_type,
            timestamp: Instant::now(),
            allowed,
        };

        self.file_log
            .entry(plugin_id.to_string())
            .or_default()
            .push(access);

        if !allowed {
            self.record_violation(
                plugin_id,
                match access_type {
                    FileAccessType::Read | FileAccessType::Execute => {
                        ViolationType::FileSystemAccess
                    }
                    FileAccessType::Write => ViolationType::FileSystemWrite,
                },
                format!(
                    "attempted {} access to {}",
                    access_type_str(access_type),
                    path.display()
                ),
            );
        }
    }

    /// Record a network access.
    pub fn record_network_access(&self, plugin_id: &str, host: &str, port: u16, allowed: bool) {
        let access = NetworkAccess {
            host: host.to_string(),
            port,
            timestamp: Instant::now(),
            allowed,
        };

        self.network_log
            .entry(plugin_id.to_string())
            .or_default()
            .push(access);

        if !allowed {
            self.record_violation(
                plugin_id,
                ViolationType::NetworkAccess,
                format!("attempted connection to {host}:{port}"),
            );
        }
    }

    /// Record a violation.
    pub fn record_violation(
        &self,
        plugin_id: &str,
        violation_type: ViolationType,
        details: String,
    ) {
        let violation = SandboxViolation {
            plugin_id: plugin_id.to_string(),
            violation_type,
            details: details.clone(),
            timestamp: Instant::now(),
        };

        warn!(
            plugin = %plugin_id,
            violation = %violation_type,
            details = %details,
            "sandbox violation"
        );

        self.violations
            .entry(plugin_id.to_string())
            .or_default()
            .push(violation);
    }

    /// Get violations for a plugin.
    #[must_use]
    pub fn get_violations(&self, plugin_id: &str) -> Vec<SandboxViolation> {
        self.violations
            .get(plugin_id)
            .map(|v| v.clone())
            .unwrap_or_default()
    }

    /// Get all violations.
    #[must_use]
    pub fn all_violations(&self) -> Vec<SandboxViolation> {
        self.violations
            .iter()
            .flat_map(|entry| entry.value().clone())
            .collect()
    }

    /// Check if a plugin has violations.
    #[must_use]
    pub fn has_violations(&self, plugin_id: &str) -> bool {
        self.violations
            .get(plugin_id)
            .is_some_and(|v| !v.is_empty())
    }

    /// Clear violations for a plugin.
    pub fn clear_violations(&self, plugin_id: &str) {
        self.violations.remove(plugin_id);
    }

    /// Get timeout for a plugin.
    #[must_use]
    pub fn get_timeout(&self, plugin_id: &str) -> Duration {
        if let Some(override_config) = self.config.plugin_overrides.get(plugin_id)
            && let Some(timeout) = override_config.timeout
        {
            return timeout;
        }
        self.config.timeout
    }

    /// Get memory limit for a plugin.
    #[must_use]
    pub fn get_memory_limit(&self, plugin_id: &str) -> usize {
        if let Some(override_config) = self.config.plugin_overrides.get(plugin_id)
            && let Some(limit) = override_config.memory_limit
        {
            return limit;
        }
        self.config.memory_limit
    }

    /// Start tracking an operation.
    pub fn start_operation(&self, plugin_id: &str) {
        self.active_operations
            .insert(plugin_id.to_string(), Instant::now());
    }

    /// End tracking an operation.
    #[must_use]
    pub fn end_operation(&self, plugin_id: &str) -> Option<Duration> {
        self.active_operations
            .remove(plugin_id)
            .map(|(_, start)| start.elapsed())
    }

    /// Check if any operation has exceeded its timeout.
    #[must_use]
    pub fn check_timeouts(&self) -> Vec<String> {
        let now = Instant::now();
        self.active_operations
            .iter()
            .filter(|entry| now.duration_since(*entry.value()) > self.get_timeout(entry.key()))
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// Helper to check if a host matches a pattern.
    fn host_matches(host: &str, pattern: &str) -> bool {
        if let Some(suffix) = pattern.strip_prefix('*') {
            // Wildcard pattern (e.g., *.example.com)
            host.ends_with(suffix)
        } else {
            host == pattern
        }
    }
}

const fn access_type_str(access_type: FileAccessType) -> &'static str {
    match access_type {
        FileAccessType::Read => "read",
        FileAccessType::Write => "write",
        FileAccessType::Execute => "execute",
    }
}

/// Duration serialization helpers.
mod duration_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        duration.as_secs().serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let secs = u64::deserialize(deserializer)?;
        Ok(Duration::from_secs(secs))
    }
}

mod option_duration_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Option<Duration>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        duration.map(|d| d.as_secs()).serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Duration>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let secs = Option::<u64>::deserialize(deserializer)?;
        Ok(secs.map(Duration::from_secs))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_creation() {
        let config = SandboxConfig::default();
        let sandbox = Sandbox::new(config);

        assert!(sandbox.all_violations().is_empty());
    }

    #[test]
    fn file_access_check() {
        let sandbox = Sandbox::new(SandboxConfig {
            allowed_read_paths: vec![PathBuf::from("/project")],
            allowed_write_paths: vec![PathBuf::from("/project/vendor")],
            ..Default::default()
        });

        assert!(sandbox.is_read_allowed("test", Path::new("/project/src/file.php")));
        assert!(!sandbox.is_read_allowed("test", Path::new("/etc/passwd")));

        assert!(sandbox.is_write_allowed("test", Path::new("/project/vendor/autoload.php")));
        assert!(!sandbox.is_write_allowed("test", Path::new("/project/src/file.php")));
    }

    #[test]
    fn network_access_check() {
        let sandbox = Sandbox::new(SandboxConfig {
            allowed_hosts: vec!["packagist.org".to_string(), "*.github.com".to_string()],
            ..Default::default()
        });

        assert!(sandbox.is_network_allowed("test", "packagist.org"));
        assert!(sandbox.is_network_allowed("test", "api.github.com"));
        assert!(!sandbox.is_network_allowed("test", "evil.com"));
    }

    #[test]
    fn network_blocked() {
        let sandbox = Sandbox::new(SandboxConfig {
            block_network: true,
            ..Default::default()
        });

        assert!(!sandbox.is_network_allowed("test", "packagist.org"));
    }

    #[test]
    fn command_execution_check() {
        let sandbox = Sandbox::new(SandboxConfig {
            allow_exec: true,
            allowed_commands: vec!["php".to_string(), "composer".to_string()],
            ..Default::default()
        });

        assert!(sandbox.is_command_allowed("test", "php script.php"));
        assert!(sandbox.is_command_allowed("test", "composer install"));
        assert!(!sandbox.is_command_allowed("test", "rm -rf /"));
    }

    #[test]
    fn plugin_override() {
        let mut config = SandboxConfig {
            allowed_hosts: vec!["packagist.org".to_string()],
            ..Default::default()
        };

        let override_config = SandboxOverride {
            additional_hosts: vec!["special.example.com".to_string()],
            ..Default::default()
        };
        config
            .plugin_overrides
            .insert("special-plugin".to_string(), override_config);

        let sandbox = Sandbox::new(config);

        // Regular plugin can't access special host
        assert!(!sandbox.is_network_allowed("regular", "special.example.com"));

        // Special plugin can access it
        assert!(sandbox.is_network_allowed("special-plugin", "special.example.com"));
    }

    #[test]
    fn violation_recording() {
        let sandbox = Sandbox::new(SandboxConfig::default());

        sandbox.record_violation("test-plugin", ViolationType::NetworkAccess, "test".into());

        assert!(sandbox.has_violations("test-plugin"));
        assert_eq!(sandbox.get_violations("test-plugin").len(), 1);
        assert!(!sandbox.has_violations("other-plugin"));
    }

    #[test]
    fn timeout_tracking() {
        let sandbox = Sandbox::new(SandboxConfig {
            timeout: Duration::from_millis(100),
            ..Default::default()
        });

        sandbox.start_operation("test");

        // Should not timeout immediately
        assert!(sandbox.check_timeouts().is_empty());

        // Simulate timeout (in real usage, this would be checked periodically)
        std::thread::sleep(Duration::from_millis(150));
        let timeouts = sandbox.check_timeouts();
        assert!(timeouts.contains(&"test".to_string()));
    }

    #[tokio::test]
    async fn execute_with_timeout() {
        let sandbox = Sandbox::new(SandboxConfig::default());

        // Should succeed
        let result = sandbox
            .execute_with_timeout(Duration::from_secs(1), async { Ok(42) })
            .await;
        assert_eq!(result.unwrap(), 42);

        // Should timeout
        let result: Result<i32> = sandbox
            .execute_with_timeout(Duration::from_millis(10), async {
                tokio::time::sleep(Duration::from_secs(1)).await;
                Ok(42)
            })
            .await;
        assert!(result.is_err());
    }

    #[test]
    fn host_matching() {
        assert!(Sandbox::host_matches("packagist.org", "packagist.org"));
        assert!(!Sandbox::host_matches("packagist.org", "other.org"));

        // Wildcard matching
        assert!(Sandbox::host_matches("api.github.com", "*.github.com"));
        assert!(Sandbox::host_matches("raw.github.com", "*.github.com"));
        assert!(!Sandbox::host_matches("github.com", "*.github.com")); // No subdomain
    }
}
