//! Composer-compatible version handling.
//!
//! This module provides a complete implementation of Composer's version
//! constraint system, including:
//!
//! - Semantic versions: `1.2.3`, `v1.2.3`
//! - Pre-release versions: `1.0.0-alpha`, `1.0.0-beta.1`, `1.0.0-RC1`
//! - Dev branches: `dev-master`, `dev-feature/foo`
//! - Branch aliases: `dev-master as 1.0.x-dev`
//!
//! Constraints supported:
//! - Exact: `1.0.0`
//! - Range: `>=1.0.0 <2.0.0`, `>1.0 <=2.0`
//! - Hyphen range: `1.0.0 - 2.0.0`
//! - Wildcard: `1.0.*`, `1.*`
//! - Tilde: `~1.2.3` (>=1.2.3 <1.3.0)
//! - Caret: `^1.2.3` (>=1.2.3 <2.0.0)
//! - OR: `^1.0 || ^2.0`
//! - Stability flags: `>=1.0@dev`, `^1.0@beta`

use parking_lot::RwLock;
use regex::Regex;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smallvec::SmallVec;
use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::str::FromStr;
use std::sync::{Arc, LazyLock};
use version_ranges::Ranges;

/// Cache for parsed versions to avoid repeated parsing.
static VERSION_CACHE: LazyLock<RwLock<ahash::AHashMap<Arc<str>, ComposerVersion>>> =
    LazyLock::new(|| RwLock::new(ahash::AHashMap::with_capacity(4096)));

/// Cache for parsed constraints.
static CONSTRAINT_CACHE: LazyLock<RwLock<ahash::AHashMap<Arc<str>, ComposerConstraint>>> =
    LazyLock::new(|| RwLock::new(ahash::AHashMap::with_capacity(4096)));

/// Maximum cache size before eviction.
const MAX_CACHE_SIZE: usize = 16384;

/// Stability level for package versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[repr(u8)]
pub enum Stability {
    /// Development version (lowest stability).
    Dev = 0,
    /// Alpha release.
    Alpha = 1,
    /// Beta release.
    Beta = 2,
    /// Release candidate.
    RC = 3,
    /// Stable release (highest stability).
    #[default]
    Stable = 4,
}

impl Stability {
    /// Parse stability from string.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "dev" => Some(Self::Dev),
            "alpha" | "a" => Some(Self::Alpha),
            "beta" | "b" => Some(Self::Beta),
            "rc" | "rc1" | "rc2" | "rc3" | "rc4" | "rc5" => Some(Self::RC),
            "stable" | "" => Some(Self::Stable),
            "patch" | "pl" | "p" => Some(Self::Stable), // Composer treats these as stable
            _ => None,
        }
    }

    /// Check if this stability is at least as stable as the minimum.
    #[must_use]
    #[inline]
    pub fn satisfies_minimum(&self, minimum: Self) -> bool {
        *self >= minimum
    }
}

impl fmt::Display for Stability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dev => write!(f, "dev"),
            Self::Alpha => write!(f, "alpha"),
            Self::Beta => write!(f, "beta"),
            Self::RC => write!(f, "RC"),
            Self::Stable => write!(f, "stable"),
        }
    }
}

impl Serialize for Stability {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Stability {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::parse(&s).ok_or_else(|| serde::de::Error::custom(format!("invalid stability: {s}")))
    }
}

/// Pre-release identifier component.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PreReleaseId {
    /// Numeric identifier (compared numerically).
    Numeric(u64),
    /// String identifier (compared lexicographically).
    String(Arc<str>),
}

impl PartialOrd for PreReleaseId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PreReleaseId {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Numeric(a), Self::Numeric(b)) => a.cmp(b),
            (Self::String(a), Self::String(b)) => a.cmp(b),
            // Numeric identifiers have lower precedence than string identifiers
            (Self::Numeric(_), Self::String(_)) => Ordering::Less,
            (Self::String(_), Self::Numeric(_)) => Ordering::Greater,
        }
    }
}

impl fmt::Display for PreReleaseId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Numeric(n) => write!(f, "{n}"),
            Self::String(s) => write!(f, "{s}"),
        }
    }
}

/// A Composer version with full support for pre-release and dev branches.
#[derive(Clone)]
pub struct ComposerVersion {
    /// Major version component.
    pub major: u64,
    /// Minor version component.
    pub minor: u64,
    /// Patch version component.
    pub patch: u64,
    /// Fourth version component (Composer supports X.Y.Z.W).
    pub fourth: u64,
    /// Pre-release identifiers (e.g., alpha, beta.1, RC2).
    pub pre_release: SmallVec<[PreReleaseId; 3]>,
    /// Build metadata (ignored in comparisons).
    pub build_metadata: Option<Arc<str>>,
    /// Stability level.
    pub stability: Stability,
    /// Is this a dev branch version (e.g., dev-master)?
    pub is_dev_branch: bool,
    /// Branch name for dev versions.
    pub branch: Option<Arc<str>>,
    /// Original string representation.
    original: Arc<str>,
    /// Packed representation for fast comparison.
    packed: u64,
}

