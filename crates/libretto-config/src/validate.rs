//! Configuration validation with descriptive errors.

use crate::error::{ConfigError, Result};
use crate::types::{
    ComposerConfig, ComposerManifest, RepositoryConfig, RepositoryDefinition, RepositoryType,
};
use libretto_core::is_platform_package_name;
use std::path::Path;

/// Validation severity level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Informational message.
    Info,
    /// Warning (may indicate issues).
    Warning,
    /// Error (must be fixed).
    Error,
}

/// Validation issue.
#[derive(Debug, Clone)]
pub struct ValidationIssue {
    /// Severity level.
    pub severity: Severity,
    /// Issue code for programmatic handling.
    pub code: &'static str,
    /// Field path (dot-notation).
    pub field: String,
    /// Human-readable message.
    pub message: String,
    /// Suggested fix.
    pub hint: Option<String>,
}

impl ValidationIssue {
    /// Create a new error.
    #[must_use]
    pub fn error(code: &'static str, field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            code,
            field: field.into(),
            message: message.into(),
            hint: None,
        }
    }

    /// Create a new warning.
    #[must_use]
    pub fn warning(
        code: &'static str,
        field: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            severity: Severity::Warning,
            code,
            field: field.into(),
            message: message.into(),
            hint: None,
        }
    }

    /// Create a new info message.
    #[must_use]
    pub fn info(code: &'static str, field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Info,
            code,
            field: field.into(),
            message: message.into(),
            hint: None,
        }
    }

    /// Add a hint to the issue.
    #[must_use]
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

/// Validation result containing all issues.
#[derive(Debug, Default)]
pub struct ValidationResult {
    /// All validation issues.
    pub issues: Vec<ValidationIssue>,
}

impl ValidationResult {
    /// Create a new empty result.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an issue.
    pub fn add(&mut self, issue: ValidationIssue) {
        self.issues.push(issue);
    }

    /// Add multiple issues.
    pub fn extend(&mut self, issues: impl IntoIterator<Item = ValidationIssue>) {
        self.issues.extend(issues);
    }

    /// Check if there are any errors.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.issues.iter().any(|i| i.severity == Severity::Error)
    }

    /// Check if there are any warnings.
    #[must_use]
    pub fn has_warnings(&self) -> bool {
        self.issues.iter().any(|i| i.severity == Severity::Warning)
    }

    /// Get error count.
    #[must_use]
    pub fn error_count(&self) -> usize {
        self.issues
            .iter()
            .filter(|i| i.severity == Severity::Error)
            .count()
    }

    /// Get warning count.
    #[must_use]
    pub fn warning_count(&self) -> usize {
        self.issues
            .iter()
            .filter(|i| i.severity == Severity::Warning)
            .count()
    }

    /// Convert to result, failing if there are errors.
    ///
    /// # Errors
    /// Returns error if validation has errors.
    pub fn into_result(self) -> Result<()> {
        if self.has_errors() {
            let errors: Vec<String> = self
                .issues
                .iter()
                .filter(|i| i.severity == Severity::Error)
                .map(|i| format!("{}: {}", i.field, i.message))
                .collect();
            Err(ConfigError::ValidationFailed {
                count: errors.len(),
                errors,
            })
        } else {
            Ok(())
        }
    }
}

/// Configuration validator.
#[derive(Debug, Default)]
pub struct Validator {
    /// Strict mode (treat warnings as errors).
    strict: bool,
}

impl Validator {
    /// Create a new validator.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Enable strict mode.
    #[must_use]
    pub const fn strict(mut self, strict: bool) -> Self {
        self.strict = strict;
        self
    }

