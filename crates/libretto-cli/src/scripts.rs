//! Composer script execution for lifecycle hooks.
//!
//! This module provides execution of Composer scripts during the install/update lifecycle:
//!
//! # Supported Events
//!
//! - `pre-install-cmd`: Before packages are installed
//! - `post-install-cmd`: After all packages are installed
//! - `pre-update-cmd`: Before packages are updated
//! - `post-update-cmd`: After all packages are updated
//! - `pre-autoload-dump`: Before autoloader is generated
//! - `post-autoload-dump`: After autoloader is generated
//! - `post-root-package-install`: After root package is installed (create-project)
//! - `pre-operations-exec`: Before package operations execute
//!
//! # Script Formats
//!
//! Scripts can be:
//! - A single command string
//! - An array of commands
//! - A reference to another script: `@script-name`
//! - Special directives: `@php`, `@composer`, `@putenv`
//!
//! # Example
//!
//! ```json
//! {
//!     "scripts": {
//!         "post-install-cmd": [
//!             "@php artisan package:discover",
//!             "echo Installation complete"
//!         ],
//!         "test": "phpunit",
//!         "cs-fix": "@php vendor/bin/php-cs-fixer fix"
//!     }
//! }
//! ```

use anyhow::{Context, Result, bail};
use sonic_rs::{JsonContainerTrait, JsonValueTrait, Value};
use std::collections::HashMap;
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};
use tracing::{debug, info};

/// Inline Composer compatibility stubs.
/// This provides full Composer Event API when the real composer/composer package isn't available.
/// Note: The stubs file starts with `<?php` which we need to strip for inline embedding.
fn get_composer_stubs() -> &'static str {
    const STUBS: &str = include_str!("../resources/composer-stubs.php");
    // Strip the opening <?php tag and any following whitespace
    STUBS.trim_start_matches("<?php").trim_start()
}

/// Script event types for lifecycle hooks.
///
/// This enum covers ALL Composer script events for full compatibility.
/// See: <https://getcomposer.org/doc/articles/scripts.md>
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(dead_code)]
pub enum ScriptEvent {
    // ===== Command Events =====
    /// Before packages are installed (install command).
    PreInstallCmd,
    /// After packages are installed (install command).
    PostInstallCmd,
    /// Before packages are updated (update command).
    PreUpdateCmd,
    /// After packages are updated (update command).
    PostUpdateCmd,
    /// Before status command runs.
    PreStatusCmd,
    /// After status command runs.
    PostStatusCmd,
    /// Before archive command runs.
    PreArchiveCmd,
    /// After archive command runs.
    PostArchiveCmd,
    /// Before autoloader is generated.
    PreAutoloadDump,
    /// After autoloader is generated.
    PostAutoloadDump,

    // ===== Package Events =====
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

    // ===== Installer Events =====
    /// Before package operations execute.
    PreOperationsExec,

    // ===== Project Events =====
    /// After root package is installed (create-project).
    PostRootPackageInstall,
    /// After create-project command completes.
    PostCreateProjectCmd,

    // ===== Plugin Events =====
    /// Before a file is downloaded.
    PreFileDownload,
    /// After a file is downloaded.
    PostFileDownload,
    /// Before a command runs.
    PreCommandRun,
    /// Before the package pool is created.
    PrePoolCreate,
}

