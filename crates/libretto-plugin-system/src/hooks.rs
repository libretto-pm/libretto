//! Composer-compatible hooks system.
//!
//! This module defines all the hooks that plugins can register for, matching
//! Composer's plugin event system for full compatibility.

use dashmap::DashMap;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

/// Hook priority levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum HookPriority {
    /// Highest priority (executed first).
    Highest,
    /// High priority.
    High,
    /// Normal priority (default).
    #[default]
    Normal,
    /// Low priority.
    Low,
    /// Lowest priority (executed last).
    Lowest,
    /// Custom priority value.
    Custom(i32),
}

impl HookPriority {
    /// Convert to numeric value for sorting.
    #[must_use]
    pub const fn as_value(&self) -> i32 {
        match self {
            Self::Highest => -1000,
            Self::High => -100,
            Self::Normal => 0,
            Self::Low => 100,
            Self::Lowest => 1000,
            Self::Custom(v) => *v,
        }
    }
}

impl From<i32> for HookPriority {
    fn from(value: i32) -> Self {
        match value {
            v if v <= -500 => Self::Highest,
            v if v <= -50 => Self::High,
            v if v <= 50 => Self::Normal,
            v if v <= 500 => Self::Low,
            v if v > 500 => Self::Lowest,
            _ => Self::Custom(value),
        }
    }
}

/// Composer-compatible hook events.
///
/// These match the events defined in Composer's plugin system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Hook {
    // Command events
    /// Before `install` command runs.
    PreInstallCmd,
    /// After `install` command completes.
    PostInstallCmd,
    /// Before `update` command runs.
    PreUpdateCmd,
    /// After `update` command completes.
    PostUpdateCmd,
    /// Before `status` command runs.
    PreStatusCmd,
    /// After `status` command completes.
    PostStatusCmd,

    // Autoload events
    /// Before autoload dump.
    PreAutoloadDump,
    /// After autoload dump.
    PostAutoloadDump,

    // Package events
    /// Before a package is installed.
    PrePackageInstall,
    /// After a package is installed.
    PostPackageInstall,
    /// Before a package is updated.
    PrePackageUpdate,
    /// After a package is updated.
    PostPackageUpdate,
    /// Before a package is uninstalled.
    PrePackageUninstall,
    /// After a package is uninstalled.
    PostPackageUninstall,

    // Dependency resolution events
    /// Before dependency resolution starts.
    PreDependenciesSolving,
    /// After dependency resolution completes.
    PostDependenciesSolving,

    // Download events
    /// Before a file is downloaded (can modify URL).
    PreFileDownload,

    // Archive events
    /// Before archive creation.
    PreArchiveCreate,
    /// After archive creation.
    PostArchiveCreate,

    // Script events
    /// Before scripts are run.
    PreScriptsRun,
    /// After scripts are run.
    PostScriptsRun,

    // Pool events
    /// Before pool creation.
    PrePoolCreate,

    // Operations events
    /// Before operations execution.
    PreOperationsExec,

    // Custom command event
    /// Custom command executed.
    Command,

    // Init event
    /// Plugin initialization.
    Init,
}

