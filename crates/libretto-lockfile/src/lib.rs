//! Ultra-high-performance composer.lock file management.
//!
//! This crate provides Composer-compatible lock file handling with:
//!
//! - **Atomic operations**: Crash-safe writes using temp file + rename
//! - **SIMD acceleration**: Fast hashing and comparison
//! - **Deterministic output**: Same input always produces identical output
//! - **Full validation**: Structural checks, drift detection, manual edit detection
//! - **Migration support**: Handles old lock file versions
//!
//! # Performance
//!
//! Designed for <10ms operations on 500 packages:
//! - SIMD-accelerated BLAKE3 hashing
//! - Parallel package sorting and diff computation
//! - Memory-efficient streaming serialization
//! - File locking with fs2 for concurrent access
//!
//! # Example
//!
//! ```no_run
//! use libretto_lockfile::{LockfileManager, LockGenerator, LockedPackage};
//! use std::collections::BTreeMap;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Create a lock file manager
//! let manager = LockfileManager::new("composer.lock")?;
//!
//! // Generate a new lock file
//! let mut generator = LockGenerator::new();
//! generator
//!     .minimum_stability("stable")
//!     .prefer_stable(true)
//!     .add_package(LockedPackage::new("vendor/package", "1.0.0"));
//!
//! let require = BTreeMap::new();
//! let lock = generator.generate(&require, &BTreeMap::new());
//!
//! // Write atomically
//! manager.write(&lock)?;
//!
//! // Read back
//! let loaded = manager.read()?;
//! # Ok(())
//! # }
//! ```

#![deny(clippy::all)]
#![allow(clippy::module_name_repetitions)]

pub mod atomic;
pub mod diff;
pub mod error;
pub mod generator;
pub mod hash;
pub mod migration;
pub mod types;
pub mod validation;

pub use atomic::{AtomicReader, AtomicWriter, Transaction, WriteResult};
pub use diff::{compute_diff, ChangeType, FieldChange, LockDiff, PackageChange};
pub use error::{LockfileError, Result};
pub use generator::{DeterministicSerializer, LockGenerator};
pub use hash::{bytes_to_hex, hex_to_bytes, ContentHasher, IntegrityHasher, ParallelHasher};
pub use migration::{MigrationResult, Migrator, SchemaVersion};
pub use types::{
    AutoloadConfig, AutoloadPath, ComposerLock, LockedPackage, PackageAlias, PackageAuthor,
    PackageDistInfo, PackageSourceInfo, StabilityFlag,
};
pub use validation::{DriftDetector, DriftResult, ManualEditDetector, ValidationResult, Validator};

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tracing::{debug, info};

/// High-level lock file manager.
///
/// Provides a convenient API for all lock file operations:
/// - Reading and writing with atomic guarantees
/// - Validation and drift detection
/// - Diff generation
/// - Migration handling
#[derive(Debug)]
pub struct LockfileManager {
    /// Path to the lock file.
    path: PathBuf,
    /// Validation settings.
    validator: Validator,
    /// Whether to auto-migrate old versions.
    auto_migrate: bool,
    /// Whether to create backups on write.
    create_backup: bool,
}

impl LockfileManager {
    /// Create a new lock file manager.
    ///
    /// # Arguments
    /// * `path` - Path to the composer.lock file
    ///
    /// # Errors
    /// Returns error if path is invalid.
    pub fn new(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        Ok(Self {
            path,
            validator: Validator::new(),
            auto_migrate: true,
            create_backup: true,
        })
    }

    /// Set custom validator.
    pub fn with_validator(mut self, validator: Validator) -> Self {
        self.validator = validator;
        self
    }

    /// Disable auto-migration.
    pub fn no_auto_migrate(mut self) -> Self {
        self.auto_migrate = false;
        self
    }

    /// Disable backup creation.
    pub fn no_backup(mut self) -> Self {
        self.create_backup = false;
        self
    }

    /// Get the lock file path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Check if lock file exists.
    #[must_use]
    pub fn exists(&self) -> bool {
        self.path.exists()
    }

    /// Read the lock file.
    ///
    /// Automatically migrates old versions if enabled.
    ///
    /// # Errors
    /// Returns error if file cannot be read or parsed.
    pub fn read(&self) -> Result<ComposerLock> {
        debug!(path = %self.path.display(), "Reading lock file");

        let reader = AtomicReader::new(&self.path)?;
        if !reader.exists() {
            return Err(LockfileError::NotFound(self.path.clone()));
        }

        let content = reader.read_string()?;
        let lock: ComposerLock = sonic_rs::from_str(&content)?;

        // Auto-migrate if needed
        if self.auto_migrate && migration::needs_migration(&content) {
            info!("Lock file needs migration, auto-migrating");
            let result = Migrator::new().migrate(lock)?;
            if result.has_changes() {
                for change in &result.changes {
                    debug!("Migration: {}", change);
                }
            }
            return Ok(result.lock);
        }

        Ok(lock)
    }

