//! Deterministic lock file generation.
//!
//! Ensures same input always produces identical output for reproducible builds.

use crate::error::{LockfileError, Result};
use crate::hash::ContentHasher;
use crate::types::{
    AutoloadConfig, AutoloadPath, ComposerLock, LockedPackage, PackageAlias, PackageAuthor,
    PackageDistInfo, PackageSourceInfo, StabilityFlag,
};
use rayon::prelude::*;
use serde::ser::SerializeMap;
use serde::{Serialize, Serializer};
use std::collections::BTreeMap;
use std::io::Write;

/// Lock file generator with deterministic output.
#[derive(Debug, Clone)]
pub struct LockGenerator {
    /// Minimum stability.
    minimum_stability: String,
    /// Prefer stable flag.
    prefer_stable: bool,
    /// Prefer lowest flag.
    prefer_lowest: bool,
    /// Platform requirements.
    platform: BTreeMap<String, String>,
    /// Dev platform requirements.
    platform_dev: BTreeMap<String, String>,
    /// Plugin API version.
    plugin_api_version: String,
    /// Production packages.
    packages: Vec<LockedPackage>,
    /// Dev packages.
    packages_dev: Vec<LockedPackage>,
    /// Package aliases.
    aliases: Vec<PackageAlias>,
    /// Stability flags per package.
    stability_flags: BTreeMap<String, u8>,
}

impl Default for LockGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl LockGenerator {
    /// Create a new lock generator.
    #[must_use]
    pub fn new() -> Self {
        Self {
            minimum_stability: "stable".to_string(),
            prefer_stable: false,
            prefer_lowest: false,
            platform: BTreeMap::new(),
            platform_dev: BTreeMap::new(),
            plugin_api_version: "2.6.0".to_string(),
            packages: Vec::new(),
            packages_dev: Vec::new(),
            aliases: Vec::new(),
            stability_flags: BTreeMap::new(),
        }
    }

    /// Set minimum stability.
    pub fn minimum_stability(&mut self, stability: impl Into<String>) -> &mut Self {
        self.minimum_stability = stability.into();
        self
    }

    /// Set prefer stable flag.
    pub fn prefer_stable(&mut self, prefer: bool) -> &mut Self {
        self.prefer_stable = prefer;
        self
    }

    /// Set prefer lowest flag.
    pub fn prefer_lowest(&mut self, prefer: bool) -> &mut Self {
        self.prefer_lowest = prefer;
        self
    }

    /// Set plugin API version.
    pub fn plugin_api_version(&mut self, version: impl Into<String>) -> &mut Self {
        self.plugin_api_version = version.into();
        self
    }

    /// Add platform requirement.
    pub fn add_platform(
        &mut self,
        name: impl Into<String>,
        version: impl Into<String>,
    ) -> &mut Self {
        self.platform.insert(name.into(), version.into());
        self
    }

    /// Add dev platform requirement.
    pub fn add_platform_dev(
        &mut self,
        name: impl Into<String>,
        version: impl Into<String>,
    ) -> &mut Self {
        self.platform_dev.insert(name.into(), version.into());
        self
    }

    /// Add a production package.
    pub fn add_package(&mut self, package: LockedPackage) -> &mut Self {
        self.packages.push(package);
        self
    }

    /// Add a dev package.
    pub fn add_package_dev(&mut self, package: LockedPackage) -> &mut Self {
        self.packages_dev.push(package);
        self
    }

    /// Add multiple production packages.
    pub fn add_packages(&mut self, packages: impl IntoIterator<Item = LockedPackage>) -> &mut Self {
        self.packages.extend(packages);
        self
    }

    /// Add multiple dev packages.
    pub fn add_packages_dev(
        &mut self,
        packages: impl IntoIterator<Item = LockedPackage>,
    ) -> &mut Self {
        self.packages_dev.extend(packages);
        self
    }

    /// Add package alias.
    pub fn add_alias(&mut self, alias: PackageAlias) -> &mut Self {
        self.aliases.push(alias);
        self
    }

    /// Set stability flag for a package.
    pub fn set_stability_flag(
        &mut self,
        package: impl Into<String>,
        flag: StabilityFlag,
    ) -> &mut Self {
        self.stability_flags.insert(package.into(), flag.as_u8());
        self
    }