impl ComposerVersion {
    /// Create a new version with major.minor.patch components.
    #[must_use]
    pub fn new(major: u64, minor: u64, patch: u64) -> Self {
        let original: Arc<str> = Arc::from(format!("{major}.{minor}.{patch}"));
        Self {
            major,
            minor,
            patch,
            fourth: 0,
            pre_release: SmallVec::new(),
            build_metadata: None,
            stability: Stability::Stable,
            is_dev_branch: false,
            branch: None,
            packed: Self::pack(major, minor, patch, 0),
            original,
        }
    }

    /// Create a dev branch version.
    #[must_use]
    pub fn dev_branch(branch: impl Into<Arc<str>>) -> Self {
        let branch: Arc<str> = branch.into();
        let original: Arc<str> = Arc::from(format!("dev-{branch}"));
        Self {
            major: 0,
            minor: 0,
            patch: 0,
            fourth: 0,
            pre_release: SmallVec::new(),
            build_metadata: None,
            stability: Stability::Dev,
            is_dev_branch: true,
            branch: Some(branch),
            packed: 0,
            original,
        }
    }

    /// Parse a Composer version string.
    ///
    /// # Examples
    ///
    /// ```
    /// use libretto_resolver::version::ComposerVersion;
    ///
    /// let v = ComposerVersion::parse("1.2.3").unwrap();
    /// assert_eq!((v.major, v.minor, v.patch), (1, 2, 3));
    ///
    /// let v = ComposerVersion::parse("dev-master").unwrap();
    /// assert!(v.is_dev_branch);
    /// ```
    #[must_use]
    pub fn parse(input: &str) -> Option<Self> {
        let input = input.trim();
        if input.is_empty() {
            return None;
        }

        // Check cache first
        {
            let cache = VERSION_CACHE.read();
            if let Some(cached) = cache.get(input) {
                return Some(cached.clone());
            }
        }

        let result = Self::parse_uncached(input)?;

        // Cache the result
        {
            let mut cache = VERSION_CACHE.write();
            if cache.len() >= MAX_CACHE_SIZE {
                // Simple eviction: clear half the cache
                let keys: Vec<_> = cache.keys().take(MAX_CACHE_SIZE / 2).cloned().collect();
                for key in keys {
                    cache.remove(&key);
                }
            }
            cache.insert(Arc::from(input), result.clone());
        }

        Some(result)
    }

    fn parse_uncached(input: &str) -> Option<Self> {
        // Handle dev-* branches
        if let Some(branch) = input.strip_prefix("dev-") {
            return Some(Self::dev_branch(branch));
        }

        // Handle *-dev suffix (e.g., 1.0.x-dev, master-dev)
        let (version_part, is_dev_suffix) = if let Some(prefix) = input.strip_suffix("-dev") {
            (prefix, true)
        } else {
            (input, false)
        };

        // Remove 'v' or 'V' prefix
        let version_part = version_part
            .strip_prefix('v')
            .or_else(|| version_part.strip_prefix('V'))
            .unwrap_or(version_part);

        // Regex for parsing version strings
        static VERSION_REGEX: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
            Regex::new(
                r"(?ix)
                ^
                (\d+)                           # major
                (?:\.(\d+))?                    # minor
                (?:\.(\d+))?                    # patch
                (?:\.(\d+))?                    # fourth
                (?:
                    [-.]?
                    (alpha|beta|rc|a|b|dev|patch|pl|p)  # pre-release type
                    \.?
                    (\d+)?                      # pre-release number
                )?
                (?:\+(.+))?                     # build metadata
                $
                ",
            )
            .expect("valid regex")
        });

        let caps = if let Some(c) = VERSION_REGEX.captures(version_part) {
            c
        } else {
            if is_dev_suffix {
                // Fallback: treat as dev branch (e.g. master-dev -> dev-master)
                return Some(Self::dev_branch(version_part));
            }
            return None;
        };

        let major: u64 = caps.get(1)?.as_str().parse().ok()?;
        let minor: u64 = caps.get(2).map_or(0, |m| m.as_str().parse().unwrap_or(0));
        let patch: u64 = caps.get(3).map_or(0, |m| m.as_str().parse().unwrap_or(0));
        let fourth: u64 = caps.get(4).map_or(0, |m| m.as_str().parse().unwrap_or(0));

        let mut pre_release = SmallVec::new();
        let mut stability = Stability::Stable;

        if let Some(pre_type) = caps.get(5) {
            let pre_str = pre_type.as_str().to_ascii_lowercase();
            stability = Stability::parse(&pre_str).unwrap_or(Stability::Stable);
            pre_release.push(PreReleaseId::String(Arc::from(pre_str)));

            if let Some(pre_num) = caps.get(6)
                && let Ok(n) = pre_num.as_str().parse::<u64>()
            {
                pre_release.push(PreReleaseId::Numeric(n));
            }
        }

        if is_dev_suffix {
            stability = Stability::Dev;
        }

        let build_metadata = caps.get(7).map(|m| Arc::from(m.as_str()));

        Some(Self {
            major,
            minor,
            patch,
            fourth,
            pre_release,
            build_metadata,
            stability,
            is_dev_branch: false,
            branch: None,
            packed: Self::pack(major, minor, patch, fourth),
            original: Arc::from(input),
        })
    }

    /// Pack version components into a single u64 for fast comparison.
    #[inline]
    #[must_use]
    const fn pack(major: u64, minor: u64, patch: u64, fourth: u64) -> u64 {
        // Use 16 bits for each component (supports up to 65535)
        ((major & 0xFFFF) << 48)
            | ((minor & 0xFFFF) << 32)
            | ((patch & 0xFFFF) << 16)
            | (fourth & 0xFFFF)
    }

    /// Get the original string representation.
    #[must_use]
    #[inline]
    pub fn as_str(&self) -> &str {
        &self.original
    }

    /// Check if this is a pre-release version.
    #[must_use]
    #[inline]
    pub fn is_prerelease(&self) -> bool {
        self.stability != Stability::Stable || !self.pre_release.is_empty()
    }

    /// Get the next major version.
    #[must_use]
    pub fn bump_major(&self) -> Self {
        Self::new(self.major.saturating_add(1), 0, 0)
    }

    /// Get the next minor version.
    #[must_use]
    pub fn bump_minor(&self) -> Self {
        Self::new(self.major, self.minor.saturating_add(1), 0)
    }

    /// Get the next patch version.
    #[must_use]
    pub fn bump_patch(&self) -> Self {
        Self::new(self.major, self.minor, self.patch.saturating_add(1))
    }

    /// Increment to the next version (used by pubgrub).
    #[must_use]
    pub fn bump(&self) -> Self {
        if self.is_dev_branch {
            // Dev branches can't be bumped meaningfully
            return self.clone();
        }
        // For pre-release, bump to stable
        if self.is_prerelease() {
            return Self::new(self.major, self.minor, self.patch);
        }
        // Otherwise bump patch
        self.bump_patch()
    }

    /// Get the lowest possible version.
    #[must_use]
    pub fn lowest() -> Self {
        Self::new(0, 0, 0)
    }
}

