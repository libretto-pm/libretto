//! Platform requirement validation for PHP environments.
//!
//! This module provides high-performance validation of PHP platform requirements:
//! - PHP version checking
//! - Extension availability and version checking
//! - Library version checking (lib-*)
//!
//! # Performance
//!
//! - **Parallel detection**: Uses rayon for parallel extension checking
//! - **Caching**: Caches detected platform state
//! - **Lazy detection**: Only checks required extensions
//!
//! # Usage
//!
//! ```rust,ignore
//! use libretto_cli::platform::{PlatformValidator, ValidationResult};
//!
//! let validator = PlatformValidator::detect().await?;
//!
//! let requirements = vec![
//!     ("php", ">=8.0"),
//!     ("ext-json", "*"),
//!     ("ext-mbstring", "*"),
//! ];
//!
//! let result = validator.validate(&requirements)?;
//! if !result.is_satisfied() {
//!     for error in &result.errors {
//!         println!("Missing: {} (required {})", error.name, error.constraint);
//!     }
//! }
//! ```

use anyhow::Result;
use rayon::prelude::*;
use serde::Serialize;
use std::collections::HashMap;
use std::io;
use std::process::{Command, Output};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

/// Detected PHP platform information.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct DetectedPlatform {
    /// PHP version.
    pub php_version: Option<String>,
    /// Loaded PHP extensions with versions.
    pub extensions: HashMap<String, Option<String>>,
    /// Detected library versions.
    pub libraries: HashMap<String, Option<String>>,
    /// PHP binary path.
    pub php_binary: String,
    /// Detection timestamp.
    pub detected_at: Instant,
}

impl Default for DetectedPlatform {
    fn default() -> Self {
        Self {
            php_version: None,
            extensions: HashMap::new(),
            libraries: HashMap::new(),
            php_binary: "php".to_string(),
            detected_at: Instant::now(),
        }
    }
}

/// A platform requirement validation error.
#[derive(Debug, Clone, Serialize)]
pub struct ValidationError {
    /// Requirement name (php, ext-*, lib-*).
    pub name: String,
    /// Required constraint.
    pub constraint: String,
    /// Installed version (if any).
    pub installed: Option<String>,
    /// Error message.
    pub message: String,
    /// Packages that require this.
    pub required_by: Vec<String>,
}

/// Result of platform validation.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ValidationResult {
    /// All requirements that were checked.
    pub checked: Vec<(String, String, bool)>,
    /// Validation errors.
    pub errors: Vec<ValidationError>,
    /// Warnings (non-fatal).
    pub warnings: Vec<String>,
    /// Validation duration.
    pub duration: Duration,
}

#[allow(dead_code)]
impl ValidationResult {
    /// Check if all requirements are satisfied.
    #[must_use]
    pub const fn is_satisfied(&self) -> bool {
        self.errors.is_empty()
    }

    /// Get summary message.
    #[must_use]
    pub fn summary(&self) -> String {
        if self.is_satisfied() {
            format!("All {} platform requirements satisfied", self.checked.len())
        } else {
            format!(
                "{} of {} platform requirements failed",
                self.errors.len(),
                self.checked.len()
            )
        }
    }
}

/// Platform validator for checking PHP requirements.
#[allow(dead_code)]
pub struct PlatformValidator {
    /// Detected platform info.
    platform: DetectedPlatform,
    /// PHP binary to use.
    php_binary: String,
}

#[allow(dead_code)]
impl PlatformValidator {
    /// Run a PHP command and, on Windows, retry through `cmd /C` when a shim
    /// such as `php.bat` cannot be executed directly.
    fn run_php_command(&self, args: &[&str]) -> io::Result<Output> {
        match Command::new(&self.php_binary).args(args).output() {
            Ok(output) => {
                #[cfg(windows)]
                {
                    if !output.status.success() {
                        debug!(
                            php_binary = %self.php_binary,
                            status = ?output.status.code(),
                            "direct PHP execution failed, retrying through cmd /C"
                        );
                        return Command::new("cmd")
                            .arg("/C")
                            .arg(&self.php_binary)
                            .args(args)
                            .output();
                    }
                }

                Ok(output)
            }
            Err(err) => {
                #[cfg(windows)]
                {
                    debug!(
                        php_binary = %self.php_binary,
                        error = %err,
                        "direct PHP execution failed, retrying through cmd /C"
                    );
                    return Command::new("cmd")
                        .arg("/C")
                        .arg(&self.php_binary)
                        .args(args)
                        .output();
                }

                #[cfg(not(windows))]
                {
                    Err(err)
                }
            }
        }
    }