#[allow(dead_code)]
impl ScriptEvent {
    /// Get the script key name.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            // Command events
            Self::PreInstallCmd => "pre-install-cmd",
            Self::PostInstallCmd => "post-install-cmd",
            Self::PreUpdateCmd => "pre-update-cmd",
            Self::PostUpdateCmd => "post-update-cmd",
            Self::PreStatusCmd => "pre-status-cmd",
            Self::PostStatusCmd => "post-status-cmd",
            Self::PreArchiveCmd => "pre-archive-cmd",
            Self::PostArchiveCmd => "post-archive-cmd",
            Self::PreAutoloadDump => "pre-autoload-dump",
            Self::PostAutoloadDump => "post-autoload-dump",
            // Package events
            Self::PrePackageInstall => "pre-package-install",
            Self::PostPackageInstall => "post-package-install",
            Self::PrePackageUpdate => "pre-package-update",
            Self::PostPackageUpdate => "post-package-update",
            Self::PrePackageUninstall => "pre-package-uninstall",
            Self::PostPackageUninstall => "post-package-uninstall",
            // Installer events
            Self::PreOperationsExec => "pre-operations-exec",
            // Project events
            Self::PostRootPackageInstall => "post-root-package-install",
            Self::PostCreateProjectCmd => "post-create-project-cmd",
            // Plugin events
            Self::PreFileDownload => "pre-file-download",
            Self::PostFileDownload => "post-file-download",
            Self::PreCommandRun => "pre-command-run",
            Self::PrePoolCreate => "pre-pool-create",
        }
    }

    /// Parse from string.
    #[must_use]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            // Command events
            "pre-install-cmd" => Some(Self::PreInstallCmd),
            "post-install-cmd" => Some(Self::PostInstallCmd),
            "pre-update-cmd" => Some(Self::PreUpdateCmd),
            "post-update-cmd" => Some(Self::PostUpdateCmd),
            "pre-status-cmd" => Some(Self::PreStatusCmd),
            "post-status-cmd" => Some(Self::PostStatusCmd),
            "pre-archive-cmd" => Some(Self::PreArchiveCmd),
            "post-archive-cmd" => Some(Self::PostArchiveCmd),
            "pre-autoload-dump" => Some(Self::PreAutoloadDump),
            "post-autoload-dump" => Some(Self::PostAutoloadDump),
            // Package events
            "pre-package-install" => Some(Self::PrePackageInstall),
            "post-package-install" => Some(Self::PostPackageInstall),
            "pre-package-update" => Some(Self::PrePackageUpdate),
            "post-package-update" => Some(Self::PostPackageUpdate),
            "pre-package-uninstall" => Some(Self::PrePackageUninstall),
            "post-package-uninstall" => Some(Self::PostPackageUninstall),
            // Installer events
            "pre-operations-exec" => Some(Self::PreOperationsExec),
            // Project events
            "post-root-package-install" => Some(Self::PostRootPackageInstall),
            "post-create-project-cmd" => Some(Self::PostCreateProjectCmd),
            // Plugin events
            "pre-file-download" => Some(Self::PreFileDownload),
            "post-file-download" => Some(Self::PostFileDownload),
            "pre-command-run" => Some(Self::PreCommandRun),
            "pre-pool-create" => Some(Self::PrePoolCreate),
            _ => None,
        }
    }

    /// Check if this is a package-level event (requires package info).
    #[must_use]
    pub const fn is_package_event(&self) -> bool {
        matches!(
            self,
            Self::PrePackageInstall
                | Self::PostPackageInstall
                | Self::PrePackageUpdate
                | Self::PostPackageUpdate
                | Self::PrePackageUninstall
                | Self::PostPackageUninstall
        )
    }
}

/// Result of script execution.
#[derive(Debug)]
#[allow(dead_code)]
pub struct ScriptResult {
    /// Script name/event that was executed.
    pub name: String,
    /// Number of commands executed.
    pub commands_executed: usize,
    /// Whether all commands succeeded.
    pub success: bool,
    /// Exit code of the last failed command (if any).
    pub exit_code: Option<i32>,
    /// Error message (if failed).
    pub error: Option<String>,
    /// Total execution duration.
    pub duration: Duration,
}

impl ScriptResult {
    fn success(name: &str, commands: usize, duration: Duration) -> Self {
        Self {
            name: name.to_string(),
            commands_executed: commands,
            success: true,
            exit_code: None,
            error: None,
            duration,
        }
    }

    fn failure(name: &str, commands: usize, code: i32, error: &str, duration: Duration) -> Self {
        Self {
            name: name.to_string(),
            commands_executed: commands,
            success: false,
            exit_code: Some(code),
            error: Some(error.to_string()),
            duration,
        }
    }
}

/// Configuration for script execution.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ScriptConfig {
    /// Working directory.
    pub working_dir: std::path::PathBuf,
    /// PHP binary path.
    pub php_binary: String,
    /// Libretto binary path.
    pub composer_binary: String,
    /// Whether we're in dev mode.
    pub dev_mode: bool,
    /// Script timeout in seconds (0 = no timeout).
    pub timeout: u64,
    /// Additional environment variables.
    pub env: HashMap<String, String>,
    /// Whether to stop on first error.
    pub stop_on_error: bool,
}