    /// Generate the lock file with content hash.
    ///
    /// # Arguments
    /// * `require` - Production dependencies from composer.json
    /// * `require_dev` - Dev dependencies from composer.json
    #[must_use]
    pub fn generate(
        mut self,
        require: &BTreeMap<String, String>,
        require_dev: &BTreeMap<String, String>,
    ) -> ComposerLock {
        // Sort packages deterministically
        self.packages.par_sort();
        self.packages_dev.par_sort();
        self.aliases.sort_by(|a, b| a.package.cmp(&b.package));

        // Compute content hash
        let content_hash = ContentHasher::compute_content_hash(
            require,
            require_dev,
            Some(&self.minimum_stability),
            Some(self.prefer_stable),
            if self.prefer_lowest { Some(true) } else { None },
            &self.platform,
            &BTreeMap::new(), // platform overrides
        );

        ComposerLock {
            readme: vec![
                "This file locks the dependencies of your project to a known state".to_string(),
                "Read more about it at https://getcomposer.org/doc/01-basic-usage.md#installing-dependencies".to_string(),
                "This file is @generated automatically".to_string(),
            ],
            content_hash,
            packages: self.packages,
            packages_dev: self.packages_dev,
            aliases: self.aliases,
            minimum_stability: self.minimum_stability,
            stability_flags: self.stability_flags,
            prefer_stable: self.prefer_stable,
            prefer_lowest: self.prefer_lowest,
            platform: self.platform,
            platform_dev: self.platform_dev,
            plugin_api_version: self.plugin_api_version,
        }
    }
}

/// Deterministic JSON serializer for lock files.
///
/// Ensures:
/// - Alphabetically sorted keys
/// - 4-space indentation
/// - Consistent formatting
/// - No trailing whitespace
#[derive(Debug, Clone, Copy, Default)]
pub struct DeterministicSerializer;

impl DeterministicSerializer {
    /// Serialize lock file to deterministic JSON.
    ///
    /// # Errors
    /// Returns error if serialization fails.
    pub fn serialize(lock: &ComposerLock) -> Result<String> {
        // Use a custom serializer for deterministic output
        let mut output = Vec::with_capacity(64 * 1024); // Pre-allocate 64KB
        Self::write_lock(&mut output, lock)?;
        String::from_utf8(output).map_err(|e| LockfileError::Serialization(e.to_string()))
    }