impl Default for ComposerVersion {
    fn default() -> Self {
        Self::new(0, 0, 0)
    }
}

impl fmt::Debug for ComposerVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ComposerVersion")
            .field("major", &self.major)
            .field("minor", &self.minor)
            .field("patch", &self.patch)
            .field("stability", &self.stability)
            .field("is_dev_branch", &self.is_dev_branch)
            .finish()
    }
}

impl fmt::Display for ComposerVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.original)
    }
}

impl PartialEq for ComposerVersion {
    fn eq(&self, other: &Self) -> bool {
        if self.is_dev_branch || other.is_dev_branch {
            return self.is_dev_branch == other.is_dev_branch && self.branch == other.branch;
        }
        self.packed == other.packed
            && self.pre_release == other.pre_release
            && self.stability == other.stability
    }
}

impl Eq for ComposerVersion {}

impl Hash for ComposerVersion {
    fn hash<H: Hasher>(&self, state: &mut H) {
        if self.is_dev_branch {
            self.branch.hash(state);
        } else {
            self.packed.hash(state);
            self.pre_release.hash(state);
        }
    }
}

impl PartialOrd for ComposerVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ComposerVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        // Dev branches are always considered "lowest" for version ordering
        match (self.is_dev_branch, other.is_dev_branch) {
            (true, true) => return self.branch.cmp(&other.branch),
            (true, false) => return Ordering::Less,
            (false, true) => return Ordering::Greater,
            (false, false) => {}
        }

        // Fast path: compare packed representation
        match self.packed.cmp(&other.packed) {
            Ordering::Equal => {}
            ord => return ord,
        }

        // Compare pre-release identifiers
        // A version without pre-release is greater than one with pre-release
        match (self.pre_release.is_empty(), other.pre_release.is_empty()) {
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (true, true) => Ordering::Equal,
            (false, false) => self.pre_release.cmp(&other.pre_release),
        }
    }
}

impl FromStr for ComposerVersion {
    type Err = VersionParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s).ok_or_else(|| VersionParseError(s.to_string()))
    }
}

impl Serialize for ComposerVersion {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.original)
    }
}

impl<'de> Deserialize<'de> for ComposerVersion {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::parse(&s).ok_or_else(|| serde::de::Error::custom(format!("invalid version: {s}")))
    }
}

/// Error when parsing a version string.
#[derive(Debug, Clone, thiserror::Error)]
#[error("invalid version string: {0}")]
pub struct VersionParseError(pub String);

/// A Composer version constraint.
///
/// This wraps `version_ranges::Ranges` with Composer-specific parsing
/// and additional support for dev branches and stability flags.
#[derive(Clone)]
pub struct ComposerConstraint {
    /// The version ranges (union of ranges).
    ranges: Ranges<ComposerVersion>,
    /// Minimum stability required.
    min_stability: Stability,
    /// Allowed dev branches (empty means no specific branches required).
    dev_branches: SmallVec<[Arc<str>; 2]>,
    /// Original constraint string.
    original: Arc<str>,
}

impl ComposerConstraint {
    /// Create a constraint matching any version.
    #[must_use]
    pub fn any() -> Self {
        Self {
            ranges: Ranges::full(),
            min_stability: Stability::Stable,
            dev_branches: SmallVec::new(),
            original: Arc::from("*"),
        }
    }

    /// Create a constraint with full ranges and a custom original string.
    ///
    /// Used for special constraint values like `"self.version"` in replace/provide.
    #[must_use]
    pub fn with_original(
        ranges: Ranges<ComposerVersion>,
        min_stability: Stability,
        original: &str,
    ) -> Self {
        Self {
            ranges,
            min_stability,
            dev_branches: SmallVec::new(),
            original: Arc::from(original),
        }
    }