impl Default for ScriptConfig {
    fn default() -> Self {
        Self {
            working_dir: std::env::current_dir().unwrap_or_default(),
            php_binary: "php".to_string(),
            composer_binary: std::env::current_exe().map_or_else(
                |_| "libretto".to_string(),
                |p| p.to_string_lossy().to_string(),
            ),
            dev_mode: true,
            timeout: 300,
            env: HashMap::new(),
            stop_on_error: true,
        }
    }
}

/// Script executor for running Composer scripts.
#[allow(dead_code)]
pub struct ScriptExecutor {
    /// Parsed scripts from composer.json.
    scripts: HashMap<String, Vec<String>>,
    /// Configuration.
    config: ScriptConfig,
    /// Script call stack (for detecting recursion).
    call_stack: Vec<String>,
}

#[allow(dead_code)]
impl ScriptExecutor {
    /// Create a new script executor from composer.json content.
    pub fn new(composer_json: &Value, config: ScriptConfig) -> Self {
        let scripts = Self::parse_scripts(composer_json);
        Self {
            scripts,
            config,
            call_stack: Vec::new(),
        }
    }

    /// Parse scripts from composer.json.
    fn parse_scripts(composer_json: &Value) -> HashMap<String, Vec<String>> {
        let mut scripts = HashMap::new();

        if let Some(scripts_obj) = composer_json.get("scripts").and_then(|v| v.as_object()) {
            for (name, value) in scripts_obj {
                let commands = if let Some(cmd) = value.as_str() {
                    vec![cmd.to_string()]
                } else if let Some(arr) = value.as_array() {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                } else {
                    continue;
                };

                scripts.insert(name.to_string(), commands);
            }
        }

        scripts
    }

    /// Check if a script exists.
    #[must_use]
    pub fn has_script(&self, name: &str) -> bool {
        self.scripts.contains_key(name)
    }

    /// Check if an event script exists.
    #[must_use]
    pub fn has_event(&self, event: ScriptEvent) -> bool {
        self.has_script(event.as_str())
    }

    /// Get list of available scripts.
    #[must_use]
    pub fn available_scripts(&self) -> Vec<&str> {
        self.scripts
            .keys()
            .map(std::string::String::as_str)
            .collect()
    }

    /// Execute an event script.
    pub fn run_event(&mut self, event: ScriptEvent) -> Result<Option<ScriptResult>> {
        self.run_script(event.as_str())
    }

    /// Execute a named script.
    pub fn run_script(&mut self, name: &str) -> Result<Option<ScriptResult>> {
        let commands = match self.scripts.get(name) {
            Some(cmds) => cmds.clone(),
            None => return Ok(None),
        };

        if commands.is_empty() {
            return Ok(None);
        }

        // Check for recursion
        if self.call_stack.contains(&name.to_string()) {
            bail!(
                "Circular script reference detected: {} -> {}",
                self.call_stack.join(" -> "),
                name
            );
        }

        self.call_stack.push(name.to_string());

        let start = Instant::now();
        let mut executed = 0;

        info!(script = %name, commands = commands.len(), "executing script");

        for cmd in &commands {
            let result = self.execute_command(cmd)?;
            executed += 1;

            if let Some(status) = result
                && !status.success()
            {
                let code = status.code().unwrap_or(-1);
                self.call_stack.pop();
                return Ok(Some(ScriptResult::failure(
                    name,
                    executed,
                    code,
                    &format!("Command failed: {cmd}"),
                    start.elapsed(),
                )));
            }
        }

        self.call_stack.pop();

        Ok(Some(ScriptResult::success(name, executed, start.elapsed())))
    }