    /// Create a new validator with the default PHP binary.
    pub fn new() -> Self {
        Self {
            platform: DetectedPlatform::default(),
            php_binary: "php".to_string(),
        }
    }

    /// Create with a specific PHP binary path.
    pub fn with_php_binary(php_binary: impl Into<String>) -> Self {
        let php_binary = php_binary.into();
        Self {
            platform: DetectedPlatform {
                php_binary: php_binary.clone(),
                ..Default::default()
            },
            php_binary,
        }
    }

    /// Detect the platform (PHP version, extensions).
    ///
    /// This runs PHP commands to detect the installed environment.
    pub fn detect(&mut self) -> Result<&DetectedPlatform> {
        let start = Instant::now();
        info!(php_binary = %self.php_binary, "detecting PHP platform");

        // Detect PHP version
        self.platform.php_version = self.detect_php_version()?;
        debug!(version = ?self.platform.php_version, "detected PHP version");

        // Detect loaded extensions
        self.platform.extensions = self.detect_extensions()?;
        debug!(
            count = self.platform.extensions.len(),
            "detected PHP extensions"
        );

        // Detect common libraries
        self.platform.libraries = self.detect_libraries();

        self.platform.detected_at = Instant::now();

        info!(
            version = ?self.platform.php_version,
            extensions = self.platform.extensions.len(),
            duration_ms = start.elapsed().as_millis(),
            "platform detection complete"
        );

        Ok(&self.platform)
    }

    /// Validate requirements against the detected platform.
    pub fn validate<S: AsRef<[String]>>(
        &self,
        requirements: &[(&str, &str, S)],
    ) -> Result<ValidationResult> {
        let start = Instant::now();
        let mut checked: Vec<(String, String, bool)> = Vec::new();
        let mut errors: Vec<ValidationError> = Vec::new();
        let warnings: Vec<String> = Vec::new();

        for (name, constraint, required_by) in requirements {
            let (installed, satisfied) = self.check_requirement(name, constraint)?;

            checked.push((name.to_string(), constraint.to_string(), satisfied));

            if !satisfied {
                errors.push(ValidationError {
                    name: name.to_string(),
                    constraint: constraint.to_string(),
                    installed,
                    message: format!(
                        "{} {} is required but {}",
                        name,
                        constraint,
                        if self.platform.php_version.is_none() && *name == "php" {
                            "PHP is not installed".to_string()
                        } else {
                            "the installed version does not satisfy the constraint".to_string()
                        }
                    ),
                    required_by: required_by.as_ref().to_vec(),
                });
            }
        }

        Ok(ValidationResult {
            checked,
            errors,
            warnings,
            duration: start.elapsed(),
        })
    }

    /// Check a single requirement.
    fn check_requirement(&self, name: &str, constraint: &str) -> Result<(Option<String>, bool)> {
        if name == "php" {
            return self.check_php_version(constraint);
        }

        if let Some(ext_name) = name.strip_prefix("ext-") {
            return self.check_extension(ext_name, constraint);
        }

        if let Some(lib_name) = name.strip_prefix("lib-") {
            return self.check_library(lib_name, constraint);
        }

        // Composer plugin API - always satisfied by Libretto
        if name == "composer-plugin-api" || name == "composer-runtime-api" {
            return Ok((Some("2.6.0".to_string()), true));
        }

        // Unknown requirement type - warn but allow
        warn!(name = %name, "unknown platform requirement type");
        Ok((None, true))
    }

    /// Check PHP version requirement.
    fn check_php_version(&self, constraint: &str) -> Result<(Option<String>, bool)> {
        let installed = self.platform.php_version.clone();

        if constraint == "*" {
            return Ok((installed.clone(), installed.is_some()));
        }

        let installed_version = match &installed {
            Some(v) => v,
            None => return Ok((None, false)),
        };

        let satisfied = check_version_constraint(installed_version, constraint);
        Ok((installed, satisfied))
    }