    /// Read and validate the lock file.
    ///
    /// # Errors
    /// Returns error if file cannot be read, parsed, or validation fails.
    pub fn read_validated(&self) -> Result<(ComposerLock, ValidationResult)> {
        let lock = self.read()?;
        let validation = self.validator.validate(&lock);
        Ok((lock, validation))
    }

    /// Write the lock file atomically.
    ///
    /// # Arguments
    /// * `lock` - The lock file to write
    ///
    /// # Errors
    /// Returns error if write fails.
    pub fn write(&self, lock: &ComposerLock) -> Result<WriteResult> {
        debug!(path = %self.path.display(), "Writing lock file");

        // Serialize deterministically
        let content = DeterministicSerializer::serialize(lock)?;

        // Write atomically
        let mut writer = AtomicWriter::new(&self.path)?;
        writer.content(content.as_bytes());
        if !self.create_backup {
            writer.no_backup();
        }

        let result = writer.commit()?;

        info!(
            path = %self.path.display(),
            bytes = result.bytes_written,
            "Lock file written"
        );

        Ok(result)
    }

    /// Update the lock file atomically.
    ///
    /// Reads the current lock, applies the update function, and writes back.
    ///
    /// # Arguments
    /// * `update_fn` - Function to modify the lock file
    ///
    /// # Errors
    /// Returns error if read, update, or write fails.
    pub fn update<F>(&self, update_fn: F) -> Result<WriteResult>
    where
        F: FnOnce(&mut ComposerLock),
    {
        let mut lock = self.read()?;
        update_fn(&mut lock);
        self.write(&lock)
    }

    /// Validate the lock file against composer.json.
    ///
    /// # Arguments
    /// * `require` - Production dependencies
    /// * `require_dev` - Development dependencies
    /// * `minimum_stability` - Minimum stability setting
    /// * `prefer_stable` - Prefer stable flag
    /// * `platform` - Platform requirements
    ///
    /// # Errors
    /// Returns error if lock file cannot be read.
    pub fn validate_against(
        &self,
        require: &BTreeMap<String, String>,
        require_dev: &BTreeMap<String, String>,
        minimum_stability: Option<&str>,
        prefer_stable: Option<bool>,
        platform: &BTreeMap<String, String>,
    ) -> Result<ValidationResult> {
        let lock = self.read()?;
        Ok(self.validator.validate_against_manifest(
            &lock,
            require,
            require_dev,
            minimum_stability,
            prefer_stable,
            platform,
        ))
    }

    /// Check for drift between lock file and composer.json.
    ///
    /// # Errors
    /// Returns error if lock file cannot be read.
    pub fn check_drift(
        &self,
        require: &BTreeMap<String, String>,
        require_dev: &BTreeMap<String, String>,
        minimum_stability: Option<&str>,
        prefer_stable: Option<bool>,
        platform: &BTreeMap<String, String>,
    ) -> Result<DriftResult> {
        let lock = self.read()?;
        Ok(DriftDetector::check_drift(
            &lock,
            require,
            require_dev,
            minimum_stability,
            prefer_stable,
            platform,
        ))
    }

    /// Compute diff between current and another lock file.
    ///
    /// # Errors
    /// Returns error if lock files cannot be read.
    pub fn diff(&self, other: &ComposerLock) -> Result<LockDiff> {
        let current = self.read()?;
        Ok(compute_diff(&current, other))
    }

    /// Compute diff between current lock file and a path.
    ///
    /// # Errors
    /// Returns error if lock files cannot be read.
    pub fn diff_with(&self, other_path: impl AsRef<Path>) -> Result<LockDiff> {
        let current = self.read()?;
        let other_manager = LockfileManager::new(other_path)?;
        let other = other_manager.read()?;
        Ok(compute_diff(&current, &other))
    }

    /// Detect manual edits in the lock file.
    ///
    /// # Errors
    /// Returns error if lock file cannot be read.
    pub fn detect_manual_edits(&self) -> Result<Vec<String>> {
        let lock = self.read()?;
        Ok(ManualEditDetector::detect(&lock))
    }

