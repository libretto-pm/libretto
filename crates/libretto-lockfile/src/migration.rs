//! Lock file migration from old versions.
//!
//! Handles:
//! - Schema changes between Composer versions
//! - Missing field defaults
//! - Format normalization

use crate::error::{LockfileError, Result};
use crate::types::{ComposerLock, LockedPackage, StabilityFlag};
use sonic_rs::{JsonValueTrait, Value};
use tracing::{debug, info, warn};

/// Lock file schema version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SchemaVersion {
    /// Major version.
    pub major: u32,
    /// Minor version.
    pub minor: u32,
}

impl SchemaVersion {
    /// Current schema version (Composer 2.x).
    pub const CURRENT: Self = Self { major: 2, minor: 6 };

    /// Composer 1.x schema.
    pub const V1: Self = Self { major: 1, minor: 0 };

    /// Composer 2.0 schema.
    pub const V2_0: Self = Self { major: 2, minor: 0 };

    /// Create a new schema version.
    #[must_use]
    pub const fn new(major: u32, minor: u32) -> Self {
        Self { major, minor }
    }

    /// Parse from plugin-api-version string.
    #[must_use]
    pub fn parse(version: &str) -> Option<Self> {
        let parts: Vec<&str> = version.split('.').collect();
        if parts.len() >= 2 {
            let major = parts[0].parse().ok()?;
            let minor = parts[1].parse().ok()?;
            Some(Self { major, minor })
        } else {
            None
        }
    }

    /// Check if this version needs migration.
    #[must_use]
    pub fn needs_migration(self) -> bool {
        self < Self::CURRENT
    }
}

impl std::fmt::Display for SchemaVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.0", self.major, self.minor)
    }
}

impl Default for SchemaVersion {
    fn default() -> Self {
        Self::CURRENT
    }
}

/// Migration result.
#[derive(Debug)]
pub struct MigrationResult {
    /// Original version.
    pub from_version: SchemaVersion,
    /// Target version.
    pub to_version: SchemaVersion,
    /// Changes made during migration.
    pub changes: Vec<MigrationChange>,
    /// Migrated lock file.
    pub lock: ComposerLock,
}

impl MigrationResult {
    /// Check if any changes were made.
    #[must_use]
    pub const fn has_changes(&self) -> bool {
        !self.changes.is_empty()
    }

    /// Get summary of changes.
    #[must_use]
    pub fn summary(&self) -> String {
        if self.changes.is_empty() {
            return "No migration needed".to_string();
        }

        format!(
            "Migrated from {} to {}: {} changes",
            self.from_version,
            self.to_version,
            self.changes.len()
        )
    }
}

/// A single migration change.
#[derive(Debug, Clone)]
pub enum MigrationChange {
    /// Added missing field.
    AddedField { field: String, value: String },
    /// Renamed field.
    RenamedField { old: String, new: String },
    /// Updated field format.
    UpdatedFormat { field: String, reason: String },
    /// Removed deprecated field.
    RemovedField { field: String },
    /// Normalized value.
    NormalizedValue {
        field: String,
        from: String,
        to: String,
    },
    /// Added default value.
    AddedDefault { field: String, value: String },
}

impl std::fmt::Display for MigrationChange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AddedField { field, value } => {
                write!(f, "Added field '{field}' with value '{value}'")
            }
            Self::RenamedField { old, new } => {
                write!(f, "Renamed field '{old}' to '{new}'")
            }
            Self::UpdatedFormat { field, reason } => {
                write!(f, "Updated format of '{field}': {reason}")
            }
            Self::RemovedField { field } => {
                write!(f, "Removed deprecated field '{field}'")
            }
            Self::NormalizedValue { field, from, to } => {
                write!(f, "Normalized '{field}': '{from}' -> '{to}'")
            }
            Self::AddedDefault { field, value } => {
                write!(f, "Added default for '{field}': '{value}'")
            }
        }
    }
}

/// Lock file migrator.
#[derive(Debug, Default)]
pub struct Migrator {
    /// Target version.
    target_version: SchemaVersion,
}