    /// Check extension requirement.
    fn check_extension(&self, ext_name: &str, constraint: &str) -> Result<(Option<String>, bool)> {
        let ext_lower = ext_name.to_lowercase();

        // Check if extension is loaded
        let installed = self.platform.extensions.get(&ext_lower).cloned().flatten();

        // If extension exists and constraint is *, it's satisfied
        if constraint == "*" {
            return Ok((installed, self.platform.extensions.contains_key(&ext_lower)));
        }

        // Check version constraint
        match &installed {
            Some(version) => {
                let satisfied = check_version_constraint(version, constraint);
                Ok((Some(version.clone()), satisfied))
            }
            None => {
                // Extension not loaded
                Ok((None, false))
            }
        }
    }

    /// Check library requirement.
    fn check_library(&self, lib_name: &str, constraint: &str) -> Result<(Option<String>, bool)> {
        let lib_lower = lib_name.to_lowercase();

        let installed = self.platform.libraries.get(&lib_lower).cloned().flatten();

        if constraint == "*" {
            return Ok((installed.clone(), installed.is_some()));
        }

        match &installed {
            Some(version) => {
                let satisfied = check_version_constraint(version, constraint);
                Ok((Some(version.clone()), satisfied))
            }
            None => Ok((None, false)),
        }
    }

    /// Detect PHP version.
    fn detect_php_version(&self) -> Result<Option<String>> {
        let output = self.run_php_command(&["-r", "echo PHP_VERSION;"]);

        match output {
            Ok(output) if output.status.success() => {
                let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
                Ok(Some(version))
            }
            Ok(_) => Ok(None),
            Err(e) => {
                debug!(error = %e, "PHP not found");
                Ok(None)
            }
        }
    }

    /// Detect loaded PHP extensions.
    fn detect_extensions(&self) -> Result<HashMap<String, Option<String>>> {
        let mut extensions = HashMap::new();

        // Get list of loaded extensions
        let output = self.run_php_command(&["-r", "echo json_encode(get_loaded_extensions());"]);

        let loaded: Vec<String> = match output {
            Ok(output) if output.status.success() => {
                let json = String::from_utf8_lossy(&output.stdout);
                sonic_rs::from_str(&json).unwrap_or_default()
            }
            _ => Vec::new(),
        };

        // Get versions for each extension in parallel
        let versions: Vec<(String, Option<String>)> = loaded
            .par_iter()
            .map(|ext| {
                let ext_lower = ext.to_lowercase();
                let version = self.get_extension_version(&ext_lower);
                (ext_lower, version)
            })
            .collect();

        for (ext, version) in versions {
            extensions.insert(ext, version);
        }

        Ok(extensions)
    }

    /// Get version of a specific extension.
    fn get_extension_version(&self, ext_name: &str) -> Option<String> {
        let code = format!(
            "echo phpversion('{ext_name}') ?: (extension_loaded('{ext_name}') ? '0.0.0' : '');"
        );

        let output = self.run_php_command(&["-r", code.as_str()]).ok()?;

        if output.status.success() {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if version.is_empty() {
                None
            } else {
                Some(version)
            }
        } else {
            None
        }
    }

    /// Detect common libraries.
    fn detect_libraries(&self) -> HashMap<String, Option<String>> {
        let mut libraries = HashMap::new();

        // OpenSSL
        if let Some(version) = self.detect_openssl_version() {
            libraries.insert("openssl".to_string(), Some(version));
        }

        // cURL
        if let Some(version) = self.detect_curl_version() {
            libraries.insert("curl".to_string(), Some(version));
        }

        // ICU (via intl extension)
        if let Some(version) = self.detect_icu_version() {
            libraries.insert("icu".to_string(), Some(version));
        }

        // libxml
        if let Some(version) = self.detect_libxml_version() {
            libraries.insert("libxml".to_string(), Some(version));
        }

        libraries
    }

    fn detect_openssl_version(&self) -> Option<String> {
        let output = Command::new("openssl").args(["version"]).output().ok()?;

        if output.status.success() {
            let output_str = String::from_utf8_lossy(&output.stdout);
            output_str.split_whitespace().nth(1).map(String::from)
        } else {
            None
        }
    }

    fn detect_curl_version(&self) -> Option<String> {
        let output = Command::new("curl").args(["--version"]).output().ok()?;

        if output.status.success() {
            let output_str = String::from_utf8_lossy(&output.stdout);
            output_str.split_whitespace().nth(1).map(String::from)
        } else {
            None
        }
    }

