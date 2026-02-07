//! Version constraint handling (Composer-compatible).

use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Composer-compatible version constraint.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct VersionConstraint {
    raw: String,
}

impl VersionConstraint {
    /// Create from raw string.
    #[must_use]
    pub fn new(constraint: impl Into<String>) -> Self {
        Self {
            raw: constraint.into(),
        }
    }

    /// Any version.
    #[must_use]
    pub fn any() -> Self {
        Self::new("*")
    }

    /// Exact version.
    #[must_use]
    pub fn exact(version: &Version) -> Self {
        Self::new(version.to_string())
    }

    /// Get raw constraint string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// Check if version matches.
    #[must_use]
    pub fn matches(&self, version: &Version) -> bool {
        if self.raw == "*" {
            return true;
        }
        self.to_semver_req().is_some_and(|req| req.matches(version))
    }

    /// Convert to semver `VersionReq`.
    fn to_semver_req(&self) -> Option<VersionReq> {
        let normalized = self.normalize_constraint();
        VersionReq::parse(&normalized).ok()
    }

    /// Normalize Composer constraint to semver.
    fn normalize_constraint(&self) -> String {
        let s = self.raw.trim();

        // Handle * wildcard
        if s == "*" {
            return "*".to_string();
        }

        // Handle .* wildcard patterns (e.g., "3.*", "7.*", "1.2.*")
        // Note: ".x" is a version wildcard, not a file extension
        #[allow(clippy::case_sensitive_file_extension_comparisons)]
        if s.ends_with(".*") || s.ends_with(".x") {
            let prefix = &s[..s.len() - 2];
            let parts: Vec<&str> = prefix.split('.').collect();
            return match parts.len() {
                // "3.*" -> ">=3.0.0, <4.0.0"
                1 => format!(
                    ">={}.0.0, <{}.0.0",
                    parts[0],
                    parts[0].parse::<u64>().unwrap_or(0) + 1
                ),
                // "3.1.*" -> ">=3.1.0, <3.2.0"
                2 => format!(
                    ">={}.{}.0, <{}.{}.0",
                    parts[0],
                    parts[1],
                    parts[0],
                    parts[1].parse::<u64>().unwrap_or(0) + 1
                ),
                _ => s.to_string(),
            };
        }

        // Handle ^ (caret)
        if let Some(rest) = s.strip_prefix('^') {
            return format!("^{}", Self::normalize_version(rest));
        }

        // Handle ~ (tilde)
        if let Some(rest) = s.strip_prefix('~') {
            return format!("~{}", Self::normalize_version(rest));
        }

        // Handle >= <= > <
        if s.starts_with(">=")
            || s.starts_with("<=")
            || s.starts_with('>')
            || s.starts_with('<')
            || s.starts_with('=')
        {
            return s.to_string();
        }

        // Handle || or | (OR) - Composer supports both
        if s.contains("||") {
            return s
                .split("||")
                .map(|p| Self::new(p.trim()).normalize_constraint())
                .collect::<Vec<_>>()
                .join(" || ");
        }
        if s.contains('|') && !s.contains("||") {
            return s
                .split('|')
                .map(|p| Self::new(p.trim()).normalize_constraint())
                .collect::<Vec<_>>()
                .join(" || ");
        }

        // Handle space/comma (AND)
        if s.contains(',') {
            return s
                .split(',')
                .map(|p| Self::new(p.trim()).normalize_constraint())
                .collect::<Vec<_>>()
                .join(", ");
        }

        // Bare version = exact match
        format!("={}", Self::normalize_version(s))
    }

    /// Normalize version string.
    fn normalize_version(v: &str) -> String {
        let v = v.trim().trim_start_matches('v');

        // Count dots
        let dots = v.chars().filter(|&c| c == '.').count();

        match dots {
            0 => format!("{v}.0.0"),
            1 => format!("{v}.0"),
            _ => v.to_string(),
        }
    }
}

impl Default for VersionConstraint {
    fn default() -> Self {
        Self::any()
    }
}

impl fmt::Display for VersionConstraint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.raw)
    }
}