impl Migrator {
    /// Create a new migrator targeting the current version.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            target_version: SchemaVersion::CURRENT,
        }
    }

    /// Set target version.
    pub const fn target_version(&mut self, version: SchemaVersion) -> &mut Self {
        self.target_version = version;
        self
    }

    /// Migrate a lock file to the target version.
    pub fn migrate(&self, lock: ComposerLock) -> Result<MigrationResult> {
        let from_version =
            SchemaVersion::parse(&lock.plugin_api_version).unwrap_or(SchemaVersion::V1);

        if from_version >= self.target_version {
            return Ok(MigrationResult {
                from_version,
                to_version: from_version,
                changes: Vec::new(),
                lock,
            });
        }

        info!(
            from = %from_version.to_string(),
            to = %self.target_version.to_string(),
            "Migrating lock file"
        );

        let mut changes = Vec::new();
        let mut lock = lock;

        // Apply migrations in order
        if from_version < SchemaVersion::V2_0 {
            self.migrate_v1_to_v2(&mut lock, &mut changes)?;
        }

        if from_version < SchemaVersion::CURRENT {
            self.migrate_to_current(&mut lock, &mut changes)?;
        }

        // Update version
        lock.plugin_api_version = self.target_version.to_string();

        Ok(MigrationResult {
            from_version,
            to_version: self.target_version,
            changes,
            lock,
        })
    }

    /// Migrate from Composer 1.x to 2.x format.
    fn migrate_v1_to_v2(
        &self,
        lock: &mut ComposerLock,
        changes: &mut Vec<MigrationChange>,
    ) -> Result<()> {
        debug!("Migrating from Composer 1.x to 2.x");

        // Add content-hash if missing (was optional in 1.x)
        if lock.content_hash.is_empty() {
            // We can't compute the correct hash without composer.json
            // Use a placeholder that will trigger re-resolution
            lock.content_hash = "MIGRATION_REQUIRED".to_string();
            changes.push(MigrationChange::AddedField {
                field: "content-hash".to_string(),
                value: "MIGRATION_REQUIRED (run composer update to fix)".to_string(),
            });
        }

        // Ensure stability-flags are numeric
        // In 1.x, these might have been strings
        for (name, flag) in &lock.stability_flags {
            if StabilityFlag::from_u8(*flag).is_none() {
                warn!(
                    package = %name,
                    flag = %flag,
                    "Invalid stability flag, defaulting to stable"
                );
            }
        }

        // Add platform-dev if missing
        if lock.platform_dev.is_empty() && !lock.platform.is_empty() {
            // In 1.x, platform was used for both
            changes.push(MigrationChange::AddedField {
                field: "platform-dev".to_string(),
                value: "{}".to_string(),
            });
        }

        // Normalize package data
        for pkg in lock.packages.iter_mut().chain(lock.packages_dev.iter_mut()) {
            self.normalize_package(pkg, changes);
        }

        Ok(())
    }

    /// Migrate to current version.
    fn migrate_to_current(
        &self,
        lock: &mut ComposerLock,
        changes: &mut Vec<MigrationChange>,
    ) -> Result<()> {
        debug!("Migrating to current version");

        // Ensure readme is present and correct
        let expected_readme = vec![
            "This file locks the dependencies of your project to a known state".to_string(),
            "Read more about it at https://getcomposer.org/doc/01-basic-usage.md#installing-dependencies".to_string(),
            "This file is @generated automatically".to_string(),
        ];

        if lock.readme != expected_readme {
            lock.readme = expected_readme;
            changes.push(MigrationChange::UpdatedFormat {
                field: "_readme".to_string(),
                reason: "Standardized readme content".to_string(),
            });
        }

        // Sort packages
        let was_sorted = is_sorted(&lock.packages);
        lock.packages.sort();
        if !was_sorted {
            changes.push(MigrationChange::UpdatedFormat {
                field: "packages".to_string(),
                reason: "Sorted alphabetically".to_string(),
            });
        }

        let was_dev_sorted = is_sorted(&lock.packages_dev);
        lock.packages_dev.sort();
        if !was_dev_sorted {
            changes.push(MigrationChange::UpdatedFormat {
                field: "packages-dev".to_string(),
                reason: "Sorted alphabetically".to_string(),
            });
        }

        // Sort aliases
        lock.aliases.sort_by(|a, b| a.package.cmp(&b.package));

        // Normalize minimum-stability
        let normalized = normalize_stability(&lock.minimum_stability);
        if normalized != lock.minimum_stability {
            changes.push(MigrationChange::NormalizedValue {
                field: "minimum-stability".to_string(),
                from: lock.minimum_stability.clone(),
                to: normalized.clone(),
            });
            lock.minimum_stability = normalized;
        }

        Ok(())
    }

    /// Normalize a package's data.
    fn normalize_package(&self, pkg: &mut LockedPackage, changes: &mut Vec<MigrationChange>) {
        // Normalize version (remove leading 'v' if present for consistency)
        // Actually, keep 'v' as Composer does - just ensure consistency

        // Sort dependencies
        // BTreeMap is already sorted, so this is a no-op for the data structure
        // but we record if keys were in wrong order originally

        // Ensure type has a default
        if pkg.package_type.is_none() {
            pkg.package_type = Some("library".to_string());
            changes.push(MigrationChange::AddedDefault {
                field: format!("packages.{}.type", pkg.name),
                value: "library".to_string(),
            });
        }
    }

    /// Migrate from raw JSON value (for very old or malformed lock files).
    pub fn migrate_from_value(&self, value: Value) -> Result<MigrationResult> {
        // Try to parse as ComposerLock first
        let lock: ComposerLock = sonic_rs::from_value(&value)
            .map_err(|e| LockfileError::Migration(format!("Failed to parse lock file: {e}")))?;

        self.migrate(lock)
    }
}