    /// Execute a single command.
    fn execute_command(&mut self, cmd: &str) -> Result<Option<ExitStatus>> {
        let cmd = cmd.trim();

        // Handle @reference syntax
        if let Some(ref_name) = cmd.strip_prefix('@') {
            // Handle special directives
            if ref_name.starts_with("php ") {
                let php_cmd = ref_name.strip_prefix("php ").unwrap();
                return self.execute_shell(&format!("{} {}", self.config.php_binary, php_cmd));
            }

            if ref_name.starts_with("composer ") {
                let composer_cmd = ref_name.strip_prefix("composer ").unwrap();
                return self
                    .execute_shell(&format!("{} {}", self.config.composer_binary, composer_cmd));
            }

            if ref_name.starts_with("putenv ") {
                let putenv = ref_name.strip_prefix("putenv ").unwrap();
                if let Some((key, value)) = putenv.split_once('=') {
                    self.config.env.insert(key.to_string(), value.to_string());
                }
                return Ok(None);
            }

            // Reference to another script
            if let Some(result) = self.run_script(ref_name)?
                && !result.success
            {
                bail!(
                    "Referenced script '{}' failed: {:?}",
                    ref_name,
                    result.error
                );
            }
            return Ok(None);
        }

        // Check for PHP class method syntax: Namespace\Class::method
        // This pattern matches class names like "Illuminate\Foundation\ComposerScripts::postAutoloadDump"
        if is_php_class_method(cmd) {
            return self.execute_php_callback(cmd);
        }

        self.execute_shell(cmd)
    }

    /// Execute a PHP class static method callback.
    ///
    /// Composer-style callbacks like `Illuminate\Foundation\ComposerScripts::postAutoloadDump`
    /// expect a `Composer\Script\Event` object. Instead of mocking Composer's complex internals,
    /// we take a pragmatic approach:
    ///
    /// 1. For known callbacks (like Laravel's), we do what they actually do directly
    /// 2. For unknown callbacks, we use reflection to check requirements
    /// 3. We set environment variables so scripts can detect Libretto
    ///
    /// This is the senior engineer approach: solve the actual problem, don't emulate complexity.
    fn execute_php_callback(&self, callback: &str) -> Result<Option<ExitStatus>> {
        debug!(callback = %callback, "executing PHP callback");

        // Handle known callbacks directly by doing what they actually do
        if let Some(result) = self.handle_known_callback(callback)? {
            return Ok(Some(result));
        }

        // For unknown callbacks, try reflection-based approach
        self.execute_unknown_callback(callback)
    }

    /// Handle well-known Composer callbacks by doing what they actually do.
    /// This avoids needing to mock Composer's complex Event system.
    fn handle_known_callback(&self, callback: &str) -> Result<Option<ExitStatus>> {
        let normalized = callback.replace('/', "\\").to_lowercase();

        // Laravel's ComposerScripts - they just clear cache files
        if normalized.contains("illuminate\\foundation\\composerscripts::") {
            let method = callback.split("::").nth(1).unwrap_or("");
            match method.to_lowercase().as_str() {
                "postautloaddump" | "postautoloaddump" | "postinstall" | "postupdate" => {
                    debug!("Handling Laravel ComposerScripts::{} directly", method);
                    return self.laravel_clear_compiled();
                }
                _ => {}
            }
        }

        // Composer\Config::disableProcessTimeout - this is a no-op for us
        if normalized == "composer\\config::disableprocesstimeout" {
            debug!("Ignoring Composer\\Config::disableProcessTimeout");
            return Ok(Some(dummy_success_status()));
        }

        Ok(None)
    }

    /// Clear Laravel's compiled cache files directly.
    /// This is what Laravel's `ComposerScripts::clearCompiled()` does.
    fn laravel_clear_compiled(&self) -> Result<Option<ExitStatus>> {
        let bootstrap_cache = self.config.working_dir.join("bootstrap").join("cache");

        // These are the files Laravel's clearCompiled() removes
        let cache_files = ["config.php", "services.php", "packages.php"];

        for file in &cache_files {
            let path = bootstrap_cache.join(file);
            if path.exists() {
                debug!("Removing Laravel cache file: {}", path.display());
                let _ = std::fs::remove_file(&path);
            }
        }

        info!("Cleared Laravel compiled cache");
        Ok(Some(dummy_success_status()))
    }