    fn detect_icu_version(&self) -> Option<String> {
        let output = self
            .run_php_command(&[
                "-r",
                "echo defined('INTL_ICU_VERSION') ? INTL_ICU_VERSION : '';",
            ])
            .ok()?;

        if output.status.success() {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if version.is_empty() {
                None
            } else {
                Some(version)
            }
        } else {
            None
        }
    }

    fn detect_libxml_version(&self) -> Option<String> {
        let output = self
            .run_php_command(&["-r", "echo LIBXML_DOTTED_VERSION;"])
            .ok()?;

        if output.status.success() {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if version.is_empty() {
                None
            } else {
                Some(version)
            }
        } else {
            None
        }
    }

    /// Get the detected platform.
    #[must_use]
    pub const fn platform(&self) -> &DetectedPlatform {
        &self.platform
    }
}

impl Default for PlatformValidator {
    fn default() -> Self {
        Self::new()
    }
}

/// Check if an installed version satisfies a constraint.
///
/// Supports Composer constraint formats:
/// - Exact: 1.0.0
/// - Range: >=1.0, <2.0, >1.0 <=2.0
/// - Caret: ^1.0 (>=1.0 <2.0)
/// - Tilde: ~1.2 (>=1.2 <2.0)
/// - Wildcard: 1.0.*, *
fn check_version_constraint(installed: &str, constraint: &str) -> bool {
    let constraint = constraint.trim();

    // Wildcard matches anything
    if constraint == "*" {
        return true;
    }

    // Parse installed version
    let installed_parts: Vec<u64> = installed
        .split(|c: char| !c.is_ascii_digit())
        .filter_map(|s| s.parse().ok())
        .collect();

    if installed_parts.is_empty() {
        return false;
    }

    // Handle OR constraints
    if constraint.contains("||") {
        return constraint
            .split("||")
            .any(|part| check_version_constraint(installed, part.trim()));
    }

    // Handle AND constraints (space or comma separated)
    let parts: Vec<&str> = if constraint.contains(',') {
        constraint.split(',').map(str::trim).collect()
    } else {
        constraint.split_whitespace().collect()
    };

    if parts.len() > 1 {
        return parts
            .iter()
            .all(|part| check_version_constraint(installed, part));
    }

    let constraint = parts[0];

    // Caret constraint
    if let Some(rest) = constraint.strip_prefix('^') {
        let req_parts = parse_version_parts(rest);
        if req_parts.is_empty() {
            return true;
        }

        // ^X.Y.Z means >=X.Y.Z <(X+1).0.0
        let satisfies_lower = compare_versions(&installed_parts, &req_parts) >= 0;
        let upper: Vec<u64> = vec![req_parts[0] + 1, 0, 0];
        let satisfies_upper = compare_versions(&installed_parts, &upper) < 0;

        return satisfies_lower && satisfies_upper;
    }

    // Tilde constraint
    if let Some(rest) = constraint.strip_prefix('~') {
        let req_parts = parse_version_parts(rest);
        if req_parts.is_empty() {
            return true;
        }

        // ~X.Y means >=X.Y <X.(Y+1)
        let satisfies_lower = compare_versions(&installed_parts, &req_parts) >= 0;
        let upper: Vec<u64> = if req_parts.len() >= 2 {
            vec![req_parts[0], req_parts[1] + 1, 0]
        } else {
            vec![req_parts[0] + 1, 0, 0]
        };
        let satisfies_upper = compare_versions(&installed_parts, &upper) < 0;

        return satisfies_lower && satisfies_upper;
    }

    // Greater than or equal
    if let Some(rest) = constraint.strip_prefix(">=") {
        let req_parts = parse_version_parts(rest);
        return compare_versions(&installed_parts, &req_parts) >= 0;
    }

    // Less than or equal
    if let Some(rest) = constraint.strip_prefix("<=") {
        let req_parts = parse_version_parts(rest);
        return compare_versions(&installed_parts, &req_parts) <= 0;
    }

    // Greater than
    if let Some(rest) = constraint.strip_prefix('>') {
        let req_parts = parse_version_parts(rest);
        return compare_versions(&installed_parts, &req_parts) > 0;
    }

    // Less than
    if let Some(rest) = constraint.strip_prefix('<') {
        let req_parts = parse_version_parts(rest);
        return compare_versions(&installed_parts, &req_parts) < 0;
    }

    // Not equal
    if let Some(rest) = constraint.strip_prefix("!=") {
        let req_parts = parse_version_parts(rest);
        return compare_versions(&installed_parts, &req_parts) != 0;
    }

    // Exact match (or with = prefix)
    let constraint = constraint.strip_prefix('=').unwrap_or(constraint);
    let req_parts = parse_version_parts(constraint);

    // Handle wildcards
    if constraint.contains('*') || constraint.contains('x') {
        let prefix_len = req_parts.len();
        return installed_parts
            .iter()
            .take(prefix_len)
            .zip(req_parts.iter())
            .all(|(a, b)| a == b);
    }

    compare_versions(&installed_parts, &req_parts) == 0
}

