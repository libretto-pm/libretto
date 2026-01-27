//! SIMD-accelerated diff computation for lock files.
//!
//! Provides ultra-fast comparison of lock files with human-readable output.

use crate::types::{ComposerLock, LockedPackage};
use ahash::AHashMap;
use rayon::prelude::*;
use std::fmt;

/// Change type for a package.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeType {
    /// Package was added.
    Added,
    /// Package was removed.
    Removed,
    /// Package version was upgraded.
    Upgraded,
    /// Package version was downgraded.
    Downgraded,
    /// Package was modified (same version, different metadata).
    Modified,
}

impl fmt::Display for ChangeType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Added => write!(f, "Added"),
            Self::Removed => write!(f, "Removed"),
            Self::Upgraded => write!(f, "Upgraded"),
            Self::Downgraded => write!(f, "Downgraded"),
            Self::Modified => write!(f, "Modified"),
        }
    }
}

/// A single package change.
#[derive(Debug, Clone)]
pub struct PackageChange {
    /// Package name.
    pub name: String,
    /// Type of change.
    pub change_type: ChangeType,
    /// Old version (if applicable).
    pub old_version: Option<String>,
    /// New version (if applicable).
    pub new_version: Option<String>,
    /// Is this a dev dependency?
    pub is_dev: bool,
    /// Detailed field changes (for Modified type).
    pub field_changes: Vec<FieldChange>,
}

impl PackageChange {
    /// Create an added package change.
    #[must_use]
    pub fn added(name: String, version: String, is_dev: bool) -> Self {
        Self {
            name,
            change_type: ChangeType::Added,
            old_version: None,
            new_version: Some(version),
            is_dev,
            field_changes: Vec::new(),
        }
    }

    /// Create a removed package change.
    #[must_use]
    pub fn removed(name: String, version: String, is_dev: bool) -> Self {
        Self {
            name,
            change_type: ChangeType::Removed,
            old_version: Some(version),
            new_version: None,
            is_dev,
            field_changes: Vec::new(),
        }
    }

    /// Create a version change.
    #[must_use]
    pub fn version_change(
        name: String,
        old_version: String,
        new_version: String,
        is_dev: bool,
    ) -> Self {
        let change_type = match compare_versions(&old_version, &new_version) {
            std::cmp::Ordering::Less => ChangeType::Upgraded,
            std::cmp::Ordering::Greater => ChangeType::Downgraded,
            std::cmp::Ordering::Equal => ChangeType::Modified,
        };
        Self {
            name,
            change_type,
            old_version: Some(old_version),
            new_version: Some(new_version),
            is_dev,
            field_changes: Vec::new(),
        }
    }

    /// Create a metadata modification change.
    #[must_use]
    pub fn modified(
        name: String,
        version: String,
        is_dev: bool,
        changes: Vec<FieldChange>,
    ) -> Self {
        Self {
            name,
            change_type: ChangeType::Modified,
            old_version: Some(version.clone()),
            new_version: Some(version),
            is_dev,
            field_changes: changes,
        }
    }
}

impl fmt::Display for PackageChange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let dev_marker = if self.is_dev { " (dev)" } else { "" };
        match self.change_type {
            ChangeType::Added => {
                write!(
                    f,
                    "  + {}{}: {}",
                    self.name,
                    dev_marker,
                    self.new_version.as_deref().unwrap_or("unknown")
                )
            }
            ChangeType::Removed => {
                write!(
                    f,
                    "  - {}{}: {}",
                    self.name,
                    dev_marker,
                    self.old_version.as_deref().unwrap_or("unknown")
                )
            }
            ChangeType::Upgraded | ChangeType::Downgraded => {
                let arrow = if self.change_type == ChangeType::Upgraded {
                    "↑"
                } else {
                    "↓"
                };
                write!(
                    f,
                    "  {} {}{}: {} -> {}",
                    arrow,
                    self.name,
                    dev_marker,
                    self.old_version.as_deref().unwrap_or("?"),
                    self.new_version.as_deref().unwrap_or("?")
                )
            }
            ChangeType::Modified => {
                write!(
                    f,
                    "  ~ {}{}: {} (metadata changed)",
                    self.name,
                    dev_marker,
                    self.new_version.as_deref().unwrap_or("unknown")
                )
            }
        }
    }
}