    /// Create a constraint matching no versions.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            ranges: Ranges::empty(),
            min_stability: Stability::Stable,
            dev_branches: SmallVec::new(),
            original: Arc::from("<0.0.0-dev"),
        }
    }

    /// Create an exact version constraint.
    #[must_use]
    pub fn exact(version: ComposerVersion) -> Self {
        let original = Arc::from(version.to_string());
        Self {
            ranges: Ranges::singleton(version),
            min_stability: Stability::Stable,
            dev_branches: SmallVec::new(),
            original,
        }
    }

    /// Parse a Composer constraint string.
    ///
    /// # Examples
    ///
    /// ```
    /// use libretto_resolver::version::{ComposerConstraint, ComposerVersion};
    ///
    /// let c = ComposerConstraint::parse("^1.0").unwrap();
    /// assert!(c.matches(&ComposerVersion::parse("1.5.0").unwrap()));
    /// assert!(!c.matches(&ComposerVersion::parse("2.0.0").unwrap()));
    /// ```
    #[must_use]
    pub fn parse(input: &str) -> Option<Self> {
        let input = input.trim();
        if input.is_empty() {
            return None;
        }

        // Check cache first
        {
            let cache = CONSTRAINT_CACHE.read();
            if let Some(cached) = cache.get(input) {
                return Some(cached.clone());
            }
        }

        let result = Self::parse_uncached(input)?;

        // Cache the result
        {
            let mut cache = CONSTRAINT_CACHE.write();
            if cache.len() >= MAX_CACHE_SIZE {
                let keys: Vec<_> = cache.keys().take(MAX_CACHE_SIZE / 2).cloned().collect();
                for key in keys {
                    cache.remove(&key);
                }
            }
            cache.insert(Arc::from(input), result.clone());
        }

        Some(result)
    }

    fn parse_uncached(input: &str) -> Option<Self> {
        // Handle empty or wildcard
        if input.is_empty() || input == "*" {
            return Some(Self::any());
        }

        // Extract stability flag at the end (e.g., @dev, @beta)
        let (constraint_part, min_stability) = if let Some(at_pos) = input.rfind('@') {
            let stability_str = &input[at_pos + 1..];
            let stability = Stability::parse(stability_str).unwrap_or(Stability::Stable);
            (&input[..at_pos], stability)
        } else {
            (input, Stability::Stable)
        };

        // Handle dev-* branches
        if let Some(branch) = constraint_part.strip_prefix("dev-") {
            let mut constraint = Self::any();
            constraint.original = Arc::from(input);
            constraint.min_stability = Stability::Dev;
            constraint.dev_branches.push(Arc::from(branch));
            return Some(constraint);
        }

        // Handle OR constraints (|| or |)
        if constraint_part.contains('|') {
            // Split by | to handle both | and || (|| results in empty intermediate parts)
            let parts: Vec<&str> = constraint_part
                .split('|')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect();

            let mut ranges = Ranges::empty();
            let mut dev_branches = SmallVec::new();

            for part in parts {
                if let Some(parsed) = Self::parse_single_or_and(part) {
                    ranges = ranges.union(&parsed.ranges);
                    for branch in parsed.dev_branches {
                        if !dev_branches.contains(&branch) {
                            dev_branches.push(branch);
                        }
                    }
                }
            }

            return Some(Self {
                ranges,
                min_stability,
                dev_branches,
                original: Arc::from(input),
            });
        }

        // Parse single constraint or AND constraints
        let mut result = Self::parse_single_or_and(constraint_part)?;
        result.min_stability = min_stability;
        result.original = Arc::from(input);
        Some(result)
    }

    fn parse_single_or_and(input: &str) -> Option<Self> {
        let input = input.trim();

        // Handle AND constraints (comma or space separated)
        let parts: Vec<&str> = if input.contains(',') {
            input.split(',').map(str::trim).collect()
        } else {
            Self::split_and_constraints(input)
        };

        if parts.len() == 1 {
            return Self::parse_single(parts[0]);
        }

        // Check for hyphen range: "1.0.0 - 2.0.0"
        if parts.len() == 3 && parts[1] == "-" {
            let lower = ComposerVersion::parse(parts[0])?;
            let upper = ComposerVersion::parse(parts[2])?;
            return Some(Self {
                ranges: Ranges::between(lower, upper.bump()),
                min_stability: Stability::Stable,
                dev_branches: SmallVec::new(),
                original: Arc::from(input),
            });
        }

        // AND all constraints together
        let mut result = Ranges::full();
        for part in parts {
            let parsed = Self::parse_single(part)?;
            result = result.intersection(&parsed.ranges);
        }

        Some(Self {
            ranges: result,
            min_stability: Stability::Stable,
            dev_branches: SmallVec::new(),
            original: Arc::from(input),
        })
    }

    fn split_and_constraints(input: &str) -> Vec<&str> {
        let mut parts = Vec::new();
        let mut start = 0;
        let chars: Vec<char> = input.chars().collect();
        let mut i = 0;

        while i < chars.len() {
            if chars[i].is_whitespace() {
                if start < i {
                    parts.push(&input[start..i]);
                }
                // Skip whitespace
                while i < chars.len() && chars[i].is_whitespace() {
                    i += 1;
                }
                start = i;
            } else {
                i += 1;
            }
        }

        if start < input.len() {
            parts.push(&input[start..]);
        }

        // Merge operators with their operands
        let mut merged = Vec::new();
        let mut i = 0;
        while i < parts.len() {
            let part = parts[i];
            if (part == ">" || part == "<" || part == ">=" || part == "<=" || part == "=")
                && i + 1 < parts.len()
            {
                let combined = format!("{}{}", part, parts[i + 1]);
                merged.push(combined);
                i += 2;
                continue;
            }
            merged.push(part.to_string());
            i += 1;
        }

        // Convert back to &str by leaking (this is okay for cached constraints)
        merged
            .into_iter()
            .map(|s| -> &'static str { Box::leak(s.into_boxed_str()) })
            .collect()
    }

    fn parse_single(input: &str) -> Option<Self> {
        let input = input.trim();

        // Wildcard
        if input == "*" {
            return Some(Self::any());
        }

        // Dev branch
        if let Some(branch) = input.strip_prefix("dev-") {
            let mut constraint = Self::any();
            constraint.original = Arc::from(input);
            constraint.min_stability = Stability::Dev;
            constraint.dev_branches.push(Arc::from(branch));
            return Some(constraint);
        }

        // Operators
        if let Some(rest) = input.strip_prefix(">=") {
            let version = ComposerVersion::parse(rest.trim())?;
            return Some(Self {
                ranges: Ranges::higher_than(version),
                min_stability: Stability::Stable,
                dev_branches: SmallVec::new(),
                original: Arc::from(input),
            });
        }

        if let Some(rest) = input.strip_prefix("<=") {
            let version = ComposerVersion::parse(rest.trim())?;
            return Some(Self {
                ranges: Ranges::lower_than(version.bump()),
                min_stability: Stability::Stable,
                dev_branches: SmallVec::new(),
                original: Arc::from(input),
            });
        }

        if let Some(rest) = input.strip_prefix('>') {
            let version = ComposerVersion::parse(rest.trim())?;
            return Some(Self {
                ranges: Ranges::strictly_higher_than(version),
                min_stability: Stability::Stable,
                dev_branches: SmallVec::new(),
                original: Arc::from(input),
            });
        }

        if let Some(rest) = input.strip_prefix('<') {
            let version = ComposerVersion::parse(rest.trim())?;
            return Some(Self {
                ranges: Ranges::strictly_lower_than(version),
                min_stability: Stability::Stable,
                dev_branches: SmallVec::new(),
                original: Arc::from(input),
            });
        }

        if let Some(rest) = input.strip_prefix("!=") {
            let version = ComposerVersion::parse(rest.trim())?;
            return Some(Self {
                ranges: Ranges::singleton(version).complement(),
                min_stability: Stability::Stable,
                dev_branches: SmallVec::new(),
                original: Arc::from(input),
            });
        }

        if let Some(rest) = input.strip_prefix('=') {
            let version = ComposerVersion::parse(rest.trim())?;
            return Some(Self::exact(version));
        }

        // Caret: ^1.2.3 means >=1.2.3 <2.0.0 (or <1.3.0 if major is 0)
        if let Some(rest) = input.strip_prefix('^') {
            let version = ComposerVersion::parse(rest.trim())?;
            let upper = if version.major == 0 {
                version.bump_minor()
            } else {
                version.bump_major()
            };
            return Some(Self {
                ranges: Ranges::between(version, upper),
                min_stability: Stability::Stable,
                dev_branches: SmallVec::new(),
                original: Arc::from(input),
            });
        }

        // Tilde: ~1.2.3 means >=1.2.3 <1.3.0, ~1.2 means >=1.2.0 <2.0.0
        if let Some(rest) = input.strip_prefix('~') {
            let version = ComposerVersion::parse(rest.trim())?;
            // If only major.minor specified, upper bound is next major
            // If major.minor.patch specified, upper bound is next minor
            let has_patch = rest.trim().matches('.').count() >= 2;
            let upper = if has_patch {
                version.bump_minor()
            } else {
                version.bump_major()
            };
            return Some(Self {
                ranges: Ranges::between(version, upper),
                min_stability: Stability::Stable,
                dev_branches: SmallVec::new(),
                original: Arc::from(input),
            });
        }

        // Wildcard patterns: 1.0.*, 1.*
        if input.ends_with(".*") || input.ends_with(".x") {
            let prefix = &input[..input.len() - 2];
            let parts: Vec<&str> = prefix.split('.').collect();

            let (lower, upper) = match parts.as_slice() {
                [major] => {
                    let major: u64 = major.parse().ok()?;
                    (
                        ComposerVersion::new(major, 0, 0),
                        ComposerVersion::new(major.saturating_add(1), 0, 0),
                    )
                }
                [major, minor] => {
                    let major: u64 = major.parse().ok()?;
                    let minor: u64 = minor.parse().ok()?;
                    (
                        ComposerVersion::new(major, minor, 0),
                        ComposerVersion::new(major, minor.saturating_add(1), 0),
                    )
                }
                _ => return None,
            };

            return Some(Self {
                ranges: Ranges::between(lower, upper),
                min_stability: Stability::Stable,
                dev_branches: SmallVec::new(),
                original: Arc::from(input),
            });
        }

        // Bare version = exact match
        let version = ComposerVersion::parse(input)?;
        Some(Self::exact(version))
    }

    /// Check if a version matches this constraint.
    #[must_use]
    pub fn matches(&self, version: &ComposerVersion) -> bool {
        // Check stability requirement
        if !version.stability.satisfies_minimum(self.min_stability) && !version.is_dev_branch {
            return false;
        }

        // Check dev branches
        if version.is_dev_branch {
            if let Some(ref branch) = version.branch {
                if self.dev_branches.is_empty() {
                    // No specific branches required - match if min_stability allows dev
                    return self.min_stability == Stability::Dev;
                }
                return self.dev_branches.iter().any(|b| {
                    b.as_ref() == branch.as_ref()
                        || b.as_ref() == "*"
                        || (b.ends_with('*') && branch.starts_with(&b[..b.len() - 1]))
                });
            }
            return false;
        }

        // Check version ranges
        self.ranges.contains(version)
    }

    /// Get the underlying ranges.
    #[must_use]
    pub const fn ranges(&self) -> &Ranges<ComposerVersion> {
        &self.ranges
    }

    /// Get the minimum stability.
    #[must_use]
    pub const fn min_stability(&self) -> Stability {
        self.min_stability
    }

    /// Get the original constraint string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.original
    }

    /// Check if this constraint is empty (matches nothing).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty() && self.dev_branches.is_empty()
    }

    /// Compute the intersection of two constraints.
    #[must_use]
    pub fn intersection(&self, other: &Self) -> Self {
        let ranges = self.ranges.intersection(&other.ranges);

        // Intersect dev branches
        let dev_branches = if self.dev_branches.is_empty() {
            other.dev_branches.clone()
        } else if other.dev_branches.is_empty() {
            self.dev_branches.clone()
        } else {
            self.dev_branches
                .iter()
                .filter(|b| other.dev_branches.contains(b))
                .cloned()
                .collect()
        };

        Self {
            ranges,
            min_stability: self.min_stability.max(other.min_stability),
            dev_branches,
            original: Arc::from(format!("({}) ∩ ({})", self.original, other.original)),
        }
    }

    /// Compute the union of two constraints.
    #[must_use]
    pub fn union(&self, other: &Self) -> Self {
        let ranges = self.ranges.union(&other.ranges);

        let mut dev_branches = self.dev_branches.clone();
        for branch in &other.dev_branches {
            if !dev_branches.contains(branch) {
                dev_branches.push(branch.clone());
            }
        }

        Self {
            ranges,
            min_stability: self.min_stability.min(other.min_stability),
            dev_branches,
            original: Arc::from(format!("{} || {}", self.original, other.original)),
        }
    }

    /// Compute the complement of this constraint.
    #[must_use]
    pub fn complement(&self) -> Self {
        Self {
            ranges: self.ranges.complement(),
            min_stability: self.min_stability,
            dev_branches: SmallVec::new(), // Complement doesn't include dev branches
            original: Arc::from(format!("not({})", self.original)),
        }
    }
}