/// Parse version string into parts.
fn parse_version_parts(version: &str) -> Vec<u64> {
    version
        .trim()
        .trim_start_matches('v')
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty() && *s != "*" && *s != "x")
        .filter_map(|s| s.parse().ok())
        .collect()
}

/// Compare two version part arrays.
fn compare_versions(a: &[u64], b: &[u64]) -> i32 {
    let max_len = a.len().max(b.len());

    for i in 0..max_len {
        let av = a.get(i).copied().unwrap_or(0);
        let bv = b.get(i).copied().unwrap_or(0);

        match av.cmp(&bv) {
            std::cmp::Ordering::Less => return -1,
            std::cmp::Ordering::Greater => return 1,
            std::cmp::Ordering::Equal => continue,
        }
    }

    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_constraint_exact() {
        assert!(check_version_constraint("8.1.0", "8.1.0"));
        assert!(!check_version_constraint("8.1.0", "8.2.0"));
    }

    #[test]
    fn test_version_constraint_wildcard() {
        assert!(check_version_constraint("8.1.0", "*"));
        assert!(check_version_constraint("7.4.0", "*"));
    }

    #[test]
    fn test_version_constraint_caret() {
        assert!(check_version_constraint("8.1.0", "^8.0"));
        assert!(check_version_constraint("8.2.5", "^8.0"));
        assert!(!check_version_constraint("9.0.0", "^8.0"));
        assert!(!check_version_constraint("7.4.0", "^8.0"));
    }

    #[test]
    fn test_version_constraint_tilde() {
        assert!(check_version_constraint("8.1.0", "~8.1"));
        assert!(check_version_constraint("8.1.5", "~8.1"));
        assert!(!check_version_constraint("8.2.0", "~8.1"));
        assert!(!check_version_constraint("8.0.0", "~8.1"));
    }

    #[test]
    fn test_version_constraint_range() {
        assert!(check_version_constraint("8.1.0", ">=8.0"));
        assert!(check_version_constraint("8.0.0", ">=8.0"));
        assert!(!check_version_constraint("7.4.0", ">=8.0"));

        assert!(check_version_constraint("8.1.0", "<9.0"));
        assert!(!check_version_constraint("9.0.0", "<9.0"));
    }

    #[test]
    fn test_version_constraint_or() {
        assert!(check_version_constraint("7.4.0", "^7.4 || ^8.0"));
        assert!(check_version_constraint("8.1.0", "^7.4 || ^8.0"));
        assert!(!check_version_constraint("7.3.0", "^7.4 || ^8.0"));
    }

    #[test]
    fn test_version_constraint_and() {
        assert!(check_version_constraint("8.1.0", ">=8.0 <9.0"));
        assert!(check_version_constraint("8.1.0", ">=8.0, <9.0"));
        assert!(!check_version_constraint("9.0.0", ">=8.0 <9.0"));
        assert!(!check_version_constraint("7.4.0", ">=8.0 <9.0"));
    }

    #[test]
    fn test_parse_version_parts() {
        assert_eq!(parse_version_parts("8.1.0"), vec![8, 1, 0]);
        assert_eq!(parse_version_parts("v8.1.0"), vec![8, 1, 0]);
        assert_eq!(parse_version_parts("8.1"), vec![8, 1]);
        assert_eq!(parse_version_parts("8.1.*"), vec![8, 1]);
    }

    #[test]
    fn test_compare_versions() {
        assert_eq!(compare_versions(&[8, 1, 0], &[8, 1, 0]), 0);
        assert_eq!(compare_versions(&[8, 1, 1], &[8, 1, 0]), 1);
        assert_eq!(compare_versions(&[8, 0, 0], &[8, 1, 0]), -1);
        assert_eq!(compare_versions(&[9, 0, 0], &[8, 1, 0]), 1);
    }
}