    /// Get the content hash from the lock file.
    ///
    /// # Errors
    /// Returns error if lock file cannot be read.
    pub fn content_hash(&self) -> Result<String> {
        let lock = self.read()?;
        Ok(lock.content_hash)
    }

    /// Compute what the content hash should be.
    #[must_use]
    pub fn compute_content_hash(
        require: &BTreeMap<String, String>,
        require_dev: &BTreeMap<String, String>,
        minimum_stability: Option<&str>,
        prefer_stable: Option<bool>,
        prefer_lowest: Option<bool>,
        platform: &BTreeMap<String, String>,
    ) -> String {
        ContentHasher::compute_content_hash(
            require,
            require_dev,
            minimum_stability,
            prefer_stable,
            prefer_lowest,
            platform,
            &BTreeMap::new(),
        )
    }

    /// Get all locked package names.
    ///
    /// # Errors
    /// Returns error if lock file cannot be read.
    pub fn package_names(&self) -> Result<Vec<String>> {
        let lock = self.read()?;
        let mut names: Vec<String> = lock
            .packages
            .iter()
            .chain(lock.packages_dev.iter())
            .map(|p| p.name.clone())
            .collect();
        names.sort();
        Ok(names)
    }

    /// Get a specific package from the lock file.
    ///
    /// # Errors
    /// Returns error if lock file cannot be read or package not found.
    pub fn get_package(&self, name: &str) -> Result<LockedPackage> {
        let lock = self.read()?;
        lock.packages
            .iter()
            .chain(lock.packages_dev.iter())
            .find(|p| p.name.eq_ignore_ascii_case(name))
            .cloned()
            .ok_or_else(|| LockfileError::PackageNotFound {
                name: name.to_string(),
            })
    }

    /// Check if a package is locked.
    ///
    /// # Errors
    /// Returns error if lock file cannot be read.
    pub fn has_package(&self, name: &str) -> Result<bool> {
        let lock = self.read()?;
        Ok(lock
            .packages
            .iter()
            .chain(lock.packages_dev.iter())
            .any(|p| p.name.eq_ignore_ascii_case(name)))
    }

    /// Get lock file statistics.
    ///
    /// # Errors
    /// Returns error if lock file cannot be read.
    pub fn stats(&self) -> Result<LockStats> {
        let lock = self.read()?;
        Ok(LockStats {
            packages: lock.packages.len(),
            packages_dev: lock.packages_dev.len(),
            aliases: lock.aliases.len(),
            minimum_stability: lock.minimum_stability.clone(),
            prefer_stable: lock.prefer_stable,
            prefer_lowest: lock.prefer_lowest,
            platform_count: lock.platform.len(),
            content_hash: lock.content_hash.clone(),
        })
    }

    /// Recover from crashes (clean up temp files).
    ///
    /// # Errors
    /// Returns error if recovery fails.
    pub fn recover(&self) -> Result<atomic::RecoveryResult> {
        if let Some(dir) = self.path.parent() {
            atomic::recover(dir)
        } else {
            Ok(atomic::RecoveryResult::default())
        }
    }
}

/// Lock file statistics.
#[derive(Debug, Clone)]
pub struct LockStats {
    /// Number of production packages.
    pub packages: usize,
    /// Number of dev packages.
    pub packages_dev: usize,
    /// Number of aliases.
    pub aliases: usize,
    /// Minimum stability.
    pub minimum_stability: String,
    /// Prefer stable flag.
    pub prefer_stable: bool,
    /// Prefer lowest flag.
    pub prefer_lowest: bool,
    /// Number of platform requirements.
    pub platform_count: usize,
    /// Content hash.
    pub content_hash: String,
}

impl LockStats {
    /// Total package count.
    #[must_use]
    pub fn total_packages(&self) -> usize {
        self.packages + self.packages_dev
    }
}

impl std::fmt::Display for LockStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Lock file statistics:")?;
        writeln!(f, "  Production packages: {}", self.packages)?;
        writeln!(f, "  Dev packages: {}", self.packages_dev)?;
        writeln!(f, "  Total: {}", self.total_packages())?;
        writeln!(f, "  Aliases: {}", self.aliases)?;
        writeln!(f, "  Minimum stability: {}", self.minimum_stability)?;
        writeln!(f, "  Prefer stable: {}", self.prefer_stable)?;
        writeln!(
            f,
            "  Content hash: {}",
            &self.content_hash[..8.min(self.content_hash.len())]
        )?;
        Ok(())
    }
}