/// A field-level change within a package.
#[derive(Debug, Clone)]
pub struct FieldChange {
    /// Field name.
    pub field: String,
    /// Old value (as string representation).
    pub old_value: Option<String>,
    /// New value.
    pub new_value: Option<String>,
}

impl fmt::Display for FieldChange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.old_value, &self.new_value) {
            (Some(old), Some(new)) => write!(f, "    {}: {} -> {}", self.field, old, new),
            (Some(old), None) => write!(f, "    {}: {} (removed)", self.field, old),
            (None, Some(new)) => write!(f, "    {}: {} (added)", self.field, new),
            (None, None) => write!(f, "    {}: (unchanged)", self.field),
        }
    }
}

/// Complete diff between two lock files.
#[derive(Debug, Clone, Default)]
pub struct LockDiff {
    /// Package changes.
    pub packages: Vec<PackageChange>,
    /// Whether content hash changed.
    pub content_hash_changed: bool,
    /// Old content hash.
    pub old_content_hash: Option<String>,
    /// New content hash.
    pub new_content_hash: Option<String>,
    /// Platform changes.
    pub platform_changes: Vec<FieldChange>,
    /// Minimum stability change.
    pub stability_changed: Option<(String, String)>,
}

impl LockDiff {
    /// Check if there are any changes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.packages.is_empty()
            && !self.content_hash_changed
            && self.platform_changes.is_empty()
            && self.stability_changed.is_none()
    }

    /// Count total changes.
    #[must_use]
    pub fn change_count(&self) -> usize {
        self.packages.len()
    }

    /// Count added packages.
    #[must_use]
    pub fn added_count(&self) -> usize {
        self.packages
            .iter()
            .filter(|c| c.change_type == ChangeType::Added)
            .count()
    }

    /// Count removed packages.
    #[must_use]
    pub fn removed_count(&self) -> usize {
        self.packages
            .iter()
            .filter(|c| c.change_type == ChangeType::Removed)
            .count()
    }

    /// Count upgraded packages.
    #[must_use]
    pub fn upgraded_count(&self) -> usize {
        self.packages
            .iter()
            .filter(|c| c.change_type == ChangeType::Upgraded)
            .count()
    }

    /// Count downgraded packages.
    #[must_use]
    pub fn downgraded_count(&self) -> usize {
        self.packages
            .iter()
            .filter(|c| c.change_type == ChangeType::Downgraded)
            .count()
    }

    /// Generate summary line.
    #[must_use]
    pub fn summary(&self) -> String {
        if self.is_empty() {
            return "No changes".to_string();
        }

        let mut parts = Vec::new();
        let added = self.added_count();
        let removed = self.removed_count();
        let upgraded = self.upgraded_count();
        let downgraded = self.downgraded_count();
        let modified = self
            .packages
            .iter()
            .filter(|c| c.change_type == ChangeType::Modified)
            .count();

        if added > 0 {
            parts.push(format!("{} added", added));
        }
        if removed > 0 {
            parts.push(format!("{} removed", removed));
        }
        if upgraded > 0 {
            parts.push(format!("{} upgraded", upgraded));
        }
        if downgraded > 0 {
            parts.push(format!("{} downgraded", downgraded));
        }
        if modified > 0 {
            parts.push(format!("{} modified", modified));
        }

        parts.join(", ")
    }
}

