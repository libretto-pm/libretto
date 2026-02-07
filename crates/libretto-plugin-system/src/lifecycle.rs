//! Plugin lifecycle management.
//!
//! This module handles the complete lifecycle of plugins:
//! Load → Initialize → Register hooks → Execute → Unload

use crate::error::{PluginError, Result};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

/// Plugin state in its lifecycle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginState {
    /// Plugin is discovered but not loaded.
    Unloaded,
    /// Plugin is being loaded.
    Loading,
    /// Plugin is loaded but not activated.
    Loaded,
    /// Plugin is being initialized.
    Initializing,
    /// Plugin is active and ready.
    Active,
    /// Plugin is being deactivated.
    Deactivating,
    /// Plugin is being unloaded.
    Unloading,
    /// Plugin encountered an error.
    Error(String),
    /// Plugin is suspended (temporarily disabled).
    Suspended,
}

impl PluginState {
    /// Check if the plugin can process events.
    #[must_use]
    pub const fn can_process_events(&self) -> bool {
        matches!(self, Self::Active)
    }

    /// Check if the plugin is in an error state.
    #[must_use]
    pub const fn is_error(&self) -> bool {
        matches!(self, Self::Error(_))
    }

    /// Check if the plugin can be loaded.
    #[must_use]
    pub const fn can_load(&self) -> bool {
        matches!(self, Self::Unloaded | Self::Error(_))
    }

    /// Check if the plugin can be unloaded.
    #[must_use]
    pub const fn can_unload(&self) -> bool {
        matches!(
            self,
            Self::Loaded | Self::Active | Self::Suspended | Self::Error(_)
        )
    }

    /// Check if the plugin can be activated.
    #[must_use]
    pub const fn can_activate(&self) -> bool {
        matches!(self, Self::Loaded | Self::Suspended)
    }

    /// Check if the plugin can be deactivated.
    #[must_use]
    pub const fn can_deactivate(&self) -> bool {
        matches!(self, Self::Active)
    }
}

impl std::fmt::Display for PluginState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unloaded => write!(f, "unloaded"),
            Self::Loading => write!(f, "loading"),
            Self::Loaded => write!(f, "loaded"),
            Self::Initializing => write!(f, "initializing"),
            Self::Active => write!(f, "active"),
            Self::Deactivating => write!(f, "deactivating"),
            Self::Unloading => write!(f, "unloading"),
            Self::Error(e) => write!(f, "error: {e}"),
            Self::Suspended => write!(f, "suspended"),
        }
    }
}

/// Lifecycle event types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleEvent {
    /// Plugin is about to load.
    BeforeLoad,
    /// Plugin finished loading.
    AfterLoad,
    /// Plugin is about to initialize.
    BeforeInit,
    /// Plugin finished initializing.
    AfterInit,
    /// Plugin is about to activate.
    BeforeActivate,
    /// Plugin finished activating.
    AfterActivate,
    /// Plugin is about to deactivate.
    BeforeDeactivate,
    /// Plugin finished deactivating.
    AfterDeactivate,
    /// Plugin is about to unload.
    BeforeUnload,
    /// Plugin finished unloading.
    AfterUnload,
}

/// Lifecycle callback type.
pub type LifecycleCallback = Arc<dyn Fn(&str, LifecycleEvent) + Send + Sync>;

/// Plugin lifecycle manager.
#[derive(Default)]
pub struct PluginLifecycle {
    /// Lifecycle callbacks.
    callbacks: RwLock<Vec<LifecycleCallback>>,
    /// Load times for performance tracking.
    load_times: RwLock<std::collections::HashMap<String, Duration>>,
    /// Activation times.
    activation_times: RwLock<std::collections::HashMap<String, Duration>>,
}

impl std::fmt::Debug for PluginLifecycle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginLifecycle")
            .field("callbacks_count", &self.callbacks.read().len())
            .field("load_times", &self.load_times.read().len())
            .field("activation_times", &self.activation_times.read().len())
            .finish()
    }
}

impl PluginLifecycle {
    /// Create a new lifecycle manager.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a lifecycle callback.
    pub fn on_event(&self, callback: LifecycleCallback) {
        self.callbacks.write().push(callback);
    }