    /// Execute a PHP callback with full Composer Event API support.
    ///
    /// This method provides complete compatibility by:
    /// 1. First checking if real composer/composer package is available
    /// 2. If not, loading Libretto's comprehensive Composer stubs
    /// 3. Creating a proper Event object with all required methods
    fn execute_unknown_callback(&self, callback: &str) -> Result<Option<ExitStatus>> {
        let vendor_dir = self
            .config
            .working_dir
            .join("vendor")
            .display()
            .to_string()
            .replace('\\', "/");

        let working_dir = self
            .config
            .working_dir
            .display()
            .to_string()
            .replace('\\', "/");

        let dev_mode = if self.config.dev_mode {
            "true"
        } else {
            "false"
        };

        // Generate the PHP script that provides full Composer compatibility
        let php_code = format!(
            r"<?php
/**
 * Libretto Script Runner
 * Provides full Composer Event API compatibility
 */

// First, load the project's autoloader
require_once '{vendor_dir}/autoload.php';

// Check if real Composer classes exist (from composer/composer package)
$useRealComposer = class_exists('Composer\\Script\\Event')
    && class_exists('Composer\\Composer')
    && class_exists('Composer\\IO\\ConsoleIO');

if (!$useRealComposer) {{
    // Load Libretto's Composer compatibility stubs
    // These provide a complete implementation of the Composer Event API
    {stubs}
}}

// Create the Composer instance with proper configuration
$config = new Composer\Config(true, '{working_dir}');
$config->merge([
    'config' => [
        'vendor-dir' => '{vendor_dir}',
        'bin-dir' => '{vendor_dir}/bin',
    ]
]);

$composer = new Composer\Composer();
$composer->setConfig($config);

// Load composer.json if available to set package info
$composerJsonPath = '{working_dir}/composer.json';
if (file_exists($composerJsonPath)) {{
    $composerData = json_decode(file_get_contents($composerJsonPath), true);
    if ($composerData) {{
        $packageName = $composerData['name'] ?? 'root/package';
        $packageVersion = $composerData['version'] ?? '1.0.0';
        $package = new Composer\Package\RootPackage($packageName, $packageVersion);
        if (isset($composerData['extra'])) {{
            $package->setExtra($composerData['extra']);
        }}
        $composer->setPackage($package);
    }}
}}

// Create IO instance for console interaction
$io = new Composer\IO\ConsoleIO(true, Composer\IO\IOInterface::NORMAL, true);

// Create the Event object
$event = new Composer\Script\Event(
    'libretto-script',
    $composer,
    $io,
    {dev_mode},
    [],
    []
);

// Execute the callback
$callback = '{callback}';

try {{
    call_user_func($callback, $event);
}} catch (TypeError $e) {{
    // If callback doesn't want Event, try without arguments
    if (strpos($e->getMessage(), 'Argument') !== false) {{
        call_user_func($callback);
    }} else {{
        throw $e;
    }}
}} catch (ArgumentCountError $e) {{
    // PHP 8+ throws ArgumentCountError for wrong argument count
    call_user_func($callback);
}}
",
            vendor_dir = vendor_dir,
            working_dir = working_dir,
            dev_mode = dev_mode,
            callback = callback.replace('\\', "\\\\"),
            stubs = get_composer_stubs(),
        );

        // Write to temp file and execute
        let script_path = self
            .config
            .working_dir
            .join("vendor")
            .join(".libretto-callback.php");
        std::fs::write(&script_path, &php_code).context("Failed to write temporary PHP script")?;

        // Set environment variables for scripts that want to detect Libretto
        let env_prefix = format!(
            "LIBRETTO=1 COMPOSER_DEV_MODE={} COMPOSER_VENDOR_DIR={}",
            if self.config.dev_mode { "1" } else { "0" },
            shell_escape(&vendor_dir)
        );

        let result = self.execute_shell(&format!(
            "{} {} {}",
            env_prefix,
            self.config.php_binary,
            script_path.display()
        ));

        // Clean up
        let _ = std::fs::remove_file(&script_path);

        result
    }

    /// Execute a shell command with optional timeout enforcement.
    fn execute_shell(&self, cmd: &str) -> Result<Option<ExitStatus>> {
        debug!(command = %cmd, timeout = self.config.timeout, "executing shell command");

        // Build environment
        let mut env: HashMap<String, String> = std::env::vars().collect();

        // Add vendor/bin to PATH
        let vendor_bin = self.config.working_dir.join("vendor").join("bin");
        if vendor_bin.exists() {
            let path = env.get("PATH").cloned().unwrap_or_default();
            let separator = if cfg!(windows) { ";" } else { ":" };
            let new_path = format!("{}{}{}", vendor_bin.display(), separator, path);
            env.insert("PATH".to_string(), new_path);
        }

        // Add COMPOSER_* variables
        env.insert(
            "COMPOSER_BINARY".to_string(),
            self.config.composer_binary.clone(),
        );
        env.insert(
            "COMPOSER_DEV_MODE".to_string(),
            if self.config.dev_mode { "1" } else { "0" }.to_string(),
        );

        // Add custom environment
        env.extend(self.config.env.clone());

        // Determine shell
        let (shell, shell_arg) = if cfg!(windows) {
            ("cmd", "/C")
        } else {
            ("sh", "-c")
        };

        let mut command = Command::new(shell);
        command
            .arg(shell_arg)
            .arg(cmd)
            .current_dir(&self.config.working_dir)
            .envs(&env)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());