    /// Validate a composer.json manifest.
    #[must_use]
    pub fn validate_manifest(&self, manifest: &ComposerManifest) -> ValidationResult {
        let mut result = ValidationResult::new();

        // Validate package name
        if let Some(ref name) = manifest.name {
            self.validate_package_name(name, "name", &mut result);
        }

        // Validate version
        if let Some(ref version) = manifest.version {
            self.validate_version(version, "version", &mut result);
        }

        // Validate require sections
        if let Some(ref require) = manifest.require {
            for (pkg, constraint) in require {
                self.validate_package_name(pkg, &format!("require.{pkg}"), &mut result);
                self.validate_version_constraint(
                    constraint,
                    &format!("require.{pkg}"),
                    &mut result,
                );
            }
        }

        if let Some(ref require_dev) = manifest.require_dev {
            for (pkg, constraint) in require_dev {
                self.validate_package_name(pkg, &format!("require-dev.{pkg}"), &mut result);
                self.validate_version_constraint(
                    constraint,
                    &format!("require-dev.{pkg}"),
                    &mut result,
                );
            }
        }

        // Validate repositories
        if let Some(ref repos) = manifest.repositories {
            self.validate_repositories(repos, &mut result);
        }

        // Validate config section
        if let Some(ref config) = manifest.config {
            self.validate_config(config, &mut result);
        }

        // Validate autoload
        if let Some(ref autoload) = manifest.autoload {
            self.validate_autoload(autoload, "autoload", &mut result);
        }

        if let Some(ref autoload_dev) = manifest.autoload_dev {
            self.validate_autoload(autoload_dev, "autoload-dev", &mut result);
        }

        // Validate URLs
        if let Some(ref homepage) = manifest.homepage {
            self.validate_url(homepage, "homepage", &mut result);
        }

        result
    }

    /// Validate a config section.
    pub fn validate_config(&self, config: &ComposerConfig, result: &mut ValidationResult) {
        // Validate process-timeout
        if let Some(timeout) = config.process_timeout
            && timeout == 0
        {
            result.add(
                ValidationIssue::warning(
                    "config.process_timeout.zero",
                    "config.process-timeout",
                    "timeout of 0 disables timeout",
                )
                .with_hint("set a positive value for timeout"),
            );
        }

        // Validate cache-files-ttl
        if let Some(ttl) = config.cache_files_ttl
            && ttl < 60
        {
            result.add(
                ValidationIssue::warning(
                    "config.cache_ttl.low",
                    "config.cache-files-ttl",
                    "very low cache TTL may impact performance",
                )
                .with_hint("consider a value of at least 3600 (1 hour)"),
            );
        }

        // Validate cache-files-maxsize
        if let Some(ref maxsize) = config.cache_files_maxsize
            && let Err(e) = crate::env::parse_byte_size(maxsize)
        {
            result.add(
                ValidationIssue::error(
                    "config.cache_maxsize.invalid",
                    "config.cache-files-maxsize",
                    format!("invalid size: {e}"),
                )
                .with_hint("use format like '300MiB' or '1G'"),
            );
        }

        // Validate secure-http with disable-tls
        if config.disable_tls == Some(true) {
            result.add(
                ValidationIssue::warning(
                    "config.disable_tls",
                    "config.disable-tls",
                    "TLS verification is disabled",
                )
                .with_hint("this is a security risk and should only be used for testing"),
            );
        }

        // Validate github-protocols
        if let Some(ref protocols) = config.github_protocols
            && protocols.is_empty()
        {
            result.add(ValidationIssue::error(
                "config.github_protocols.empty",
                "config.github-protocols",
                "at least one protocol is required",
            ));
        }

        // Validate vendor-dir
        if let Some(ref vendor) = config.vendor_dir
            && vendor.is_empty()
        {
            result.add(ValidationIssue::error(
                "config.vendor_dir.empty",
                "config.vendor-dir",
                "vendor directory cannot be empty",
            ));
        }
    }

    /// Validate package name format.
    fn validate_package_name(&self, name: &str, field: &str, result: &mut ValidationResult) {
        // Check for vendor/package format
        if !name.contains('/') && !is_platform_package_name(name) {
            result.add(
                ValidationIssue::error(
                    "package.name.format",
                    field,
                    "package name must be in vendor/package format",
                )
                .with_hint("use format like 'vendor/package-name'"),
            );
            return;
        }

        // Validate characters
        if !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '/')
        {
            result.add(
                ValidationIssue::error(
                    "package.name.chars",
                    field,
                    "package name contains invalid characters",
                )
                .with_hint("use only alphanumeric characters, hyphens, underscores, and dots"),
            );
        }