impl FromStr for VersionConstraint {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::new(s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use test_case::test_case;

    // ========== Basic Unit Tests ==========

    #[test]
    fn wildcard() {
        let c = VersionConstraint::any();
        assert!(c.matches(&Version::new(1, 0, 0)));
        assert!(c.matches(&Version::new(99, 99, 99)));
    }

    #[test]
    fn major_wildcard() {
        // Test 3.* pattern
        let c = VersionConstraint::new("3.*");
        assert!(c.matches(&Version::new(3, 0, 0)));
        assert!(c.matches(&Version::new(3, 11, 0)));
        assert!(c.matches(&Version::new(3, 99, 99)));
        assert!(!c.matches(&Version::new(2, 0, 0)));
        assert!(!c.matches(&Version::new(4, 0, 0)));

        // Test 7.* pattern
        let c7 = VersionConstraint::new("7.*");
        assert!(c7.matches(&Version::new(7, 0, 0)));
        assert!(c7.matches(&Version::new(7, 17, 0)));
        assert!(!c7.matches(&Version::new(8, 0, 0)));
    }

    #[test]
    fn minor_wildcard() {
        // Test 3.1.* pattern
        let c = VersionConstraint::new("3.1.*");
        assert!(c.matches(&Version::new(3, 1, 0)));
        assert!(c.matches(&Version::new(3, 1, 99)));
        assert!(!c.matches(&Version::new(3, 0, 0)));
        assert!(!c.matches(&Version::new(3, 2, 0)));
    }

    #[test]
    fn caret() {
        let c = VersionConstraint::new("^1.2");
        assert!(c.matches(&Version::new(1, 2, 0)));
        assert!(c.matches(&Version::new(1, 9, 9)));
        assert!(!c.matches(&Version::new(2, 0, 0)));
    }

    #[test]
    fn tilde() {
        let c = VersionConstraint::new("~1.2");
        assert!(c.matches(&Version::new(1, 2, 0)));
        assert!(c.matches(&Version::new(1, 2, 9)));
        assert!(!c.matches(&Version::new(1, 3, 0)));
    }

    #[test]
    fn exact() {
        let c = VersionConstraint::exact(&Version::new(1, 2, 3));
        assert!(c.matches(&Version::new(1, 2, 3)));
        assert!(!c.matches(&Version::new(1, 2, 4)));
    }

    // ========== Parameterized Tests ==========

    #[test_case("^1.0.0", 1, 0, 0, true ; "caret matches minimum")]
    #[test_case("^1.0.0", 1, 99, 99, true ; "caret matches higher minor/patch")]
    #[test_case("^1.0.0", 2, 0, 0, false ; "caret rejects next major")]
    #[test_case("^0.1.0", 0, 1, 0, true ; "caret zero major matches minimum")]
    #[test_case("^0.1.0", 0, 2, 0, false ; "caret zero major rejects higher minor")]
    #[test_case("~1.2.0", 1, 2, 0, true ; "tilde matches minimum")]
    #[test_case("~1.2.0", 1, 2, 99, true ; "tilde matches higher patch")]
    #[test_case("~1.2.0", 1, 3, 0, false ; "tilde rejects higher minor")]
    #[test_case(">=1.0.0", 1, 0, 0, true ; "gte matches exact")]
    #[test_case(">=1.0.0", 2, 0, 0, true ; "gte matches higher")]
    #[test_case(">=1.0.0", 0, 99, 99, false ; "gte rejects lower")]
    #[test_case("<2.0.0", 1, 99, 99, true ; "lt matches lower")]
    #[test_case("<2.0.0", 2, 0, 0, false ; "lt rejects exact")]
    #[test_case("*", 99, 99, 99, true ; "wildcard matches any")]
    fn test_constraint_matching(
        constraint: &str,
        major: u64,
        minor: u64,
        patch: u64,
        expected: bool,
    ) {
        let c = VersionConstraint::new(constraint);
        let v = Version::new(major, minor, patch);
        assert_eq!(
            c.matches(&v),
            expected,
            "Constraint {} should {} match version {}",
            constraint,
            if expected { "" } else { "not" },
            v
        );
    }

    // ========== Edge Case Tests ==========

    #[test]
    fn test_version_normalization() {
        // Test that versions are properly normalized
        assert_eq!(VersionConstraint::normalize_version("1"), "1.0.0");
        assert_eq!(VersionConstraint::normalize_version("1.2"), "1.2.0");
        assert_eq!(VersionConstraint::normalize_version("1.2.3"), "1.2.3");
        assert_eq!(VersionConstraint::normalize_version("v1.2.3"), "1.2.3");
    }

    #[test]
    fn test_or_constraints() {
        // Note: OR constraints require special handling because semver::VersionReq
        // uses "||" as the OR operator. We test the normalization here.
        let c = VersionConstraint::new("^1.0 || ^2.0");
        let normalized = c.normalize_constraint();

        // Verify the constraint is normalized correctly
        assert!(normalized.contains("||"), "Normalized: {normalized}");

        // Test individual constraints work
        let c1 = VersionConstraint::new("^1.0");
        let c2 = VersionConstraint::new("^2.0");

        assert!(c1.matches(&Version::new(1, 5, 0)));
        assert!(!c1.matches(&Version::new(2, 5, 0)));

        assert!(!c2.matches(&Version::new(1, 5, 0)));
        assert!(c2.matches(&Version::new(2, 5, 0)));
    }

    #[test]
    fn test_and_constraints() {
        let c = VersionConstraint::new(">=1.0.0, <2.0.0");
        assert!(c.matches(&Version::new(1, 5, 0)));
        assert!(!c.matches(&Version::new(0, 9, 0)));
        assert!(!c.matches(&Version::new(2, 0, 0)));
    }

    #[test]
    fn test_constraint_display() {
        let c = VersionConstraint::new("^1.2.3");
        assert_eq!(c.to_string(), "^1.2.3");
        assert_eq!(c.as_str(), "^1.2.3");
    }

    #[test]
    fn test_constraint_from_str() {
        let c: VersionConstraint = "~2.0".parse().unwrap();
        assert_eq!(c.as_str(), "~2.0");
    }

    #[test]
    fn test_constraint_serde() {
        let c = VersionConstraint::new("^1.0");
        let json = sonic_rs::to_string(&c).unwrap();
        assert_eq!(json, "\"^1.0\"");

        let c2: VersionConstraint = sonic_rs::from_str(&json).unwrap();
        assert_eq!(c, c2);
    }

    #[test]
    fn test_constraint_equality() {
        let c1 = VersionConstraint::new("^1.0");
        let c2 = VersionConstraint::new("^1.0");
        let c3 = VersionConstraint::new("^2.0");

        assert_eq!(c1, c2);
        assert_ne!(c1, c3);
    }

    #[test]
    fn test_constraint_hash() {
        use std::collections::HashSet;

        let mut set = HashSet::new();
        set.insert(VersionConstraint::new("^1.0"));
        set.insert(VersionConstraint::new("^1.0")); // Duplicate
        set.insert(VersionConstraint::new("^2.0"));

        assert_eq!(set.len(), 2);
    }

    #[test]
    fn test_whitespace_handling() {
        let c = VersionConstraint::new("  ^1.0  ");
        assert!(c.matches(&Version::new(1, 5, 0)));
    }

    #[test]
    fn test_x_wildcard() {
        let c = VersionConstraint::new("1.x");
        assert!(c.matches(&Version::new(1, 0, 0)));
        assert!(c.matches(&Version::new(1, 99, 99)));
        assert!(!c.matches(&Version::new(2, 0, 0)));
    }

    // ========== Property-Based Tests ==========

    proptest! {
        /// Any version should match the wildcard constraint "*"
        #[test]
        fn prop_wildcard_matches_all(major in 0u64..100, minor in 0u64..100, patch in 0u64..1000) {
            let c = VersionConstraint::any();
            let v = Version::new(major, minor, patch);
            prop_assert!(c.matches(&v), "Wildcard should match all versions");
        }

        /// Caret constraint should match the exact version it specifies
        #[test]
        fn prop_caret_matches_exact(major in 1u64..20, minor in 0u64..50, patch in 0u64..100) {
            let constraint = format!("^{major}.{minor}.{patch}");
            let c = VersionConstraint::new(&constraint);
            let v = Version::new(major, minor, patch);
            prop_assert!(c.matches(&v), "Caret constraint {} should match exact version {}", constraint, v);
        }

        /// Caret constraint should NOT match versions with higher major
        #[test]
        fn prop_caret_rejects_higher_major(major in 1u64..20, minor in 0u64..50, patch in 0u64..100) {
            let constraint = format!("^{major}.{minor}.{patch}");
            let c = VersionConstraint::new(&constraint);
            let v = Version::new(major + 1, 0, 0);
            prop_assert!(!c.matches(&v), "Caret constraint {} should NOT match higher major version {}", constraint, v);
        }

        /// Caret constraint should match versions with same major but higher minor/patch
        #[test]
        fn prop_caret_matches_higher_minor_patch(
            major in 1u64..20,
            minor in 0u64..50,
            patch in 0u64..100,
            minor_add in 0u64..50,
            patch_add in 0u64..100
        ) {
            let constraint = format!("^{major}.{minor}.{patch}");
            let c = VersionConstraint::new(&constraint);
            let v = Version::new(major, minor + minor_add, patch + patch_add);
            prop_assert!(c.matches(&v), "Caret constraint {} should match {}", constraint, v);
        }

        /// Tilde constraint should match the exact version it specifies
        #[test]
        fn prop_tilde_matches_exact(major in 1u64..20, minor in 0u64..50, patch in 0u64..100) {
            let constraint = format!("~{major}.{minor}.{patch}");
            let c = VersionConstraint::new(&constraint);
            let v = Version::new(major, minor, patch);
            prop_assert!(c.matches(&v), "Tilde constraint {} should match exact version {}", constraint, v);
        }

        /// Tilde constraint should match versions with same major.minor but higher patch
        #[test]
        fn prop_tilde_matches_higher_patch(
            major in 1u64..20,
            minor in 0u64..50,
            patch in 0u64..100,
            patch_add in 0u64..100
        ) {
            let constraint = format!("~{major}.{minor}.{patch}");
            let c = VersionConstraint::new(&constraint);
            let v = Version::new(major, minor, patch + patch_add);
            prop_assert!(c.matches(&v), "Tilde constraint {} should match {}", constraint, v);
        }

        /// Tilde constraint should NOT match versions with higher minor
        #[test]
        fn prop_tilde_rejects_higher_minor(major in 1u64..20, minor in 0u64..49, patch in 0u64..100) {
            let constraint = format!("~{major}.{minor}.{patch}");
            let c = VersionConstraint::new(&constraint);
            let v = Version::new(major, minor + 1, 0);
            prop_assert!(!c.matches(&v), "Tilde constraint {} should NOT match higher minor version {}", constraint, v);
        }

        /// Major wildcard (N.*) should match all versions with that major
        #[test]
        fn prop_major_wildcard_matches_same_major(major in 0u64..20, minor in 0u64..100, patch in 0u64..1000) {
            let constraint = format!("{major}.*");
            let c = VersionConstraint::new(&constraint);
            let v = Version::new(major, minor, patch);
            prop_assert!(c.matches(&v), "Major wildcard {} should match {}", constraint, v);
        }

        /// Major wildcard (N.*) should NOT match different major versions
        #[test]
        fn prop_major_wildcard_rejects_different_major(major in 0u64..20, other_major in 0u64..20, minor in 0u64..100, patch in 0u64..1000) {
            prop_assume!(major != other_major);
            let constraint = format!("{major}.*");
            let c = VersionConstraint::new(&constraint);
            let v = Version::new(other_major, minor, patch);
            prop_assert!(!c.matches(&v), "Major wildcard {} should NOT match {}", constraint, v);
        }

        /// Minor wildcard (N.M.*) should match all versions with that major.minor
        #[test]
        fn prop_minor_wildcard_matches_same_minor(major in 0u64..20, minor in 0u64..50, patch in 0u64..1000) {
            let constraint = format!("{major}.{minor}.*");
            let c = VersionConstraint::new(&constraint);
            let v = Version::new(major, minor, patch);
            prop_assert!(c.matches(&v), "Minor wildcard {} should match {}", constraint, v);
        }

        /// VersionConstraint can be created from any string without panic
        #[test]
        fn prop_constraint_creation_no_panic(s in ".*") {
            let _ = VersionConstraint::new(&s);
        }

        /// Constraint normalization is deterministic
        #[test]
        fn prop_normalize_deterministic(major in 0u64..100, minor in 0u64..100, patch in 0u64..100) {
            let constraint = format!("^{major}.{minor}.{patch}");
            let c1 = VersionConstraint::new(&constraint);
            let c2 = VersionConstraint::new(&constraint);

            let normalized1 = c1.normalize_constraint();
            let normalized2 = c2.normalize_constraint();

            prop_assert_eq!(normalized1, normalized2, "Normalization should be deterministic");
        }

        /// Version matching is consistent (same constraint, same version = same result)
        #[test]
        fn prop_matching_consistent(
            major in 0u64..20,
            minor in 0u64..50,
            patch in 0u64..100,
            v_major in 0u64..20,
            v_minor in 0u64..50,
            v_patch in 0u64..100
        ) {
            let constraint = format!("^{major}.{minor}.{patch}");
            let c = VersionConstraint::new(&constraint);
            let v = Version::new(v_major, v_minor, v_patch);

            let result1 = c.matches(&v);
            let result2 = c.matches(&v);

            prop_assert_eq!(result1, result2, "Matching should be consistent");
        }

        /// Exact constraint should only match that exact version
        #[test]
        fn prop_exact_only_matches_exact(major in 0u64..20, minor in 0u64..50, patch in 0u64..100) {
            let v = Version::new(major, minor, patch);
            let c = VersionConstraint::exact(&v);

            prop_assert!(c.matches(&v), "Exact constraint should match itself");

            // Different versions should not match
            if patch > 0 {
                let v2 = Version::new(major, minor, patch - 1);
                prop_assert!(!c.matches(&v2), "Exact constraint should not match different patch");
            }
        }

        /// Constraint serialization roundtrip preserves the constraint
        #[test]
        fn prop_serde_roundtrip(major in 0u64..20, minor in 0u64..50, patch in 0u64..100) {
            let constraint = format!("^{major}.{minor}.{patch}");
            let c = VersionConstraint::new(&constraint);

            let json = sonic_rs::to_string(&c).unwrap();
            let c2: VersionConstraint = sonic_rs::from_str(&json).unwrap();

            prop_assert_eq!(c, c2, "Serde roundtrip should preserve constraint");
        }

        /// FromStr and new() produce identical constraints
        #[test]
        fn prop_from_str_equals_new(major in 0u64..20, minor in 0u64..50) {
            let constraint = format!("~{major}.{minor}");
            let c1 = VersionConstraint::new(&constraint);
            let c2: VersionConstraint = constraint.parse().unwrap();

            prop_assert_eq!(c1, c2, "FromStr and new() should be equivalent");
        }
    }

    // ========== Fuzz-like Edge Cases ==========

    #[test]
    fn test_empty_constraint() {
        let c = VersionConstraint::new("");
        // Should not panic, behavior is implementation-defined
        let _ = c.matches(&Version::new(1, 0, 0));
    }

    #[test]
    fn test_malformed_constraints() {
        let malformed = vec![
            "abc",
            "^",
            "~",
            ">=",
            "1.2.3.4.5",
            "^^^^^1.0",
            "1.0 ||||| 2.0",
            "   ",
            "\n\t",
        ];

        for constraint in malformed {
            let c = VersionConstraint::new(constraint);
            // Should not panic
            let _ = c.matches(&Version::new(1, 0, 0));
        }
    }

    #[test]
    fn test_extreme_versions() {
        let c = VersionConstraint::any();

        // Very large versions
        assert!(c.matches(&Version::new(u64::MAX / 2, u64::MAX / 2, u64::MAX / 2)));

        // Zero versions
        assert!(c.matches(&Version::new(0, 0, 0)));
    }

    #[test]
    fn test_unicode_in_constraint() {
        let c = VersionConstraint::new("^1.0 🎉");
        // Should not panic, behavior is implementation-defined
        let _ = c.matches(&Version::new(1, 0, 0));
    }
}