    /// Write lock file to a writer.
    fn write_lock<W: Write>(w: &mut W, lock: &ComposerLock) -> Result<()> {
        writeln!(w, "{{").map_err(|e| LockfileError::Serialization(e.to_string()))?;

        // _readme
        write!(w, "    \"_readme\": [").map_err(|e| LockfileError::Serialization(e.to_string()))?;
        for (i, line) in lock.readme.iter().enumerate() {
            if i > 0 {
                write!(w, ",").map_err(|e| LockfileError::Serialization(e.to_string()))?;
            }
            writeln!(w).map_err(|e| LockfileError::Serialization(e.to_string()))?;
            write!(w, "        \"{}\"", escape_json(line))
                .map_err(|e| LockfileError::Serialization(e.to_string()))?;
        }
        writeln!(w, "\n    ],").map_err(|e| LockfileError::Serialization(e.to_string()))?;

        // content-hash
        writeln!(w, "    \"content-hash\": \"{}\",", lock.content_hash)
            .map_err(|e| LockfileError::Serialization(e.to_string()))?;

        // packages
        Self::write_packages(w, "packages", &lock.packages)?;
        writeln!(w, ",").map_err(|e| LockfileError::Serialization(e.to_string()))?;

        // packages-dev
        Self::write_packages(w, "packages-dev", &lock.packages_dev)?;
        writeln!(w, ",").map_err(|e| LockfileError::Serialization(e.to_string()))?;

        // aliases
        write!(w, "    \"aliases\": ").map_err(|e| LockfileError::Serialization(e.to_string()))?;
        if lock.aliases.is_empty() {
            write!(w, "[]").map_err(|e| LockfileError::Serialization(e.to_string()))?;
        } else {
            Self::write_aliases(w, &lock.aliases)?;
        }
        writeln!(w, ",").map_err(|e| LockfileError::Serialization(e.to_string()))?;

        // minimum-stability
        writeln!(
            w,
            "    \"minimum-stability\": \"{}\",",
            escape_json(&lock.minimum_stability)
        )
        .map_err(|e| LockfileError::Serialization(e.to_string()))?;

        // stability-flags
        write!(w, "    \"stability-flags\": ")
            .map_err(|e| LockfileError::Serialization(e.to_string()))?;
        Self::write_btree_u8(w, &lock.stability_flags)?;
        writeln!(w, ",").map_err(|e| LockfileError::Serialization(e.to_string()))?;

        // prefer-stable
        writeln!(w, "    \"prefer-stable\": {},", lock.prefer_stable)
            .map_err(|e| LockfileError::Serialization(e.to_string()))?;

        // prefer-lowest
        writeln!(w, "    \"prefer-lowest\": {},", lock.prefer_lowest)
            .map_err(|e| LockfileError::Serialization(e.to_string()))?;

        // platform
        write!(w, "    \"platform\": ").map_err(|e| LockfileError::Serialization(e.to_string()))?;
        Self::write_btree_string(w, &lock.platform)?;
        writeln!(w, ",").map_err(|e| LockfileError::Serialization(e.to_string()))?;

        // platform-dev
        write!(w, "    \"platform-dev\": ")
            .map_err(|e| LockfileError::Serialization(e.to_string()))?;
        Self::write_btree_string(w, &lock.platform_dev)?;
        writeln!(w, ",").map_err(|e| LockfileError::Serialization(e.to_string()))?;

        // plugin-api-version
        writeln!(
            w,
            "    \"plugin-api-version\": \"{}\"",
            escape_json(&lock.plugin_api_version)
        )
        .map_err(|e| LockfileError::Serialization(e.to_string()))?;

        writeln!(w, "}}").map_err(|e| LockfileError::Serialization(e.to_string()))?;

        Ok(())
    }

    /// Write packages array.
    fn write_packages<W: Write>(w: &mut W, name: &str, packages: &[LockedPackage]) -> Result<()> {
        write!(w, "    \"{}\": ", name).map_err(|e| LockfileError::Serialization(e.to_string()))?;

        if packages.is_empty() {
            write!(w, "[]").map_err(|e| LockfileError::Serialization(e.to_string()))?;
            return Ok(());
        }

        writeln!(w, "[").map_err(|e| LockfileError::Serialization(e.to_string()))?;

        for (i, pkg) in packages.iter().enumerate() {
            Self::write_package(w, pkg, 8)?;
            if i < packages.len() - 1 {
                writeln!(w, ",").map_err(|e| LockfileError::Serialization(e.to_string()))?;
            } else {
                writeln!(w).map_err(|e| LockfileError::Serialization(e.to_string()))?;
            }
        }

        write!(w, "    ]").map_err(|e| LockfileError::Serialization(e.to_string()))?;
        Ok(())
    }

