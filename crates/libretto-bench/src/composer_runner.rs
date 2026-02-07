//! Composer comparison runner for benchmarks.
//!
//! This module provides utilities to run Composer commands and compare
//! their performance against Libretto.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

/// Result of running a Composer command.
#[derive(Debug, Clone)]
pub struct ComposerResult {
    /// Time taken to execute the command.
    pub duration: Duration,
    /// Exit code of the process.
    pub exit_code: i32,
    /// Standard output.
    pub stdout: String,
    /// Standard error.
    pub stderr: String,
}

/// Runner for Composer commands.
#[derive(Debug, Default)]
pub struct ComposerRunner {
    /// Path to composer executable (defaults to "composer").
    composer_path: Option<String>,
    /// Working directory.
    working_dir: Option<String>,
}

impl ComposerRunner {
    /// Create a new Composer runner.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the path to the Composer executable.
    #[must_use]
    pub fn with_composer_path(mut self, path: impl Into<String>) -> Self {
        self.composer_path = Some(path.into());
        self
    }

    /// Set the working directory.
    #[must_use]
    pub fn with_working_dir(mut self, dir: impl Into<String>) -> Self {
        self.working_dir = Some(dir.into());
        self
    }

    /// Get the Composer executable path.
    fn composer_cmd(&self) -> &str {
        self.composer_path.as_deref().unwrap_or("composer")
    }

    /// Run a Composer command and measure its execution time.
    pub fn run(&self, args: &[&str]) -> Result<ComposerResult> {
        let start = Instant::now();

        let mut cmd = Command::new(self.composer_cmd());
        cmd.args(args);

        if let Some(dir) = &self.working_dir {
            cmd.current_dir(dir);
        }

        // Disable ANSI colors for cleaner output
        cmd.env("COMPOSER_NO_INTERACTION", "1");
        cmd.env("NO_COLOR", "1");

        let output = cmd.output().context("Failed to execute Composer")?;

        let duration = start.elapsed();

        Ok(ComposerResult {
            duration,
            exit_code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }

    /// Run `composer install` and return the result.
    pub fn install(&self) -> Result<ComposerResult> {
        self.run(&["install", "--no-progress", "--no-scripts"])
    }

    /// Run `composer update` and return the result.
    pub fn update(&self) -> Result<ComposerResult> {
        self.run(&["update", "--no-progress", "--no-scripts"])
    }

    /// Run `composer dump-autoload` and return the result.
    pub fn dump_autoload(&self) -> Result<ComposerResult> {
        self.run(&["dump-autoload", "--optimize"])
    }

    /// Check if Composer is available on the system.
    pub fn is_available(&self) -> bool {
        Command::new(self.composer_cmd())
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Get Composer version.
    pub fn version(&self) -> Result<String> {
        let result = self.run(&["--version"])?;
        Ok(result.stdout.trim().to_string())
    }
}

/// Clear Composer's cache.
pub fn clear_composer_cache() -> Result<()> {
    Command::new("composer")
        .args(["clear-cache"])
        .output()
        .context("Failed to clear Composer cache")?;
    Ok(())
}

/// Create a test composer.json file.
pub fn create_test_composer_json(dir: &Path, packages: &[(&str, &str)]) -> Result<()> {
    use std::fs;

    let require: Vec<String> = packages
        .iter()
        .map(|(name, version)| format!(r#"        "{name}": "{version}""#))
        .collect();

    let content = format!(
        r#"{{
    "name": "benchmark/test",
    "require": {{
{}
    }}
}}"#,
        require.join(",\n")
    );

    fs::write(dir.join("composer.json"), content).context("Failed to write composer.json")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_composer_runner_creation() {
        let runner = ComposerRunner::new()
            .with_composer_path("/usr/bin/composer")
            .with_working_dir("/tmp");

        assert_eq!(runner.composer_cmd(), "/usr/bin/composer");
    }
}