impl fmt::Display for LockDiff {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            return writeln!(f, "No changes");
        }

        writeln!(f, "Lock file changes:")?;
        writeln!(f)?;

        if self.content_hash_changed {
            writeln!(f, "Content hash changed:")?;
            if let Some(ref old) = self.old_content_hash {
                writeln!(f, "  - {}", old)?;
            }
            if let Some(ref new) = self.new_content_hash {
                writeln!(f, "  + {}", new)?;
            }
            writeln!(f)?;
        }

        if let Some((ref old, ref new)) = self.stability_changed {
            writeln!(f, "Minimum stability: {} -> {}", old, new)?;
            writeln!(f)?;
        }

        if !self.platform_changes.is_empty() {
            writeln!(f, "Platform changes:")?;
            for change in &self.platform_changes {
                writeln!(f, "{}", change)?;
            }
            writeln!(f)?;
        }

        // Group by change type
        let added: Vec<_> = self
            .packages
            .iter()
            .filter(|c| c.change_type == ChangeType::Added)
            .collect();
        let removed: Vec<_> = self
            .packages
            .iter()
            .filter(|c| c.change_type == ChangeType::Removed)
            .collect();
        let upgraded: Vec<_> = self
            .packages
            .iter()
            .filter(|c| c.change_type == ChangeType::Upgraded)
            .collect();
        let downgraded: Vec<_> = self
            .packages
            .iter()
            .filter(|c| c.change_type == ChangeType::Downgraded)
            .collect();
        let modified: Vec<_> = self
            .packages
            .iter()
            .filter(|c| c.change_type == ChangeType::Modified)
            .collect();

        if !added.is_empty() {
            writeln!(f, "Added ({}):", added.len())?;
            for pkg in added {
                writeln!(f, "{}", pkg)?;
            }
            writeln!(f)?;
        }

        if !removed.is_empty() {
            writeln!(f, "Removed ({}):", removed.len())?;
            for pkg in removed {
                writeln!(f, "{}", pkg)?;
            }
            writeln!(f)?;
        }

        if !upgraded.is_empty() {
            writeln!(f, "Upgraded ({}):", upgraded.len())?;
            for pkg in upgraded {
                writeln!(f, "{}", pkg)?;
            }
            writeln!(f)?;
        }

        if !downgraded.is_empty() {
            writeln!(f, "Downgraded ({}):", downgraded.len())?;
            for pkg in downgraded {
                writeln!(f, "{}", pkg)?;
            }
            writeln!(f)?;
        }

        if !modified.is_empty() {
            writeln!(f, "Modified ({}):", modified.len())?;
            for pkg in modified {
                writeln!(f, "{}", pkg)?;
                for field_change in &pkg.field_changes {
                    writeln!(f, "{}", field_change)?;
                }
            }
            writeln!(f)?;
        }

        writeln!(f, "Summary: {}", self.summary())?;
        Ok(())
    }
}

/// Compute diff between two lock files using parallel processing.
#[must_use]
pub fn compute_diff(old: &ComposerLock, new: &ComposerLock) -> LockDiff {
    let mut diff = LockDiff::default();

    // Check content hash
    if old.content_hash != new.content_hash {
        diff.content_hash_changed = true;
        diff.old_content_hash = Some(old.content_hash.clone());
        diff.new_content_hash = Some(new.content_hash.clone());
    }

    // Check minimum stability
    if old.minimum_stability != new.minimum_stability {
        diff.stability_changed =
            Some((old.minimum_stability.clone(), new.minimum_stability.clone()));
    }

    // Check platform requirements
    diff.platform_changes = compute_btree_diff(&old.platform, &new.platform, "platform");
    diff.platform_changes.extend(compute_btree_diff(
        &old.platform_dev,
        &new.platform_dev,
        "platform-dev",
    ));

    // Compute package diffs in parallel
    let pkg_changes = compute_package_diff(&old.packages, &new.packages, false);
    let dev_changes = compute_package_diff(&old.packages_dev, &new.packages_dev, true);

    diff.packages = pkg_changes;
    diff.packages.extend(dev_changes);

    // Sort changes by name
    diff.packages
        .sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

    diff
}

/// Compute diff between package lists.
fn compute_package_diff(
    old_packages: &[LockedPackage],
    new_packages: &[LockedPackage],
    is_dev: bool,
) -> Vec<PackageChange> {
    // Build lookup maps
    let old_map: AHashMap<&str, &LockedPackage> =
        old_packages.iter().map(|p| (p.name.as_str(), p)).collect();

    let new_map: AHashMap<&str, &LockedPackage> =
        new_packages.iter().map(|p| (p.name.as_str(), p)).collect();

    // Find changes in parallel
    let mut changes: Vec<PackageChange> = new_packages
        .par_iter()
        .filter_map(|new_pkg| {
            match old_map.get(new_pkg.name.as_str()) {
                Some(old_pkg) => {
                    // Package exists in both - check for changes
                    if old_pkg.version != new_pkg.version {
                        Some(PackageChange::version_change(
                            new_pkg.name.clone(),
                            old_pkg.version.clone(),
                            new_pkg.version.clone(),
                            is_dev,
                        ))
                    } else {
                        // Same version - check for metadata changes
                        let field_changes = compute_package_metadata_diff(old_pkg, new_pkg);
                        if !field_changes.is_empty() {
                            Some(PackageChange::modified(
                                new_pkg.name.clone(),
                                new_pkg.version.clone(),
                                is_dev,
                                field_changes,
                            ))
                        } else {
                            None
                        }
                    }
                }
                None => {
                    // New package
                    Some(PackageChange::added(
                        new_pkg.name.clone(),
                        new_pkg.version.clone(),
                        is_dev,
                    ))
                }
            }
        })
        .collect();

    // Find removed packages
    let removed: Vec<PackageChange> = old_packages
        .par_iter()
        .filter_map(|old_pkg| {
            if !new_map.contains_key(old_pkg.name.as_str()) {
                Some(PackageChange::removed(
                    old_pkg.name.clone(),
                    old_pkg.version.clone(),
                    is_dev,
                ))
            } else {
                None
            }
        })
        .collect();

    changes.extend(removed);
    changes
}