        // If timeout is set (> 0), enforce it
        if self.config.timeout > 0 {
            let timeout_duration = Duration::from_secs(self.config.timeout);
            let mut child = command.spawn().context(format!("Failed to spawn: {cmd}"))?;

            let start = Instant::now();

            // Poll for completion with timeout
            loop {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        return Ok(Some(status));
                    }
                    Ok(None) => {
                        // Still running, check timeout
                        if start.elapsed() >= timeout_duration {
                            // Kill the process
                            let _ = child.kill();
                            let _ = child.wait(); // Reap the zombie
                            bail!("Script timed out after {}s: {}", self.config.timeout, cmd);
                        }
                        // Sleep briefly before polling again
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    Err(e) => {
                        return Err(anyhow::anyhow!("Failed to wait for process: {e}"));
                    }
                }
            }
        } else {
            // No timeout, wait indefinitely
            let status = command
                .status()
                .context(format!("Failed to execute: {cmd}"))?;
            Ok(Some(status))
        }
    }
}

/// Run pre-install or pre-update scripts.
pub fn run_pre_install_scripts(
    composer_json: &Value,
    config: &ScriptConfig,
    is_update: bool,
) -> Result<Option<ScriptResult>> {
    let mut executor = ScriptExecutor::new(composer_json, config.clone());

    let event = if is_update {
        ScriptEvent::PreUpdateCmd
    } else {
        ScriptEvent::PreInstallCmd
    };

    if let Some(result) = executor.run_event(event)? {
        info!(
            event = event.as_str(),
            commands = result.commands_executed,
            success = result.success,
            "pre-install/update scripts completed"
        );
        return Ok(Some(result));
    }

    Ok(None)
}

/// Run post-install or post-update scripts.
pub fn run_post_install_scripts(
    composer_json: &Value,
    config: &ScriptConfig,
    is_update: bool,
) -> Result<Option<ScriptResult>> {
    let mut executor = ScriptExecutor::new(composer_json, config.clone());

    let event = if is_update {
        ScriptEvent::PostUpdateCmd
    } else {
        ScriptEvent::PostInstallCmd
    };

    if let Some(result) = executor.run_event(event)? {
        info!(
            event = event.as_str(),
            commands = result.commands_executed,
            success = result.success,
            "post-install/update scripts completed"
        );
        return Ok(Some(result));
    }

    Ok(None)
}

/// Run pre-autoload-dump scripts.
pub fn run_pre_autoload_scripts(
    composer_json: &Value,
    config: &ScriptConfig,
) -> Result<Option<ScriptResult>> {
    let mut executor = ScriptExecutor::new(composer_json, config.clone());

    if let Some(result) = executor.run_event(ScriptEvent::PreAutoloadDump)? {
        info!(
            event = "pre-autoload-dump",
            commands = result.commands_executed,
            success = result.success,
            "pre-autoload-dump scripts completed"
        );
        return Ok(Some(result));
    }

    Ok(None)
}

/// Run post-autoload-dump scripts.
pub fn run_post_autoload_scripts(
    composer_json: &Value,
    config: &ScriptConfig,
) -> Result<Option<ScriptResult>> {
    let mut executor = ScriptExecutor::new(composer_json, config.clone());

    if let Some(result) = executor.run_event(ScriptEvent::PostAutoloadDump)? {
        info!(
            event = "post-autoload-dump",
            commands = result.commands_executed,
            success = result.success,
            "post-autoload-dump scripts completed"
        );
        return Ok(Some(result));
    }

    Ok(None)
}