    /// Write a single package.
    fn write_package<W: Write>(w: &mut W, pkg: &LockedPackage, indent: usize) -> Result<()> {
        let prefix = " ".repeat(indent);
        let inner = " ".repeat(indent + 4);

        writeln!(w, "{}{{", prefix).map_err(|e| LockfileError::Serialization(e.to_string()))?;

        // Collect all fields including required ones in sorted order
        let mut fields: Vec<(&str, String)> = Vec::new();

        // Required fields
        fields.push(("name", format!("\"{}\"", escape_json(&pkg.name))));
        fields.push(("version", format!("\"{}\"", escape_json(&pkg.version))));

        // source
        if let Some(ref source) = pkg.source {
            fields.push(("source", Self::format_source(source)));
        }

        // dist
        if let Some(ref dist) = pkg.dist {
            fields.push(("dist", Self::format_dist(dist)));
        }

        // require
        if !pkg.require.is_empty() {
            fields.push(("require", Self::format_btree(&pkg.require)));
        }

        // require-dev
        if !pkg.require_dev.is_empty() {
            fields.push(("require-dev", Self::format_btree(&pkg.require_dev)));
        }

        // type
        if let Some(ref t) = pkg.package_type {
            fields.push(("type", format!("\"{}\"", escape_json(t))));
        }

        // autoload
        if let Some(ref autoload) = pkg.autoload {
            if !is_autoload_empty(autoload) {
                fields.push(("autoload", Self::format_autoload(autoload)));
            }
        }

        // notification-url
        if let Some(ref url) = pkg.notification_url {
            fields.push(("notification-url", format!("\"{}\"", escape_json(url))));
        }

        // license
        if !pkg.license.is_empty() {
            fields.push(("license", Self::format_string_array(&pkg.license)));
        }

        // authors
        if !pkg.authors.is_empty() {
            fields.push(("authors", Self::format_authors(&pkg.authors)));
        }

        // description
        if let Some(ref desc) = pkg.description {
            fields.push(("description", format!("\"{}\"", escape_json(desc))));
        }

        // homepage
        if let Some(ref hp) = pkg.homepage {
            fields.push(("homepage", format!("\"{}\"", escape_json(hp))));
        }

        // keywords
        if !pkg.keywords.is_empty() {
            fields.push(("keywords", Self::format_string_array(&pkg.keywords)));
        }

        // time
        if let Some(ref time) = pkg.time {
            fields.push(("time", format!("\"{}\"", escape_json(time))));
        }

        // Sort and write fields
        fields.sort_by(|a, b| a.0.cmp(b.0));

        for (i, (name, value)) in fields.iter().enumerate() {
            let trailing = if i < fields.len() - 1 { "," } else { "" };
            writeln!(w, "{}\"{}\": {}{}", inner, name, value, trailing)
                .map_err(|e| LockfileError::Serialization(e.to_string()))?;
        }

        write!(w, "{}}}", prefix).map_err(|e| LockfileError::Serialization(e.to_string()))?;
        Ok(())
    }

    fn format_source(source: &PackageSourceInfo) -> String {
        format!(
            "{{ \"type\": \"{}\", \"url\": \"{}\", \"reference\": \"{}\" }}",
            escape_json(&source.source_type),
            escape_json(&source.url),
            escape_json(&source.reference)
        )
    }

    fn format_dist(dist: &PackageDistInfo) -> String {
        let mut parts = vec![
            format!("\"type\": \"{}\"", escape_json(&dist.dist_type)),
            format!("\"url\": \"{}\"", escape_json(&dist.url)),
        ];
        if let Some(ref r) = dist.reference {
            parts.push(format!("\"reference\": \"{}\"", escape_json(r)));
        }
        if let Some(ref s) = dist.shasum {
            parts.push(format!("\"shasum\": \"{}\"", escape_json(s)));
        }
        format!("{{ {} }}", parts.join(", "))
    }

    fn format_btree(map: &BTreeMap<String, String>) -> String {
        if map.is_empty() {
            return "{}".to_string();
        }
        let pairs: Vec<String> = map
            .iter()
            .map(|(k, v)| format!("\"{}\": \"{}\"", escape_json(k), escape_json(v)))
            .collect();
        format!("{{ {} }}", pairs.join(", "))
    }

    fn format_string_array(arr: &[String]) -> String {
        let items: Vec<String> = arr
            .iter()
            .map(|s| format!("\"{}\"", escape_json(s)))
            .collect();
        format!("[{}]", items.join(", "))
    }

    fn format_authors(authors: &[PackageAuthor]) -> String {
        let items: Vec<String> = authors
            .iter()
            .map(|a| {
                let mut parts = vec![format!("\"name\": \"{}\"", escape_json(&a.name))];
                if let Some(ref email) = a.email {
                    parts.push(format!("\"email\": \"{}\"", escape_json(email)));
                }
                if let Some(ref hp) = a.homepage {
                    parts.push(format!("\"homepage\": \"{}\"", escape_json(hp)));
                }
                if let Some(ref role) = a.role {
                    parts.push(format!("\"role\": \"{}\"", escape_json(role)));
                }
                format!("{{ {} }}", parts.join(", "))
            })
            .collect();
        format!("[{}]", items.join(", "))
    }