    /// Emit a lifecycle event.
    pub fn emit(&self, plugin_id: &str, event: LifecycleEvent) {
        let callbacks = self.callbacks.read();
        for callback in callbacks.iter() {
            callback(plugin_id, event);
        }
    }

    /// Record plugin load time.
    pub fn record_load_time(&self, plugin_id: &str, duration: Duration) {
        self.load_times
            .write()
            .insert(plugin_id.to_string(), duration);
        debug!(plugin = %plugin_id, duration_ms = duration.as_millis(), "plugin load time recorded");
    }

    /// Record plugin activation time.
    pub fn record_activation_time(&self, plugin_id: &str, duration: Duration) {
        self.activation_times
            .write()
            .insert(plugin_id.to_string(), duration);
        debug!(plugin = %plugin_id, duration_ms = duration.as_millis(), "plugin activation time recorded");
    }

    /// Get plugin load time.
    #[must_use]
    pub fn get_load_time(&self, plugin_id: &str) -> Option<Duration> {
        self.load_times.read().get(plugin_id).copied()
    }

    /// Get plugin activation time.
    #[must_use]
    pub fn get_activation_time(&self, plugin_id: &str) -> Option<Duration> {
        self.activation_times.read().get(plugin_id).copied()
    }

    /// Get total load time for all plugins.
    #[must_use]
    pub fn total_load_time(&self) -> Duration {
        self.load_times.read().values().sum()
    }

    /// Get total activation time for all plugins.
    #[must_use]
    pub fn total_activation_time(&self) -> Duration {
        self.activation_times.read().values().sum()
    }

    /// Check if load time exceeds threshold.
    #[must_use]
    pub fn is_slow_loader(&self, plugin_id: &str, threshold: Duration) -> bool {
        self.load_times
            .read()
            .get(plugin_id)
            .is_some_and(|&t| t > threshold)
    }
}

/// State transition validator.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct StateTransition {
    /// Previous state.
    pub from: PluginState,
    /// New state.
    pub to: PluginState,
    /// Timestamp of transition.
    pub timestamp: Instant,
    /// Duration of the transition operation.
    pub duration: Option<Duration>,
}

#[allow(dead_code)]
impl StateTransition {
    /// Create a new state transition.
    #[must_use]
    pub fn new(from: PluginState, to: PluginState) -> Self {
        Self {
            from,
            to,
            timestamp: Instant::now(),
            duration: None,
        }
    }

    /// Set the duration.
    #[must_use]
    pub const fn with_duration(mut self, duration: Duration) -> Self {
        self.duration = Some(duration);
        self
    }
}

/// Validate state transitions.
#[allow(dead_code)]
pub fn validate_transition(from: &PluginState, to: &PluginState) -> Result<()> {
    let valid = match (from, to) {
        // From Unloaded
        (PluginState::Unloaded, PluginState::Loading) => true,

        // From Loading
        (PluginState::Loading, PluginState::Loaded) => true,
        (PluginState::Loading, PluginState::Error(_)) => true,

        // From Loaded
        (PluginState::Loaded, PluginState::Initializing) => true,
        (PluginState::Loaded, PluginState::Unloading) => true,

        // From Initializing
        (PluginState::Initializing, PluginState::Active) => true,
        (PluginState::Initializing, PluginState::Error(_)) => true,

        // From Active
        (PluginState::Active, PluginState::Deactivating) => true,
        (PluginState::Active, PluginState::Suspended) => true,
        (PluginState::Active, PluginState::Error(_)) => true,

        // From Deactivating
        (PluginState::Deactivating, PluginState::Loaded) => true,
        (PluginState::Deactivating, PluginState::Unloading) => true,
        (PluginState::Deactivating, PluginState::Error(_)) => true,

        // From Unloading
        (PluginState::Unloading, PluginState::Unloaded) => true,
        (PluginState::Unloading, PluginState::Error(_)) => true,

        // From Suspended
        (PluginState::Suspended, PluginState::Active) => true,
        (PluginState::Suspended, PluginState::Unloading) => true,

        // From Error
        (PluginState::Error(_), PluginState::Unloaded) => true,
        (PluginState::Error(_), PluginState::Loading) => true,

        // All other transitions are invalid
        _ => false,
    };

    if valid {
        Ok(())
    } else {
        Err(PluginError::InvalidOperation(format!(
            "invalid state transition: {from} -> {to}"
        )))
    }
}