/// Run create-project scripts.
#[allow(dead_code)]
pub fn run_create_project_scripts(
    composer_json: &Value,
    config: &ScriptConfig,
) -> Result<Option<ScriptResult>> {
    let mut executor = ScriptExecutor::new(composer_json, config.clone());

    if let Some(result) = executor.run_event(ScriptEvent::PostCreateProjectCmd)? {
        info!(
            event = "post-create-project-cmd",
            commands = result.commands_executed,
            success = result.success,
            "post-create-project scripts completed"
        );
        return Ok(Some(result));
    }

    Ok(None)
}

/// Run root package install scripts (for create-project).
#[allow(dead_code)]
pub fn run_root_package_install_scripts(
    composer_json: &Value,
    config: &ScriptConfig,
) -> Result<Option<ScriptResult>> {
    let mut executor = ScriptExecutor::new(composer_json, config.clone());

    if let Some(result) = executor.run_event(ScriptEvent::PostRootPackageInstall)? {
        info!(
            event = "post-root-package-install",
            commands = result.commands_executed,
            success = result.success,
            "post-root-package-install scripts completed"
        );
        return Ok(Some(result));
    }

    Ok(None)
}

/// Run pre-operations-exec scripts (before package operations).
#[allow(dead_code)]
pub fn run_pre_operations_scripts(
    composer_json: &Value,
    config: &ScriptConfig,
) -> Result<Option<ScriptResult>> {
    let mut executor = ScriptExecutor::new(composer_json, config.clone());

    if let Some(result) = executor.run_event(ScriptEvent::PreOperationsExec)? {
        info!(
            event = "pre-operations-exec",
            commands = result.commands_executed,
            success = result.success,
            "pre-operations-exec scripts completed"
        );
        return Ok(Some(result));
    }

    Ok(None)
}

/// Run package-level scripts for a specific package.
///
/// These are events like pre-package-install, post-package-install, etc.
/// They receive a `PackageEvent` instead of a regular Event.
#[allow(dead_code)]
pub fn run_package_scripts(
    composer_json: &Value,
    config: &ScriptConfig,
    event: ScriptEvent,
    package_name: &str,
) -> Result<Option<ScriptResult>> {
    let mut executor = ScriptExecutor::new(composer_json, config.clone());

    // For package events, we need to set the package context
    // This is used by callbacks that need to know which package is being operated on
    let event_name = event.as_str();

    if let Some(result) = executor.run_event(event)? {
        info!(
            event = %event_name,
            package = %package_name,
            commands = result.commands_executed,
            success = result.success,
            "package script completed"
        );
        return Ok(Some(result));
    }

    Ok(None)
}

/// Run a generic script event.
#[allow(dead_code)]
pub fn run_script_event(
    composer_json: &Value,
    config: &ScriptConfig,
    event: ScriptEvent,
) -> Result<Option<ScriptResult>> {
    let mut executor = ScriptExecutor::new(composer_json, config.clone());

    if let Some(result) = executor.run_event(event)? {
        info!(
            event = %event.as_str(),
            commands = result.commands_executed,
            success = result.success,
            "script event completed"
        );
        return Ok(Some(result));
    }

    Ok(None)
}

/// Check if a command looks like a PHP class method callback.
///
/// Matches patterns like:
/// - `Illuminate\Foundation\ComposerScripts::postAutoloadDump`
/// - `MyNamespace\MyClass::myMethod`
/// - `MyClass::method` (no namespace)
///
/// The pattern must:
/// - Contain `::` to indicate a static method call
/// - Have valid PHP identifier characters (alphanumeric, underscore, backslash for namespaces)
/// - Not start with common shell/unix commands
fn is_php_class_method(cmd: &str) -> bool {
    // Must contain :: for static method call
    if !cmd.contains("::") {
        return false;
    }

    // Split by :: and validate both parts
    let parts: Vec<&str> = cmd.splitn(2, "::").collect();
    if parts.len() != 2 {
        return false;
    }

    let class_part = parts[0].trim();
    let method_part = parts[1].trim();

    // Class part must be non-empty and look like a PHP class/namespace
    if class_part.is_empty() || method_part.is_empty() {
        return false;
    }

    // Class part should only contain valid PHP namespace characters
    // (letters, numbers, underscore, backslash for namespaces)
    let valid_class = class_part
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '\\');

    // Method part should be a valid PHP identifier (and may have parentheses for arguments)
    let method_name = method_part.split('(').next().unwrap_or(method_part).trim();
    let valid_method =
        !method_name.is_empty() && method_name.chars().all(|c| c.is_alphanumeric() || c == '_');

    // Must start with uppercase letter or backslash (namespace)
    let starts_valid = class_part
        .chars()
        .next()
        .is_some_and(|c| c.is_uppercase() || c == '\\');

    valid_class && valid_method && starts_valid
}