    fn format_autoload(autoload: &AutoloadConfig) -> String {
        let mut parts = Vec::new();

        if !autoload.psr4.is_empty() {
            let psr4: Vec<String> = autoload
                .psr4
                .iter()
                .map(|(k, v)| {
                    let value = match v {
                        AutoloadPath::Single(s) => format!("\"{}\"", escape_json(s)),
                        AutoloadPath::Multiple(arr) => {
                            let items: Vec<String> = arr
                                .iter()
                                .map(|s| format!("\"{}\"", escape_json(s)))
                                .collect();
                            format!("[{}]", items.join(", "))
                        }
                    };
                    format!("\"{}\": {}", escape_json(k), value)
                })
                .collect();
            parts.push(format!("\"psr-4\": {{ {} }}", psr4.join(", ")));
        }

        if !autoload.psr0.is_empty() {
            let psr0: Vec<String> = autoload
                .psr0
                .iter()
                .map(|(k, v)| {
                    let value = match v {
                        AutoloadPath::Single(s) => format!("\"{}\"", escape_json(s)),
                        AutoloadPath::Multiple(arr) => {
                            let items: Vec<String> = arr
                                .iter()
                                .map(|s| format!("\"{}\"", escape_json(s)))
                                .collect();
                            format!("[{}]", items.join(", "))
                        }
                    };
                    format!("\"{}\": {}", escape_json(k), value)
                })
                .collect();
            parts.push(format!("\"psr-0\": {{ {} }}", psr0.join(", ")));
        }

        if !autoload.classmap.is_empty() {
            let items: Vec<String> = autoload
                .classmap
                .iter()
                .map(|s| format!("\"{}\"", escape_json(s)))
                .collect();
            parts.push(format!("\"classmap\": [{}]", items.join(", ")));
        }

        if !autoload.files.is_empty() {
            let items: Vec<String> = autoload
                .files
                .iter()
                .map(|s| format!("\"{}\"", escape_json(s)))
                .collect();
            parts.push(format!("\"files\": [{}]", items.join(", ")));
        }

        format!("{{ {} }}", parts.join(", "))
    }

    fn write_aliases<W: Write>(w: &mut W, aliases: &[PackageAlias]) -> Result<()> {
        writeln!(w, "[").map_err(|e| LockfileError::Serialization(e.to_string()))?;
        for (i, alias) in aliases.iter().enumerate() {
            let trailing = if i < aliases.len() - 1 { "," } else { "" };
            writeln!(
                w,
                "        {{ \"package\": \"{}\", \"version\": \"{}\", \"alias\": \"{}\", \"alias_normalized\": \"{}\" }}{}",
                escape_json(&alias.package),
                escape_json(&alias.version),
                escape_json(&alias.alias),
                escape_json(&alias.alias_normalized),
                trailing
            )
            .map_err(|e| LockfileError::Serialization(e.to_string()))?;
        }
        write!(w, "    ]").map_err(|e| LockfileError::Serialization(e.to_string()))?;
        Ok(())
    }

    fn write_btree_string<W: Write>(w: &mut W, map: &BTreeMap<String, String>) -> Result<()> {
        if map.is_empty() {
            write!(w, "{{}}").map_err(|e| LockfileError::Serialization(e.to_string()))?;
            return Ok(());
        }
        write!(w, "{{ ").map_err(|e| LockfileError::Serialization(e.to_string()))?;
        let pairs: Vec<String> = map
            .iter()
            .map(|(k, v)| format!("\"{}\": \"{}\"", escape_json(k), escape_json(v)))
            .collect();
        write!(w, "{}", pairs.join(", "))
            .map_err(|e| LockfileError::Serialization(e.to_string()))?;
        write!(w, " }}").map_err(|e| LockfileError::Serialization(e.to_string()))?;
        Ok(())
    }