/// Partial update options.
#[derive(Debug, Clone, Default)]
pub struct PartialUpdateOptions {
    /// Only update specified packages.
    pub packages: Vec<String>,
    /// Include dev dependencies.
    pub with_dev: bool,
    /// Lock flag (update lock without installing).
    pub lock_only: bool,
}

impl PartialUpdateOptions {
    /// Create new partial update options.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a package to update.
    pub fn package(&mut self, name: impl Into<String>) -> &mut Self {
        self.packages.push(name.into());
        self
    }

    /// Include dev dependencies.
    pub fn with_dev(&mut self, include: bool) -> &mut Self {
        self.with_dev = include;
        self
    }

    /// Set lock-only mode.
    pub fn lock_only(&mut self, enabled: bool) -> &mut Self {
        self.lock_only = enabled;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_manager_new() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("composer.lock");
        let manager = LockfileManager::new(&path).unwrap();
        assert_eq!(manager.path(), path);
        assert!(!manager.exists());
    }

    #[test]
    fn test_manager_write_read() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("composer.lock");
        let manager = LockfileManager::new(&path).unwrap();

        // Create and write a lock file
        let mut generator = LockGenerator::new();
        generator.add_package(LockedPackage::new("vendor/pkg", "1.0.0"));
        let lock = generator.generate(&BTreeMap::new(), &BTreeMap::new());

        manager.write(&lock).unwrap();
        assert!(manager.exists());

        // Read it back
        let loaded = manager.read().unwrap();
        assert_eq!(loaded.packages.len(), 1);
        assert_eq!(loaded.packages[0].name, "vendor/pkg");
    }

    #[test]
    fn test_manager_update() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("composer.lock");
        let manager = LockfileManager::new(&path).unwrap();

        // Write initial lock
        let lock = LockGenerator::new().generate(&BTreeMap::new(), &BTreeMap::new());
        manager.write(&lock).unwrap();

        // Update it
        manager
            .update(|lock| {
                lock.packages.push(LockedPackage::new("new/pkg", "2.0.0"));
            })
            .unwrap();

        // Verify
        let loaded = manager.read().unwrap();
        assert_eq!(loaded.packages.len(), 1);
        assert_eq!(loaded.packages[0].name, "new/pkg");
    }

    #[test]
    fn test_manager_stats() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("composer.lock");
        let manager = LockfileManager::new(&path).unwrap();

        let mut generator = LockGenerator::new();
        generator
            .minimum_stability("dev")
            .prefer_stable(true)
            .add_package(LockedPackage::new("a/b", "1.0.0"))
            .add_package(LockedPackage::new("c/d", "2.0.0"))
            .add_package_dev(LockedPackage::new("dev/pkg", "3.0.0"));

        let lock = generator.generate(&BTreeMap::new(), &BTreeMap::new());
        manager.write(&lock).unwrap();

        let stats = manager.stats().unwrap();
        assert_eq!(stats.packages, 2);
        assert_eq!(stats.packages_dev, 1);
        assert_eq!(stats.total_packages(), 3);
        assert_eq!(stats.minimum_stability, "dev");
        assert!(stats.prefer_stable);
    }

    #[test]
    fn test_manager_has_package() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("composer.lock");
        let manager = LockfileManager::new(&path).unwrap();

        let mut generator = LockGenerator::new();
        generator.add_package(LockedPackage::new("vendor/pkg", "1.0.0"));
        let lock = generator.generate(&BTreeMap::new(), &BTreeMap::new());
        manager.write(&lock).unwrap();

        assert!(manager.has_package("vendor/pkg").unwrap());
        assert!(manager.has_package("VENDOR/PKG").unwrap()); // Case insensitive
        assert!(!manager.has_package("other/pkg").unwrap());
    }

    #[test]
    fn test_content_hash_computation() {
        let mut require = BTreeMap::new();
        require.insert("psr/log".to_string(), "^3.0".to_string());

        let hash1 = LockfileManager::compute_content_hash(
            &require,
            &BTreeMap::new(),
            Some("stable"),
            Some(true),
            None,
            &BTreeMap::new(),
        );

        let hash2 = LockfileManager::compute_content_hash(
            &require,
            &BTreeMap::new(),
            Some("stable"),
            Some(true),
            None,
            &BTreeMap::new(),
        );

        // Should be deterministic
        assert_eq!(hash1, hash2);

        // Different input should produce different hash
        let hash3 = LockfileManager::compute_content_hash(
            &require,
            &BTreeMap::new(),
            Some("dev"),
            Some(true),
            None,
            &BTreeMap::new(),
        );

        assert_ne!(hash1, hash3);
    }
}