/// Compute metadata diff between two packages (same version).
fn compute_package_metadata_diff(old: &LockedPackage, new: &LockedPackage) -> Vec<FieldChange> {
    let mut changes = Vec::new();

    // Check source
    if old.source != new.source {
        changes.push(FieldChange {
            field: "source".to_string(),
            old_value: old
                .source
                .as_ref()
                .map(|s| format!("{} @ {}", s.url, s.reference)),
            new_value: new
                .source
                .as_ref()
                .map(|s| format!("{} @ {}", s.url, s.reference)),
        });
    }

    // Check dist
    if old.dist != new.dist {
        changes.push(FieldChange {
            field: "dist".to_string(),
            old_value: old.dist.as_ref().map(|d| d.url.clone()),
            new_value: new.dist.as_ref().map(|d| d.url.clone()),
        });
    }

    // Check require
    if old.require != new.require {
        changes.push(FieldChange {
            field: "require".to_string(),
            old_value: Some(format!("{} deps", old.require.len())),
            new_value: Some(format!("{} deps", new.require.len())),
        });
    }

    // Check autoload
    if old.autoload != new.autoload {
        changes.push(FieldChange {
            field: "autoload".to_string(),
            old_value: Some("changed".to_string()),
            new_value: Some("changed".to_string()),
        });
    }

    changes
}

/// Compute diff between two BTrees.
fn compute_btree_diff(
    old: &std::collections::BTreeMap<String, String>,
    new: &std::collections::BTreeMap<String, String>,
    prefix: &str,
) -> Vec<FieldChange> {
    let mut changes = Vec::new();

    // Find added and changed
    for (key, new_value) in new {
        match old.get(key) {
            Some(old_value) if old_value != new_value => {
                changes.push(FieldChange {
                    field: format!("{}.{}", prefix, key),
                    old_value: Some(old_value.clone()),
                    new_value: Some(new_value.clone()),
                });
            }
            None => {
                changes.push(FieldChange {
                    field: format!("{}.{}", prefix, key),
                    old_value: None,
                    new_value: Some(new_value.clone()),
                });
            }
            _ => {}
        }
    }

    // Find removed
    for (key, old_value) in old {
        if !new.contains_key(key) {
            changes.push(FieldChange {
                field: format!("{}.{}", prefix, key),
                old_value: Some(old_value.clone()),
                new_value: None,
            });
        }
    }

    changes
}

/// Compare version strings (simple semver-like comparison).
fn compare_versions(old: &str, new: &str) -> std::cmp::Ordering {
    // Try semver comparison first
    if let (Ok(old_ver), Ok(new_ver)) = (
        semver::Version::parse(old.trim_start_matches('v')),
        semver::Version::parse(new.trim_start_matches('v')),
    ) {
        return old_ver.cmp(&new_ver);
    }

    // Fallback to string comparison
    old.cmp(new)
}

/// SIMD-accelerated string equality check for package names.
#[inline]
#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
pub fn fast_str_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    if a.len() < 32 {
        return a == b;
    }
    // Use SIMD for longer strings
    simd_memeq(a.as_bytes(), b.as_bytes())
}