impl Hook {
    /// Get the hook name as a string (Composer-compatible).
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::PreInstallCmd => "pre-install-cmd",
            Self::PostInstallCmd => "post-install-cmd",
            Self::PreUpdateCmd => "pre-update-cmd",
            Self::PostUpdateCmd => "post-update-cmd",
            Self::PreStatusCmd => "pre-status-cmd",
            Self::PostStatusCmd => "post-status-cmd",
            Self::PreAutoloadDump => "pre-autoload-dump",
            Self::PostAutoloadDump => "post-autoload-dump",
            Self::PrePackageInstall => "pre-package-install",
            Self::PostPackageInstall => "post-package-install",
            Self::PrePackageUpdate => "pre-package-update",
            Self::PostPackageUpdate => "post-package-update",
            Self::PrePackageUninstall => "pre-package-uninstall",
            Self::PostPackageUninstall => "post-package-uninstall",
            Self::PreDependenciesSolving => "pre-dependencies-solving",
            Self::PostDependenciesSolving => "post-dependencies-solving",
            Self::PreFileDownload => "pre-file-download",
            Self::PreArchiveCreate => "pre-archive-create",
            Self::PostArchiveCreate => "post-archive-create",
            Self::PreScriptsRun => "pre-scripts-run",
            Self::PostScriptsRun => "post-scripts-run",
            Self::PrePoolCreate => "pre-pool-create",
            Self::PreOperationsExec => "pre-operations-exec",
            Self::Command => "command",
            Self::Init => "init",
        }
    }

    /// Parse hook from string.
    #[must_use]
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "pre-install-cmd" => Some(Self::PreInstallCmd),
            "post-install-cmd" => Some(Self::PostInstallCmd),
            "pre-update-cmd" => Some(Self::PreUpdateCmd),
            "post-update-cmd" => Some(Self::PostUpdateCmd),
            "pre-status-cmd" => Some(Self::PreStatusCmd),
            "post-status-cmd" => Some(Self::PostStatusCmd),
            "pre-autoload-dump" => Some(Self::PreAutoloadDump),
            "post-autoload-dump" => Some(Self::PostAutoloadDump),
            "pre-package-install" => Some(Self::PrePackageInstall),
            "post-package-install" => Some(Self::PostPackageInstall),
            "pre-package-update" => Some(Self::PrePackageUpdate),
            "post-package-update" => Some(Self::PostPackageUpdate),
            "pre-package-uninstall" => Some(Self::PrePackageUninstall),
            "post-package-uninstall" => Some(Self::PostPackageUninstall),
            "pre-dependencies-solving" => Some(Self::PreDependenciesSolving),
            "post-dependencies-solving" => Some(Self::PostDependenciesSolving),
            "pre-file-download" => Some(Self::PreFileDownload),
            "pre-archive-create" => Some(Self::PreArchiveCreate),
            "post-archive-create" => Some(Self::PostArchiveCreate),
            "pre-scripts-run" => Some(Self::PreScriptsRun),
            "post-scripts-run" => Some(Self::PostScriptsRun),
            "pre-pool-create" => Some(Self::PrePoolCreate),
            "pre-operations-exec" => Some(Self::PreOperationsExec),
            "command" => Some(Self::Command),
            "init" => Some(Self::Init),
            _ => None,
        }
    }

    /// Check if this is a "pre" hook.
    #[must_use]
    pub const fn is_pre_hook(&self) -> bool {
        matches!(
            self,
            Self::PreInstallCmd
                | Self::PreUpdateCmd
                | Self::PreStatusCmd
                | Self::PreAutoloadDump
                | Self::PrePackageInstall
                | Self::PrePackageUpdate
                | Self::PrePackageUninstall
                | Self::PreDependenciesSolving
                | Self::PreFileDownload
                | Self::PreArchiveCreate
                | Self::PreScriptsRun
                | Self::PrePoolCreate
                | Self::PreOperationsExec
        )
    }

    /// Check if this is a "post" hook.
    #[must_use]
    pub const fn is_post_hook(&self) -> bool {
        matches!(
            self,
            Self::PostInstallCmd
                | Self::PostUpdateCmd
                | Self::PostStatusCmd
                | Self::PostAutoloadDump
                | Self::PostPackageInstall
                | Self::PostPackageUpdate
                | Self::PostPackageUninstall
                | Self::PostDependenciesSolving
                | Self::PostArchiveCreate
                | Self::PostScriptsRun
        )
    }

    /// Get the corresponding pre/post hook.
    #[must_use]
    pub const fn counterpart(&self) -> Option<Self> {
        match self {
            Self::PreInstallCmd => Some(Self::PostInstallCmd),
            Self::PostInstallCmd => Some(Self::PreInstallCmd),
            Self::PreUpdateCmd => Some(Self::PostUpdateCmd),
            Self::PostUpdateCmd => Some(Self::PreUpdateCmd),
            Self::PreStatusCmd => Some(Self::PostStatusCmd),
            Self::PostStatusCmd => Some(Self::PreStatusCmd),
            Self::PreAutoloadDump => Some(Self::PostAutoloadDump),
            Self::PostAutoloadDump => Some(Self::PreAutoloadDump),
            Self::PrePackageInstall => Some(Self::PostPackageInstall),
            Self::PostPackageInstall => Some(Self::PrePackageInstall),
            Self::PrePackageUpdate => Some(Self::PostPackageUpdate),
            Self::PostPackageUpdate => Some(Self::PrePackageUpdate),
            Self::PrePackageUninstall => Some(Self::PostPackageUninstall),
            Self::PostPackageUninstall => Some(Self::PrePackageUninstall),
            Self::PreDependenciesSolving => Some(Self::PostDependenciesSolving),
            Self::PostDependenciesSolving => Some(Self::PreDependenciesSolving),
            Self::PreArchiveCreate => Some(Self::PostArchiveCreate),
            Self::PostArchiveCreate => Some(Self::PreArchiveCreate),
            Self::PreScriptsRun => Some(Self::PostScriptsRun),
            Self::PostScriptsRun => Some(Self::PreScriptsRun),
            _ => None,
        }
    }

    /// Get all hooks.
    #[must_use]
    pub fn all() -> Vec<Self> {
        vec![
            Self::PreInstallCmd,
            Self::PostInstallCmd,
            Self::PreUpdateCmd,
            Self::PostUpdateCmd,
            Self::PreStatusCmd,
            Self::PostStatusCmd,
            Self::PreAutoloadDump,
            Self::PostAutoloadDump,
            Self::PrePackageInstall,
            Self::PostPackageInstall,
            Self::PrePackageUpdate,
            Self::PostPackageUpdate,
            Self::PrePackageUninstall,
            Self::PostPackageUninstall,
            Self::PreDependenciesSolving,
            Self::PostDependenciesSolving,
            Self::PreFileDownload,
            Self::PreArchiveCreate,
            Self::PostArchiveCreate,
            Self::PreScriptsRun,
            Self::PostScriptsRun,
            Self::PrePoolCreate,
            Self::PreOperationsExec,
            Self::Command,
            Self::Init,
        ]
    }
}