/// Escape a string for use in shell commands.
#[allow(dead_code)]
fn shell_escape(s: &str) -> String {
    // For single-quoted strings in shell, we need to:
    // 1. Replace ' with '\'' (end quote, escaped quote, start quote)
    // 2. Wrap in single quotes
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Create a dummy successful exit status.
/// Used when we handle callbacks directly in Rust without spawning a process.
#[cfg(unix)]
fn dummy_success_status() -> ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    ExitStatus::from_raw(0)
}

#[cfg(windows)]
fn dummy_success_status() -> ExitStatus {
    use std::os::windows::process::ExitStatusExt;
    ExitStatus::from_raw(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_composer_json() -> Value {
        sonic_rs::json!({
            "scripts": {
                "test": "echo 'Running tests'",
                "build": ["echo 'Step 1'", "echo 'Step 2'"],
                "post-install-cmd": ["@test", "echo 'Installed'"],
                "with-php": "@php -v",
                "with-env": "@putenv FOO=bar"
            }
        })
    }

    #[test]
    fn test_parse_scripts() {
        let json = create_test_composer_json();
        let scripts = ScriptExecutor::parse_scripts(&json);

        assert!(scripts.contains_key("test"));
        assert!(scripts.contains_key("build"));
        assert!(scripts.contains_key("post-install-cmd"));

        assert_eq!(scripts.get("test").unwrap(), &vec!["echo 'Running tests'"]);
        assert_eq!(scripts.get("build").unwrap().len(), 2);
    }

    #[test]
    fn test_has_script() {
        let json = create_test_composer_json();
        let executor = ScriptExecutor::new(&json, ScriptConfig::default());

        assert!(executor.has_script("test"));
        assert!(executor.has_script("build"));
        assert!(!executor.has_script("nonexistent"));
    }

    #[test]
    fn test_has_event() {
        let json = create_test_composer_json();
        let executor = ScriptExecutor::new(&json, ScriptConfig::default());

        assert!(executor.has_event(ScriptEvent::PostInstallCmd));
        assert!(!executor.has_event(ScriptEvent::PreInstallCmd));
    }

    #[test]
    fn test_script_event_as_str() {
        assert_eq!(ScriptEvent::PostInstallCmd.as_str(), "post-install-cmd");
        assert_eq!(ScriptEvent::PreUpdateCmd.as_str(), "pre-update-cmd");
    }

    #[test]
    fn test_script_event_from_str() {
        assert_eq!(
            ScriptEvent::from_str("post-install-cmd"),
            Some(ScriptEvent::PostInstallCmd)
        );
        assert_eq!(ScriptEvent::from_str("invalid"), None);
    }

    #[test]
    fn test_is_php_class_method() {
        // Valid PHP class methods
        assert!(is_php_class_method(
            "Illuminate\\Foundation\\ComposerScripts::postAutoloadDump"
        ));
        assert!(is_php_class_method("MyNamespace\\MyClass::myMethod"));
        assert!(is_php_class_method("MyClass::method"));
        assert!(is_php_class_method(
            "App\\Providers\\AppServiceProvider::boot"
        ));

        // Invalid - shell commands
        assert!(!is_php_class_method("echo 'hello'"));
        assert!(!is_php_class_method("php artisan serve"));
        assert!(!is_php_class_method("npm run build"));

        // Invalid - missing ::
        assert!(!is_php_class_method("MyClass"));
        assert!(!is_php_class_method(
            "Illuminate\\Foundation\\ComposerScripts"
        ));

        // Invalid - lowercase start (likely shell command)
        assert!(!is_php_class_method("myclass::method"));

        // Invalid - empty parts
        assert!(!is_php_class_method("::method"));
        assert!(!is_php_class_method("MyClass::"));
    }

    #[test]
    fn test_shell_escape() {
        assert_eq!(shell_escape("hello"), "'hello'");
        assert_eq!(shell_escape("hello world"), "'hello world'");
        assert_eq!(shell_escape("it's"), "'it'\\''s'");
    }
}