/// SIMD memory equality check.
///
/// # Safety
/// Uses AVX2 SIMD intrinsics.
#[allow(unsafe_code)]
#[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
fn simd_memeq(a: &[u8], b: &[u8]) -> bool {
    use std::arch::x86_64::{__m256i, _mm256_cmpeq_epi8, _mm256_loadu_si256, _mm256_movemask_epi8};

    debug_assert_eq!(a.len(), b.len());
    let len = a.len();
    let chunks = len / 32;

    // SAFETY: AVX2 is available (checked by cfg), pointers are valid
    unsafe {
        for i in 0..chunks {
            let offset = i * 32;
            let va = _mm256_loadu_si256(a.as_ptr().add(offset) as *const __m256i);
            let vb = _mm256_loadu_si256(b.as_ptr().add(offset) as *const __m256i);
            let cmp = _mm256_cmpeq_epi8(va, vb);
            if _mm256_movemask_epi8(cmp) != -1i32 {
                return false;
            }
        }

        // Check remaining bytes
        let remaining = chunks * 32;
        a[remaining..] == b[remaining..]
    }
}

/// Fallback string equality.
#[inline]
#[cfg(not(all(target_arch = "x86_64", target_feature = "avx2")))]
pub fn fast_str_eq(a: &str, b: &str) -> bool {
    a == b
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ComposerLock;

    #[test]
    fn test_change_display() {
        let change = PackageChange::added("vendor/pkg".to_string(), "1.0.0".to_string(), false);
        assert!(change.to_string().contains('+'));
        assert!(change.to_string().contains("vendor/pkg"));

        let change = PackageChange::removed("vendor/pkg".to_string(), "1.0.0".to_string(), true);
        assert!(change.to_string().contains('-'));
        assert!(change.to_string().contains("(dev)"));
    }

    #[test]
    fn test_version_comparison() {
        assert_eq!(compare_versions("1.0.0", "2.0.0"), std::cmp::Ordering::Less);
        assert_eq!(
            compare_versions("2.0.0", "1.0.0"),
            std::cmp::Ordering::Greater
        );
        assert_eq!(
            compare_versions("1.0.0", "1.0.0"),
            std::cmp::Ordering::Equal
        );
        assert_eq!(
            compare_versions("v1.0.0", "v2.0.0"),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn test_empty_diff() {
        let lock = ComposerLock::default();
        let diff = compute_diff(&lock, &lock);
        assert!(diff.is_empty());
    }

    #[test]
    fn test_package_added() {
        let old = ComposerLock::default();
        let mut new = ComposerLock::default();
        new.packages.push(LockedPackage::new("vendor/pkg", "1.0.0"));

        let diff = compute_diff(&old, &new);
        assert_eq!(diff.added_count(), 1);
        assert_eq!(diff.packages[0].name, "vendor/pkg");
    }

    #[test]
    fn test_package_removed() {
        let mut old = ComposerLock::default();
        old.packages.push(LockedPackage::new("vendor/pkg", "1.0.0"));
        let new = ComposerLock::default();

        let diff = compute_diff(&old, &new);
        assert_eq!(diff.removed_count(), 1);
    }

    #[test]
    fn test_package_upgraded() {
        let mut old = ComposerLock::default();
        old.packages.push(LockedPackage::new("vendor/pkg", "1.0.0"));
        let mut new = ComposerLock::default();
        new.packages.push(LockedPackage::new("vendor/pkg", "2.0.0"));

        let diff = compute_diff(&old, &new);
        assert_eq!(diff.upgraded_count(), 1);
    }

    #[test]
    fn test_diff_summary() {
        let mut old = ComposerLock::default();
        old.packages.push(LockedPackage::new("old/pkg", "1.0.0"));

        let mut new = ComposerLock::default();
        new.packages.push(LockedPackage::new("new/pkg", "1.0.0"));

        let diff = compute_diff(&old, &new);
        let summary = diff.summary();
        assert!(summary.contains("added"));
        assert!(summary.contains("removed"));
    }

    #[test]
    fn test_fast_str_eq() {
        assert!(fast_str_eq("hello", "hello"));
        assert!(!fast_str_eq("hello", "world"));
        assert!(!fast_str_eq("hello", "hell"));

        // Test with longer strings for SIMD path
        let long_a = "a".repeat(100);
        let long_b = "a".repeat(100);
        let long_c = "b".repeat(100);
        assert!(fast_str_eq(&long_a, &long_b));
        assert!(!fast_str_eq(&long_a, &long_c));
    }
}