impl std::fmt::Display for Hook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Hook handler registration.
#[derive(Debug, Clone)]
pub struct HookHandler {
    /// Plugin that registered this handler.
    pub plugin_id: String,
    /// Handler priority.
    pub priority: i32,
    /// Whether this handler is enabled.
    pub enabled: bool,
}

impl HookHandler {
    /// Create a new hook handler.
    #[must_use]
    pub fn new(plugin_id: impl Into<String>, priority: i32) -> Self {
        Self {
            plugin_id: plugin_id.into(),
            priority,
            enabled: true,
        }
    }
}

impl PartialEq for HookHandler {
    fn eq(&self, other: &Self) -> bool {
        self.plugin_id == other.plugin_id
    }
}

impl Eq for HookHandler {}

impl PartialOrd for HookHandler {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for HookHandler {
    fn cmp(&self, other: &Self) -> Ordering {
        self.priority.cmp(&other.priority)
    }
}

/// Registry for hook handlers.
#[derive(Debug, Default)]
pub struct HookRegistry {
    /// Handlers registered for each hook.
    handlers: DashMap<Hook, Vec<HookHandler>>,
    /// Cached sorted handlers.
    cache: RwLock<Option<DashMap<Hook, Vec<HookHandler>>>>,
}

impl HookRegistry {
    /// Create a new hook registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a handler for a hook.
    pub fn register(&self, hook: Hook, plugin_id: impl Into<String>, priority: i32) {
        let handler = HookHandler::new(plugin_id, priority);

        self.handlers.entry(hook).or_default().push(handler);

        // Invalidate cache
        *self.cache.write() = None;
    }

    /// Unregister all handlers for a plugin.
    pub fn unregister_plugin(&self, plugin_id: &str) {
        for mut entry in self.handlers.iter_mut() {
            entry.value_mut().retain(|h| h.plugin_id != plugin_id);
        }
        // Invalidate cache
        *self.cache.write() = None;
    }

    /// Unregister a specific handler.
    pub fn unregister(&self, hook: Hook, plugin_id: &str) {
        if let Some(mut handlers) = self.handlers.get_mut(&hook) {
            handlers.retain(|h| h.plugin_id != plugin_id);
        }
        // Invalidate cache
        *self.cache.write() = None;
    }

    /// Get handlers for a hook, sorted by priority.
    #[must_use]
    pub fn get_handlers(&self, hook: &Hook) -> Vec<HookHandler> {
        // Check cache first
        {
            let cache = self.cache.read();
            if let Some(ref cached) = *cache
                && let Some(handlers) = cached.get(hook)
            {
                return handlers.clone();
            }
        }

        // Get and sort handlers
        let mut handlers: Vec<HookHandler> = self
            .handlers
            .get(hook)
            .map(|h| h.iter().filter(|h| h.enabled).cloned().collect())
            .unwrap_or_default();

        handlers.sort();

        // Update cache
        {
            let mut cache = self.cache.write();
            if cache.is_none() {
                *cache = Some(DashMap::new());
            }
            if let Some(ref cached) = *cache {
                cached.insert(*hook, handlers.clone());
            }
        }

        handlers
    }

    /// Check if any handler is registered for a hook.
    #[must_use]
    pub fn has_handlers(&self, hook: &Hook) -> bool {
        self.handlers
            .get(hook)
            .is_some_and(|h| h.iter().any(|h| h.enabled))
    }