    fn write_btree_u8<W: Write>(w: &mut W, map: &BTreeMap<String, u8>) -> Result<()> {
        if map.is_empty() {
            write!(w, "{{}}").map_err(|e| LockfileError::Serialization(e.to_string()))?;
            return Ok(());
        }
        write!(w, "{{ ").map_err(|e| LockfileError::Serialization(e.to_string()))?;
        let pairs: Vec<String> = map
            .iter()
            .map(|(k, v)| format!("\"{}\": {}", escape_json(k), v))
            .collect();
        write!(w, "{}", pairs.join(", "))
            .map_err(|e| LockfileError::Serialization(e.to_string()))?;
        write!(w, " }}").map_err(|e| LockfileError::Serialization(e.to_string()))?;
        Ok(())
    }
}

/// Check if autoload config is empty.
fn is_autoload_empty(autoload: &AutoloadConfig) -> bool {
    autoload.psr4.is_empty()
        && autoload.psr0.is_empty()
        && autoload.classmap.is_empty()
        && autoload.files.is_empty()
}

/// Escape special characters in JSON strings.
fn escape_json(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            c if c.is_control() => {
                result.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => result.push(c),
        }
    }
    result
}

/// Wrapper for deterministic serialization via serde.
#[derive(Debug)]
pub struct DeterministicLock<'a>(pub &'a ComposerLock);

impl<'a> Serialize for DeterministicLock<'a> {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Custom serialization with sorted keys
        let lock = self.0;
        let mut map = serializer.serialize_map(Some(12))?;

        map.serialize_entry("_readme", &lock.readme)?;
        map.serialize_entry("content-hash", &lock.content_hash)?;
        map.serialize_entry("packages", &lock.packages)?;
        map.serialize_entry("packages-dev", &lock.packages_dev)?;
        map.serialize_entry("aliases", &lock.aliases)?;
        map.serialize_entry("minimum-stability", &lock.minimum_stability)?;
        map.serialize_entry("stability-flags", &lock.stability_flags)?;
        map.serialize_entry("prefer-stable", &lock.prefer_stable)?;
        map.serialize_entry("prefer-lowest", &lock.prefer_lowest)?;
        map.serialize_entry("platform", &lock.platform)?;
        map.serialize_entry("platform-dev", &lock.platform_dev)?;
        map.serialize_entry("plugin-api-version", &lock.plugin_api_version)?;

        map.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generator_basic() {
        let mut generator = LockGenerator::new();
        generator
            .minimum_stability("stable")
            .prefer_stable(true)
            .add_package(LockedPackage::new("vendor/pkg", "1.0.0"));

        let require = BTreeMap::new();
        let require_dev = BTreeMap::new();
        let lock = generator.generate(&require, &require_dev);

        assert_eq!(lock.packages.len(), 1);
        assert_eq!(lock.minimum_stability, "stable");
        assert!(lock.prefer_stable);
    }

    #[test]
    fn test_deterministic_output() {
        let mut generator = LockGenerator::new();
        generator
            .add_package(LockedPackage::new("b/pkg", "1.0.0"))
            .add_package(LockedPackage::new("a/pkg", "2.0.0"));

        let require = BTreeMap::new();
        let lock = generator.generate(&require, &BTreeMap::new());

        // Packages should be sorted
        assert_eq!(lock.packages[0].name, "a/pkg");
        assert_eq!(lock.packages[1].name, "b/pkg");
    }

    #[test]
    fn test_content_hash_deterministic() {
        let mut require = BTreeMap::new();
        require.insert("psr/log".to_string(), "^3.0".to_string());

        let lock1 = LockGenerator::new().generate(&require, &BTreeMap::new());
        let lock2 = LockGenerator::new().generate(&require, &BTreeMap::new());

        assert_eq!(lock1.content_hash, lock2.content_hash);
    }

    #[test]
    fn test_serializer() {
        let lock = LockGenerator::new().generate(&BTreeMap::new(), &BTreeMap::new());
        let json = DeterministicSerializer::serialize(&lock).unwrap();

        assert!(json.contains("\"content-hash\""));
        assert!(json.contains("\"packages\""));
        assert!(json.contains("\"_readme\""));
    }

    #[test]
    fn test_escape_json() {
        assert_eq!(escape_json("hello"), "hello");
        assert_eq!(escape_json("hello\"world"), "hello\\\"world");
        assert_eq!(escape_json("line1\nline2"), "line1\\nline2");
        assert_eq!(escape_json("path\\to\\file"), "path\\\\to\\\\file");
    }
}