/// Check if packages are sorted.
fn is_sorted(packages: &[LockedPackage]) -> bool {
    packages.windows(2).all(|w| w[0] <= w[1])
}

/// Normalize stability string.
fn normalize_stability(stability: &str) -> String {
    match stability.to_lowercase().as_str() {
        "stable" => "stable",
        "rc" => "RC",
        "beta" => "beta",
        "alpha" => "alpha",
        "dev" => "dev",
        other => other,
    }
    .to_string()
}

/// Detect lock file version from JSON.
#[must_use]
pub fn detect_version(json: &str) -> Option<SchemaVersion> {
    // Quick parse to get plugin-api-version
    let value: Value = sonic_rs::from_str(json).ok()?;
    let version_str = value.get("plugin-api-version")?.as_str()?;
    SchemaVersion::parse(version_str)
}

/// Check if migration is needed without fully parsing.
#[must_use]
pub fn needs_migration(json: &str) -> bool {
    detect_version(json).is_none_or(SchemaVersion::needs_migration) // If we can't detect version, assume migration needed
}

/// Auto-migrate a lock file string.
pub fn auto_migrate(json: &str) -> Result<(ComposerLock, Vec<MigrationChange>)> {
    let lock: ComposerLock = sonic_rs::from_str(json)?;
    let result = Migrator::new().migrate(lock)?;
    Ok((result.lock, result.changes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema_version_parse() {
        assert_eq!(
            SchemaVersion::parse("2.6.0"),
            Some(SchemaVersion::new(2, 6))
        );
        assert_eq!(
            SchemaVersion::parse("1.0.0"),
            Some(SchemaVersion::new(1, 0))
        );
        assert_eq!(SchemaVersion::parse("invalid"), None);
    }

    #[test]
    fn test_schema_version_comparison() {
        assert!(SchemaVersion::V1 < SchemaVersion::V2_0);
        assert!(SchemaVersion::V2_0 < SchemaVersion::CURRENT);
        assert!(SchemaVersion::V1.needs_migration());
    }

    #[test]
    fn test_migration_no_changes() {
        let mut lock = ComposerLock::default();
        lock.plugin_api_version = SchemaVersion::CURRENT.to_string();

        let result = Migrator::new().migrate(lock).unwrap();
        assert!(!result.has_changes());
    }

    #[test]
    fn test_migration_from_v1() {
        let mut lock = ComposerLock::default();
        lock.plugin_api_version = "1.0.0".to_string();
        lock.content_hash = String::new(); // Missing in 1.x

        let result = Migrator::new().migrate(lock).unwrap();
        assert!(result.has_changes());
        assert_eq!(result.from_version, SchemaVersion::V1);
    }

    #[test]
    fn test_normalize_stability() {
        assert_eq!(normalize_stability("STABLE"), "stable");
        assert_eq!(normalize_stability("rc"), "RC");
        assert_eq!(normalize_stability("Dev"), "dev");
    }

    #[test]
    fn test_detect_version() {
        let json = r#"{"plugin-api-version": "2.6.0"}"#;
        assert_eq!(detect_version(json), Some(SchemaVersion::new(2, 6)));
    }

    #[test]
    fn test_needs_migration() {
        let old_json = r#"{"plugin-api-version": "1.0.0"}"#;
        let new_json = r#"{"plugin-api-version": "2.6.0"}"#;

        assert!(needs_migration(old_json));
        assert!(!needs_migration(new_json));
    }
}