/// Plugin lifecycle state machine.
#[derive(Debug)]
#[allow(dead_code)]
pub struct PluginStateMachine {
    /// Current state.
    state: RwLock<PluginState>,
    /// Plugin ID.
    plugin_id: String,
    /// State history.
    history: RwLock<Vec<StateTransition>>,
    /// Lifecycle manager reference.
    lifecycle: Arc<PluginLifecycle>,
}

#[allow(dead_code)]
impl PluginStateMachine {
    /// Create a new state machine.
    #[must_use]
    pub fn new(plugin_id: impl Into<String>, lifecycle: Arc<PluginLifecycle>) -> Self {
        Self {
            state: RwLock::new(PluginState::Unloaded),
            plugin_id: plugin_id.into(),
            history: RwLock::new(Vec::new()),
            lifecycle,
        }
    }

    /// Get current state.
    #[must_use]
    pub fn state(&self) -> PluginState {
        self.state.read().clone()
    }

    /// Transition to a new state.
    ///
    /// # Errors
    /// Returns error if the transition is invalid.
    pub fn transition(&self, new_state: PluginState) -> Result<()> {
        let mut current = self.state.write();
        validate_transition(&current, &new_state)?;

        let transition = StateTransition::new(current.clone(), new_state.clone());
        self.history.write().push(transition);

        info!(
            plugin = %self.plugin_id,
            from = %current,
            to = %new_state,
            "state transition"
        );

        *current = new_state;
        Ok(())
    }

    /// Transition with a timed operation.
    ///
    /// # Errors
    /// Returns error if the transition is invalid.
    pub fn transition_with_timing(
        &self,
        intermediate: PluginState,
        final_state: PluginState,
        operation: impl FnOnce() -> Result<()>,
    ) -> Result<()> {
        // Transition to intermediate state
        self.transition(intermediate)?;

        let start = Instant::now();
        let result = operation();
        let duration = start.elapsed();

        match result {
            Ok(()) => {
                // Record timing
                match &final_state {
                    PluginState::Loaded => {
                        self.lifecycle.record_load_time(&self.plugin_id, duration);
                    }
                    PluginState::Active => {
                        self.lifecycle
                            .record_activation_time(&self.plugin_id, duration);
                    }
                    _ => {}
                }

                self.transition(final_state)?;
                Ok(())
            }
            Err(e) => {
                self.transition(PluginState::Error(e.to_string()))?;
                Err(e)
            }
        }
    }

    /// Get state history.
    #[must_use]
    pub fn history(&self) -> Vec<StateTransition> {
        self.history.read().clone()
    }

    /// Reset to unloaded state (for recovery).
    pub fn reset(&self) {
        let mut state = self.state.write();
        let transition = StateTransition::new(state.clone(), PluginState::Unloaded);
        self.history.write().push(transition);
        *state = PluginState::Unloaded;
        warn!(plugin = %self.plugin_id, "state machine reset");
    }
}

/// Lazy loading support.
#[derive(Debug)]
#[allow(dead_code)]
pub struct LazyLoader {
    /// Plugins pending lazy load.
    pending: RwLock<Vec<String>>,
    /// Hooks that should trigger lazy loading.
    trigger_hooks: RwLock<std::collections::HashMap<String, Vec<crate::hooks::Hook>>>,
}

#[allow(dead_code)]
impl LazyLoader {
    /// Create a new lazy loader.
    #[must_use]
    pub fn new() -> Self {
        Self {
            pending: RwLock::new(Vec::new()),
            trigger_hooks: RwLock::new(std::collections::HashMap::new()),
        }
    }

    /// Mark a plugin for lazy loading.
    pub fn register(&self, plugin_id: impl Into<String>, hooks: Vec<crate::hooks::Hook>) {
        let plugin_id = plugin_id.into();
        self.pending.write().push(plugin_id.clone());
        self.trigger_hooks.write().insert(plugin_id, hooks);
    }