    /// Get all registered hooks.
    #[must_use]
    pub fn registered_hooks(&self) -> Vec<Hook> {
        self.handlers
            .iter()
            .filter(|entry| !entry.value().is_empty())
            .map(|entry| *entry.key())
            .collect()
    }

    /// Enable a handler.
    pub fn enable(&self, hook: Hook, plugin_id: &str) {
        if let Some(mut handlers) = self.handlers.get_mut(&hook) {
            for handler in handlers.iter_mut() {
                if handler.plugin_id == plugin_id {
                    handler.enabled = true;
                }
            }
        }
        *self.cache.write() = None;
    }

    /// Disable a handler.
    pub fn disable(&self, hook: Hook, plugin_id: &str) {
        if let Some(mut handlers) = self.handlers.get_mut(&hook) {
            for handler in handlers.iter_mut() {
                if handler.plugin_id == plugin_id {
                    handler.enabled = false;
                }
            }
        }
        *self.cache.write() = None;
    }

    /// Clear all handlers.
    pub fn clear(&self) {
        self.handlers.clear();
        *self.cache.write() = None;
    }

    /// Get handler count for a hook.
    #[must_use]
    pub fn handler_count(&self, hook: &Hook) -> usize {
        self.handlers.get(hook).map_or(0, |h| h.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_names() {
        assert_eq!(Hook::PreInstallCmd.as_str(), "pre-install-cmd");
        assert_eq!(Hook::from_str("pre-install-cmd"), Some(Hook::PreInstallCmd));
    }

    #[test]
    fn hook_pre_post() {
        assert!(Hook::PreInstallCmd.is_pre_hook());
        assert!(!Hook::PreInstallCmd.is_post_hook());
        assert!(Hook::PostInstallCmd.is_post_hook());
        assert!(!Hook::PostInstallCmd.is_pre_hook());
    }

    #[test]
    fn hook_counterpart() {
        assert_eq!(
            Hook::PreInstallCmd.counterpart(),
            Some(Hook::PostInstallCmd)
        );
        assert_eq!(
            Hook::PostInstallCmd.counterpart(),
            Some(Hook::PreInstallCmd)
        );
    }

    #[test]
    fn registry_basic() {
        let registry = HookRegistry::new();

        registry.register(Hook::PreInstallCmd, "plugin-a", 0);
        registry.register(Hook::PreInstallCmd, "plugin-b", -10);

        let handlers = registry.get_handlers(&Hook::PreInstallCmd);
        assert_eq!(handlers.len(), 2);
        // Lower priority should come first
        assert_eq!(handlers[0].plugin_id, "plugin-b");
        assert_eq!(handlers[1].plugin_id, "plugin-a");
    }

    #[test]
    fn registry_unregister() {
        let registry = HookRegistry::new();

        registry.register(Hook::PreInstallCmd, "plugin-a", 0);
        registry.register(Hook::PreInstallCmd, "plugin-b", 0);

        registry.unregister(Hook::PreInstallCmd, "plugin-a");

        let handlers = registry.get_handlers(&Hook::PreInstallCmd);
        assert_eq!(handlers.len(), 1);
        assert_eq!(handlers[0].plugin_id, "plugin-b");
    }

    #[test]
    fn registry_unregister_plugin() {
        let registry = HookRegistry::new();

        registry.register(Hook::PreInstallCmd, "plugin-a", 0);
        registry.register(Hook::PostInstallCmd, "plugin-a", 0);
        registry.register(Hook::PreInstallCmd, "plugin-b", 0);

        registry.unregister_plugin("plugin-a");

        assert_eq!(registry.get_handlers(&Hook::PreInstallCmd).len(), 1);
        assert_eq!(registry.get_handlers(&Hook::PostInstallCmd).len(), 0);
    }

    #[test]
    fn registry_enable_disable() {
        let registry = HookRegistry::new();

        registry.register(Hook::PreInstallCmd, "plugin-a", 0);

        registry.disable(Hook::PreInstallCmd, "plugin-a");
        assert_eq!(registry.get_handlers(&Hook::PreInstallCmd).len(), 0);

        registry.enable(Hook::PreInstallCmd, "plugin-a");
        assert_eq!(registry.get_handlers(&Hook::PreInstallCmd).len(), 1);
    }

    #[test]
    fn priority_ordering() {
        assert!(HookPriority::Highest.as_value() < HookPriority::Normal.as_value());
        assert!(HookPriority::Normal.as_value() < HookPriority::Lowest.as_value());
    }
}