impl Default for ComposerConstraint {
    fn default() -> Self {
        Self::any()
    }
}

impl fmt::Debug for ComposerConstraint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ComposerConstraint")
            .field("original", &self.original)
            .field("min_stability", &self.min_stability)
            .finish()
    }
}

impl fmt::Display for ComposerConstraint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.original)
    }
}

impl PartialEq for ComposerConstraint {
    fn eq(&self, other: &Self) -> bool {
        // Use original string for equality (constraints with same meaning but different
        // representations are considered different)
        self.original == other.original
    }
}

impl Eq for ComposerConstraint {}

impl Hash for ComposerConstraint {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.original.hash(state);
    }
}

impl FromStr for ComposerConstraint {
    type Err = ConstraintParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s).ok_or_else(|| ConstraintParseError(s.to_string()))
    }
}

impl Serialize for ComposerConstraint {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.original)
    }
}

impl<'de> Deserialize<'de> for ComposerConstraint {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::parse(&s).ok_or_else(|| serde::de::Error::custom(format!("invalid constraint: {s}")))
    }
}

/// Error when parsing a constraint string.
#[derive(Debug, Clone, thiserror::Error)]
#[error("invalid constraint string: {0}")]
pub struct ConstraintParseError(pub String);

/// Clear the version and constraint caches.
pub fn clear_caches() {
    VERSION_CACHE.write().clear();
    CONSTRAINT_CACHE.write().clear();
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    mod version_parsing {
        use super::*;

        #[test]
        fn simple_versions() {
            let v = ComposerVersion::parse("1.2.3").unwrap();
            assert_eq!((v.major, v.minor, v.patch), (1, 2, 3));

            let v = ComposerVersion::parse("1.2").unwrap();
            assert_eq!((v.major, v.minor, v.patch), (1, 2, 0));

            let v = ComposerVersion::parse("1").unwrap();
            assert_eq!((v.major, v.minor, v.patch), (1, 0, 0));
        }

        #[test]
        fn v_prefix() {
            let v = ComposerVersion::parse("v1.2.3").unwrap();
            assert_eq!((v.major, v.minor, v.patch), (1, 2, 3));

            let v = ComposerVersion::parse("V1.2.3").unwrap();
            assert_eq!((v.major, v.minor, v.patch), (1, 2, 3));
        }

        #[test]
        fn four_part_versions() {
            let v = ComposerVersion::parse("1.2.3.4").unwrap();
            assert_eq!((v.major, v.minor, v.patch, v.fourth), (1, 2, 3, 4));
        }

        #[test]
        fn prerelease_versions() {
            let v = ComposerVersion::parse("1.0.0-alpha").unwrap();
            assert_eq!(v.stability, Stability::Alpha);
            assert!(v.is_prerelease());

            let v = ComposerVersion::parse("1.0.0-beta.1").unwrap();
            assert_eq!(v.stability, Stability::Beta);

            let v = ComposerVersion::parse("1.0.0-RC1").unwrap();
            assert_eq!(v.stability, Stability::RC);

            let v = ComposerVersion::parse("1.0.0-rc2").unwrap();
            assert_eq!(v.stability, Stability::RC);
        }

        #[test]
        fn dev_branches() {
            let v = ComposerVersion::parse("dev-master").unwrap();
            assert!(v.is_dev_branch);
            assert_eq!(v.branch.as_deref(), Some("master"));
            assert_eq!(v.stability, Stability::Dev);

            let v = ComposerVersion::parse("dev-feature/foo").unwrap();
            assert!(v.is_dev_branch);
            assert_eq!(v.branch.as_deref(), Some("feature/foo"));
        }

        #[test]
        fn dev_suffix() {
            let v = ComposerVersion::parse("1.0.0-dev").unwrap();
            assert_eq!(v.stability, Stability::Dev);
            assert!(!v.is_dev_branch);
        }

        #[test]
        fn build_metadata() {
            let v = ComposerVersion::parse("1.0.0+build123").unwrap();
            assert_eq!(v.build_metadata.as_deref(), Some("build123"));
        }
    }

    mod version_ordering {
        use super::*;

        #[test]
        fn basic_ordering() {
            let v1 = ComposerVersion::parse("1.0.0").unwrap();
            let v2 = ComposerVersion::parse("1.0.1").unwrap();
            let v3 = ComposerVersion::parse("1.1.0").unwrap();
            let v4 = ComposerVersion::parse("2.0.0").unwrap();

            assert!(v1 < v2);
            assert!(v2 < v3);
            assert!(v3 < v4);
        }

        #[test]
        fn prerelease_ordering() {
            let stable = ComposerVersion::parse("1.0.0").unwrap();
            let rc = ComposerVersion::parse("1.0.0-RC1").unwrap();
            let beta = ComposerVersion::parse("1.0.0-beta").unwrap();
            let alpha = ComposerVersion::parse("1.0.0-alpha").unwrap();

            assert!(alpha < beta);
            assert!(beta < rc);
            assert!(rc < stable);
        }

        #[test]
        fn dev_branch_ordering() {
            let dev = ComposerVersion::parse("dev-master").unwrap();
            let stable = ComposerVersion::parse("1.0.0").unwrap();

            assert!(dev < stable);
        }
    }

    mod constraint_parsing {
        use super::*;

        #[test]
        fn exact() {
            let c = ComposerConstraint::parse("1.0.0").unwrap();
            assert!(c.matches(&ComposerVersion::parse("1.0.0").unwrap()));
            assert!(!c.matches(&ComposerVersion::parse("1.0.1").unwrap()));
        }

        #[test]
        fn caret() {
            let c = ComposerConstraint::parse("^1.2.3").unwrap();
            assert!(c.matches(&ComposerVersion::parse("1.2.3").unwrap()));
            assert!(c.matches(&ComposerVersion::parse("1.9.9").unwrap()));
            assert!(!c.matches(&ComposerVersion::parse("2.0.0").unwrap()));
            assert!(!c.matches(&ComposerVersion::parse("1.2.2").unwrap()));
        }

        #[test]
        fn caret_zero_major() {
            let c = ComposerConstraint::parse("^0.2.3").unwrap();
            assert!(c.matches(&ComposerVersion::parse("0.2.3").unwrap()));
            assert!(c.matches(&ComposerVersion::parse("0.2.9").unwrap()));
            assert!(!c.matches(&ComposerVersion::parse("0.3.0").unwrap()));
        }

        #[test]
        fn tilde() {
            let c = ComposerConstraint::parse("~1.2.3").unwrap();
            assert!(c.matches(&ComposerVersion::parse("1.2.3").unwrap()));
            assert!(c.matches(&ComposerVersion::parse("1.2.9").unwrap()));
            assert!(!c.matches(&ComposerVersion::parse("1.3.0").unwrap()));
        }

        #[test]
        fn tilde_minor_only() {
            let c = ComposerConstraint::parse("~1.2").unwrap();
            assert!(c.matches(&ComposerVersion::parse("1.2.0").unwrap()));
            assert!(c.matches(&ComposerVersion::parse("1.9.0").unwrap()));
            assert!(!c.matches(&ComposerVersion::parse("2.0.0").unwrap()));
        }

        #[test]
        fn wildcard() {
            let c = ComposerConstraint::parse("1.2.*").unwrap();
            assert!(c.matches(&ComposerVersion::parse("1.2.0").unwrap()));
            assert!(c.matches(&ComposerVersion::parse("1.2.99").unwrap()));
            assert!(!c.matches(&ComposerVersion::parse("1.3.0").unwrap()));

            // Test major wildcard like 3.*
            let c3 = ComposerConstraint::parse("3.*").unwrap();
            assert!(c3.matches(&ComposerVersion::parse("3.0.0").unwrap()));
            assert!(c3.matches(&ComposerVersion::parse("3.11.0").unwrap()));
            assert!(c3.matches(&ComposerVersion::parse("3.99.99").unwrap()));
            assert!(!c3.matches(&ComposerVersion::parse("2.0.0").unwrap()));
            assert!(!c3.matches(&ComposerVersion::parse("4.0.0").unwrap()));

            // Test 7.* for elasticsearch
            let c7 = ComposerConstraint::parse("7.*").unwrap();
            assert!(c7.matches(&ComposerVersion::parse("7.0.0").unwrap()));
            assert!(c7.matches(&ComposerVersion::parse("7.17.0").unwrap()));
            assert!(!c7.matches(&ComposerVersion::parse("8.0.0").unwrap()));
        }

        #[test]
        fn range() {
            let c = ComposerConstraint::parse(">=1.0.0 <2.0.0").unwrap();
            assert!(c.matches(&ComposerVersion::parse("1.0.0").unwrap()));
            assert!(c.matches(&ComposerVersion::parse("1.9.9").unwrap()));
            assert!(!c.matches(&ComposerVersion::parse("2.0.0").unwrap()));
            assert!(!c.matches(&ComposerVersion::parse("0.9.9").unwrap()));
        }

        #[test]
        fn hyphen_range() {
            let c = ComposerConstraint::parse("1.0.0 - 2.0.0").unwrap();
            assert!(c.matches(&ComposerVersion::parse("1.0.0").unwrap()));
            assert!(c.matches(&ComposerVersion::parse("1.5.0").unwrap()));
            assert!(c.matches(&ComposerVersion::parse("2.0.0").unwrap()));
            assert!(!c.matches(&ComposerVersion::parse("2.0.1").unwrap()));
        }

        #[test]
        fn or_constraint() {
            let c = ComposerConstraint::parse("^1.0 || ^2.0").unwrap();
            assert!(c.matches(&ComposerVersion::parse("1.5.0").unwrap()));
            assert!(c.matches(&ComposerVersion::parse("2.5.0").unwrap()));
            assert!(!c.matches(&ComposerVersion::parse("3.0.0").unwrap()));
        }

        #[test]
        fn stability_flag() {
            let c = ComposerConstraint::parse(">=1.0@dev").unwrap();
            assert_eq!(c.min_stability(), Stability::Dev);
        }

        #[test]
        fn dev_branch_constraint() {
            let c = ComposerConstraint::parse("dev-master").unwrap();
            assert!(c.matches(&ComposerVersion::dev_branch("master")));
            assert!(!c.matches(&ComposerVersion::dev_branch("develop")));
        }

        #[test]
        fn not_equal() {
            let c = ComposerConstraint::parse("!=1.0.0").unwrap();
            assert!(!c.matches(&ComposerVersion::parse("1.0.0").unwrap()));
            assert!(c.matches(&ComposerVersion::parse("1.0.1").unwrap()));
            assert!(c.matches(&ComposerVersion::parse("0.9.9").unwrap()));
        }
    }

    mod constraint_operations {
        use super::*;

        #[test]
        fn intersection() {
            let c1 = ComposerConstraint::parse(">=1.0.0").unwrap();
            let c2 = ComposerConstraint::parse("<2.0.0").unwrap();
            let intersection = c1.intersection(&c2);

            assert!(intersection.matches(&ComposerVersion::parse("1.5.0").unwrap()));
            assert!(!intersection.matches(&ComposerVersion::parse("0.9.0").unwrap()));
            assert!(!intersection.matches(&ComposerVersion::parse("2.0.0").unwrap()));
        }

        #[test]
        fn union() {
            let c1 = ComposerConstraint::parse("^1.0").unwrap();
            let c2 = ComposerConstraint::parse("^3.0").unwrap();
            let union = c1.union(&c2);

            assert!(union.matches(&ComposerVersion::parse("1.5.0").unwrap()));
            assert!(union.matches(&ComposerVersion::parse("3.5.0").unwrap()));
            assert!(!union.matches(&ComposerVersion::parse("2.5.0").unwrap()));
        }
    }

    mod stability {
        use super::*;

        #[test]
        fn ordering() {
            assert!(Stability::Dev < Stability::Alpha);
            assert!(Stability::Alpha < Stability::Beta);
            assert!(Stability::Beta < Stability::RC);
            assert!(Stability::RC < Stability::Stable);
        }

        #[test]
        fn satisfies_minimum() {
            assert!(Stability::Stable.satisfies_minimum(Stability::Dev));
            assert!(Stability::Stable.satisfies_minimum(Stability::Stable));
            assert!(!Stability::Dev.satisfies_minimum(Stability::Stable));
            assert!(Stability::RC.satisfies_minimum(Stability::Beta));
        }
    }
}