    /// Check if a hook should trigger lazy loading.
    #[must_use]
    pub fn should_load_for_hook(&self, hook: &crate::hooks::Hook) -> Vec<String> {
        let trigger_hooks = self.trigger_hooks.read();
        let pending = self.pending.read();

        pending
            .iter()
            .filter(|plugin_id| {
                trigger_hooks
                    .get(*plugin_id)
                    .is_some_and(|hooks| hooks.contains(hook))
            })
            .cloned()
            .collect()
    }

    /// Mark a plugin as loaded.
    pub fn mark_loaded(&self, plugin_id: &str) {
        self.pending.write().retain(|id| id != plugin_id);
    }

    /// Check if any plugins are pending.
    #[must_use]
    pub fn has_pending(&self) -> bool {
        !self.pending.read().is_empty()
    }

    /// Get pending plugins.
    #[must_use]
    pub fn pending_plugins(&self) -> Vec<String> {
        self.pending.read().clone()
    }
}

impl Default for LazyLoader {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_transitions() {
        // Valid transitions
        assert!(validate_transition(&PluginState::Unloaded, &PluginState::Loading).is_ok());
        assert!(validate_transition(&PluginState::Loading, &PluginState::Loaded).is_ok());
        assert!(validate_transition(&PluginState::Active, &PluginState::Deactivating).is_ok());

        // Invalid transitions
        assert!(validate_transition(&PluginState::Unloaded, &PluginState::Active).is_err());
        assert!(validate_transition(&PluginState::Active, &PluginState::Loading).is_err());
    }

    #[test]
    fn state_machine() {
        let lifecycle = Arc::new(PluginLifecycle::new());
        let sm = PluginStateMachine::new("test-plugin", lifecycle);

        assert_eq!(sm.state(), PluginState::Unloaded);

        sm.transition(PluginState::Loading).unwrap();
        assert_eq!(sm.state(), PluginState::Loading);

        sm.transition(PluginState::Loaded).unwrap();
        assert_eq!(sm.state(), PluginState::Loaded);

        // Invalid transition should fail
        assert!(sm.transition(PluginState::Unloaded).is_err());
    }

    #[test]
    fn lifecycle_timing() {
        let lifecycle = PluginLifecycle::new();

        lifecycle.record_load_time("plugin-a", Duration::from_millis(100));
        lifecycle.record_load_time("plugin-b", Duration::from_millis(50));

        assert_eq!(
            lifecycle.get_load_time("plugin-a"),
            Some(Duration::from_millis(100))
        );
        assert_eq!(lifecycle.total_load_time(), Duration::from_millis(150));
    }

    #[test]
    fn slow_loader_detection() {
        let lifecycle = PluginLifecycle::new();

        lifecycle.record_load_time("slow-plugin", Duration::from_millis(500));
        lifecycle.record_load_time("fast-plugin", Duration::from_millis(10));

        assert!(lifecycle.is_slow_loader("slow-plugin", Duration::from_millis(100)));
        assert!(!lifecycle.is_slow_loader("fast-plugin", Duration::from_millis(100)));
    }

    #[test]
    fn lazy_loader() {
        let loader = LazyLoader::new();

        loader.register("lazy-plugin", vec![crate::hooks::Hook::PreInstallCmd]);

        assert!(loader.has_pending());
        assert_eq!(loader.pending_plugins(), vec!["lazy-plugin"]);

        let to_load = loader.should_load_for_hook(&crate::hooks::Hook::PreInstallCmd);
        assert_eq!(to_load, vec!["lazy-plugin"]);

        let to_load = loader.should_load_for_hook(&crate::hooks::Hook::PostInstallCmd);
        assert!(to_load.is_empty());

        loader.mark_loaded("lazy-plugin");
        assert!(!loader.has_pending());
    }

    #[test]
    fn state_display() {
        assert_eq!(PluginState::Unloaded.to_string(), "unloaded");
        assert_eq!(PluginState::Active.to_string(), "active");
        assert_eq!(PluginState::Error("test".into()).to_string(), "error: test");
    }

    #[test]
    fn state_checks() {
        assert!(PluginState::Active.can_process_events());
        assert!(!PluginState::Loaded.can_process_events());

        assert!(PluginState::Error(String::new()).is_error());
        assert!(!PluginState::Active.is_error());

        assert!(PluginState::Unloaded.can_load());
        assert!(!PluginState::Active.can_load());
    }
}