        // Check case
        if name != name.to_lowercase() {
            result.add(
                ValidationIssue::warning(
                    "package.name.case",
                    field,
                    "package names should be lowercase",
                )
                .with_hint("use lowercase for consistency"),
            );
        }
    }

    /// Validate version string.
    fn validate_version(&self, version: &str, field: &str, result: &mut ValidationResult) {
        // Check for common issues
        if version.starts_with('v') {
            result.add(
                ValidationIssue::warning(
                    "version.prefix",
                    field,
                    "version should not have 'v' prefix",
                )
                .with_hint("use '1.0.0' instead of 'v1.0.0'"),
            );
        }

        // Validate semver-ish format
        let parts: Vec<&str> = version.split('.').collect();
        if parts.is_empty() || parts.len() > 4 {
            result.add(
                ValidationIssue::error("version.format", field, "invalid version format")
                    .with_hint("use semver format like '1.0.0'"),
            );
        }
    }

    /// Validate version constraint.
    fn validate_version_constraint(
        &self,
        constraint: &str,
        field: &str,
        result: &mut ValidationResult,
    ) {
        let constraint = constraint.trim();

        // Empty constraint
        if constraint.is_empty() {
            result.add(ValidationIssue::error(
                "constraint.empty",
                field,
                "version constraint cannot be empty",
            ));
            return;
        }

        // Warn about very permissive constraints
        if constraint == "*" {
            result.add(
                ValidationIssue::warning(
                    "constraint.permissive",
                    field,
                    "wildcard constraint allows any version",
                )
                .with_hint("consider using a more specific constraint"),
            );
        }

        // Check for invalid characters
        let valid_chars = |c: char| {
            c.is_ascii_alphanumeric()
                || matches!(
                    c,
                    '^' | '~' | '>' | '<' | '=' | '!' | '|' | ',' | '.' | '-' | '@' | '*' | ' '
                )
        };

        if !constraint.chars().all(valid_chars) {
            result.add(ValidationIssue::error(
                "constraint.invalid",
                field,
                "version constraint contains invalid characters",
            ));
        }
    }

    /// Validate repositories configuration.
    fn validate_repositories(
        &self,
        repos: &crate::types::Repositories,
        result: &mut ValidationResult,
    ) {
        let configs = match repos {
            crate::types::Repositories::Array(arr) => arr
                .iter()
                .enumerate()
                .map(|(i, c)| (format!("repositories[{i}]"), c))
                .collect::<Vec<_>>(),
            crate::types::Repositories::Object(obj) => obj
                .iter()
                .map(|(k, c)| (format!("repositories.{k}"), c))
                .collect(),
        };

        for (field, config) in configs {
            match config {
                RepositoryConfig::Disabled(_) => {}
                RepositoryConfig::Config(def) => {
                    self.validate_repository_definition(def, &field, result);
                }
            }
        }
    }

    /// Validate a single repository definition.
    fn validate_repository_definition(
        &self,
        def: &RepositoryDefinition,
        field: &str,
        result: &mut ValidationResult,
    ) {
        // URL is required for most types
        let needs_url = !matches!(def.repo_type, RepositoryType::Package);

        if needs_url && def.url.is_none() {
            result.add(ValidationIssue::error(
                "repository.url.missing",
                field,
                "repository URL is required",
            ));
        }

        // Validate URL format
        if let Some(ref url) = def.url {
            self.validate_url(url, &format!("{field}.url"), result);
        }

        // Package type requires package definition
        if matches!(def.repo_type, RepositoryType::Package) && def.package.is_none() {
            result.add(ValidationIssue::error(
                "repository.package.missing",
                field,
                "package definition is required for type 'package'",
            ));
        }
    }

    /// Validate URL format.
    fn validate_url(&self, url: &str, field: &str, result: &mut ValidationResult) {
        // Allow special protocols
        if url.starts_with("git@") || url.starts_with("ssh://") {
            return;
        }

        match url::Url::parse(url) {
            Ok(parsed) => {
                // Warn about non-HTTPS
                if parsed.scheme() == "http" {
                    result.add(
                        ValidationIssue::warning("url.insecure", field, "HTTP URL is insecure")
                            .with_hint("use HTTPS for secure connections"),
                    );
                }
            }
            Err(_) => {
                // Could be a relative path for path repositories
                if !url.starts_with("./") && !url.starts_with("../") && !Path::new(url).exists() {
                    result.add(ValidationIssue::error(
                        "url.invalid",
                        field,
                        "invalid URL format",
                    ));
                }
            }
        }
    }

    /// Validate autoload configuration.
    fn validate_autoload(
        &self,
        autoload: &crate::types::AutoloadConfig,
        prefix: &str,
        result: &mut ValidationResult,
    ) {
        // Validate PSR-4 namespaces
        if let Some(ref psr4) = autoload.psr4 {
            for namespace in psr4.keys() {
                // PSR-4 namespaces should end with backslash
                if !namespace.is_empty() && !namespace.ends_with('\\') {
                    result.add(
                        ValidationIssue::warning(
                            "autoload.psr4.namespace",
                            format!("{prefix}.psr-4.{namespace}"),
                            "PSR-4 namespace should end with backslash",
                        )
                        .with_hint("add trailing backslash to namespace"),
                    );
                }

                // Check for invalid namespace characters
                if namespace.contains('/') {
                    result.add(ValidationIssue::error(
                        "autoload.psr4.namespace.invalid",
                        format!("{prefix}.psr-4.{namespace}"),
                        "namespace should use backslashes, not forward slashes",
                    ));
                }
            }
        }

        // Validate classmap paths exist (warning only)
        if let Some(ref classmap) = autoload.classmap {
            for path in classmap {
                if path.contains('*') {
                    continue; // Glob patterns are OK
                }
                // Note: We can't validate paths here without project context
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_package_name_valid() {
        let validator = Validator::new();
        let mut result = ValidationResult::new();
        validator.validate_package_name("vendor/package", "test", &mut result);
        assert!(!result.has_errors());
    }

    #[test]
    fn validate_package_name_invalid() {
        let validator = Validator::new();
        let mut result = ValidationResult::new();
        validator.validate_package_name("invalid", "test", &mut result);
        assert!(result.has_errors());
    }

    #[test]
    fn validate_platform_package_names() {
        let validator = Validator::new();
        let mut result = ValidationResult::new();

        validator.validate_package_name("php-64bit", "test.php-64bit", &mut result);
        validator.validate_package_name(
            "composer-runtime-api",
            "test.composer-runtime-api",
            &mut result,
        );
        validator.validate_package_name("ext-json", "test.ext-json", &mut result);
        validator.validate_package_name("lib-icu-uc", "test.lib-icu-uc", &mut result);

        assert!(!result.has_errors());
    }

    #[test]
    fn validate_version_constraint() {
        let validator = Validator::new();
        let mut result = ValidationResult::new();

        validator.validate_version_constraint("^1.0", "test", &mut result);
        assert!(!result.has_errors());

        validator.validate_version_constraint("", "test", &mut result);
        assert!(result.has_errors());
    }

    #[test]
    fn validation_result_counts() {
        let mut result = ValidationResult::new();
        result.add(ValidationIssue::error("e1", "f1", "error"));
        result.add(ValidationIssue::warning("w1", "f2", "warning"));
        result.add(ValidationIssue::error("e2", "f3", "error"));

        assert_eq!(result.error_count(), 2);
        assert_eq!(result.warning_count(), 1);
        assert!(result.has_errors());
        assert!(result.has_warnings());
    }
}
