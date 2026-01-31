//! Ultra-fast PHP autoloader generation for Libretto.
//!
//! Features:
//! - tree-sitter-php for accurate parsing of classes, interfaces, traits, enums
//! - Parallel file scanning with rayon/walkdir
//! - PSR-4, PSR-0, classmap, and files autoloader types
//! - Optimization levels (0, 1, 2) for different performance profiles
//! - Incremental updates with mtime tracking and blake3 checksums
//! - rkyv caching for fast startup

#![warn(clippy::all)]
#![allow(clippy::module_name_repetitions)]

mod parser;
mod scanner;

pub use parser::{DefinitionKind, PhpDefinition, PhpParser};
pub use scanner::{ExcludePattern, FileScanResult, Scanner, build_classmap, build_namespace_map};

use ahash::AHashMap;
use libretto_core::{Error, Result};
use parking_lot::RwLock;
use rkyv::{Archive, Deserialize, Serialize};
use serde::{Deserialize as SerdeDeserialize, Serialize as SerdeSerialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;
use tracing::{debug, info, warn};

/// Optimization level for autoloader generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum OptimizationLevel {
    /// Level 0: No optimization, PSR-4/PSR-0 runtime resolution.
    #[default]
    None = 0,
    /// Level 1 (-o): Generate authoritative classmap from PSR-4/classmap directories.
    Optimized = 1,
    /// Level 2 (-a): Authoritative classmap, assume all classes are in classmap.
    Authoritative = 2,
}

impl OptimizationLevel {
    /// Create from integer value.
    #[must_use]
    pub const fn from_int(level: u8) -> Self {
        match level {
            0 => Self::None,
            1 => Self::Optimized,
            _ => Self::Authoritative,
        }
    }
}

/// PSR-4 autoload configuration.
#[derive(Debug, Clone, Default, SerdeSerialize, SerdeDeserialize)]
pub struct Psr4Config {
    /// Namespace to directory mappings.
    #[serde(flatten)]
    pub mappings: HashMap<String, Vec<String>>,
}

/// PSR-0 autoload configuration.
#[derive(Debug, Clone, Default, SerdeSerialize, SerdeDeserialize)]
pub struct Psr0Config {
    /// Namespace to directory mappings.
    #[serde(flatten)]
    pub mappings: HashMap<String, Vec<String>>,
}

/// Classmap configuration.
#[derive(Debug, Clone, Default, SerdeSerialize, SerdeDeserialize)]
#[serde(transparent)]
pub struct ClassmapConfig {
    /// Directories/files to scan.
    pub paths: Vec<String>,
}

/// Files to always include.
#[derive(Debug, Clone, Default, SerdeSerialize, SerdeDeserialize)]
#[serde(transparent)]
pub struct FilesConfig {
    /// Files to include.
    pub files: Vec<String>,
}

/// Exclude patterns configuration.
#[derive(Debug, Clone, Default, SerdeSerialize, SerdeDeserialize)]
#[serde(transparent)]
pub struct ExcludeConfig {
    /// Patterns to exclude from scanning.
    pub patterns: Vec<String>,
}

/// Complete autoload configuration.
#[derive(Debug, Clone, Default, SerdeSerialize, SerdeDeserialize)]
pub struct AutoloadConfig {
    /// PSR-4 autoloading.
    #[serde(default, rename = "psr-4")]
    pub psr4: Psr4Config,
    /// PSR-0 autoloading.
    #[serde(default, rename = "psr-0")]
    pub psr0: Psr0Config,
    /// Classmap autoloading.
    #[serde(default)]
    pub classmap: ClassmapConfig,
    /// Files to include.
    #[serde(default)]
    pub files: FilesConfig,
    /// Patterns to exclude from classmap generation.
    #[serde(default, rename = "exclude-from-classmap")]
    pub exclude: ExcludeConfig,
}

/// Cached file entry for incremental updates.
#[derive(Debug, Clone, Archive, Deserialize, Serialize)]
pub struct CachedFileEntry {
    /// File path (relative to vendor).
    pub path: String,
    /// Modification time (unix timestamp).
    pub mtime: u64,
    /// Semantic fingerprint for change detection.
    ///
    /// This is a position-insensitive AST fingerprint that ignores whitespace
    /// and formatting changes. Two files with identical semantics will have
    /// the same fingerprint even if formatted differently.
    pub fingerprint: u64,
    /// Extracted class names.
    pub classes: Vec<String>,
}

/// Cached classmap for fast loading.
#[derive(Debug, Clone, Archive, Deserialize, Serialize)]
pub struct CachedClassmap {
    /// Version for cache invalidation.
    pub version: u32,
    /// File entries with metadata.
    pub files: Vec<CachedFileEntry>,
    /// Class to file path mapping.
    pub classmap: Vec<(String, String)>,
}

impl CachedClassmap {
    /// Current cache format version.
    pub const VERSION: u32 = 1;

    /// Create new empty cache.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            version: Self::VERSION,
            files: Vec::new(),
            classmap: Vec::new(),
        }
    }
}

impl Default for CachedClassmap {
    fn default() -> Self {
        Self::new()
    }
}

/// Incremental cache for tracking file changes.
#[derive(Debug)]
pub struct IncrementalCache {
    /// Path to cache file.
    cache_path: PathBuf,
    /// Cached data.
    data: RwLock<CachedClassmap>,
}

impl IncrementalCache {
    /// Magic bytes for rkyv cache.
    const MAGIC: &'static [u8; 8] = b"LBRTAUTL";

    /// Create or load incremental cache.
    #[must_use]
    pub fn load_or_create(cache_path: PathBuf) -> Self {
        let data = if cache_path.exists() {
            match std::fs::read(&cache_path) {
                Ok(bytes) if bytes.len() > 8 && &bytes[..8] == Self::MAGIC => {
                    match rkyv::from_bytes::<CachedClassmap, rkyv::rancor::Error>(&bytes[8..]) {
                        Ok(cached) if cached.version == CachedClassmap::VERSION => {
                            debug!("Loaded autoloader cache from {:?}", cache_path);
                            cached
                        }
                        Ok(_) => {
                            debug!("Cache version mismatch, rebuilding");
                            CachedClassmap::new()
                        }
                        Err(e) => {
                            warn!("Failed to load cache: {}, rebuilding", e);
                            CachedClassmap::new()
                        }
                    }
                }
                _ => CachedClassmap::new(),
            }
        } else {
            CachedClassmap::new()
        };

        Self {
            cache_path,
            data: RwLock::new(data),
        }
    }

    /// Check which files have changed since last scan.
    pub fn find_changed_files(&self, files: &[PathBuf]) -> Vec<PathBuf> {
        let data = self.data.read();
        let file_map: AHashMap<&str, &CachedFileEntry> =
            data.files.iter().map(|f| (f.path.as_str(), f)).collect();

        let mut changed = Vec::new();
        for path in files {
            let path_str = path.to_string_lossy();
            match file_map.get(path_str.as_ref()) {
                Some(cached) => {
                    // Check mtime
                    if let Ok(meta) = std::fs::metadata(path) {
                        let mtime = meta
                            .modified()
                            .ok()
                            .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                            .map_or(0, |d| d.as_secs());

                        if mtime != cached.mtime {
                            changed.push(path.clone());
                        }
                    } else {
                        changed.push(path.clone());
                    }
                }
                None => {
                    changed.push(path.clone());
                }
            }
        }

        changed
    }

    /// Update cache with new scan results.
    pub fn update(&self, results: &[FileScanResult], base_path: &Path) {
        let mut data = self.data.write();

        // Build index of existing files
        let mut file_index: AHashMap<String, usize> = data
            .files
            .iter()
            .enumerate()
            .map(|(i, f)| (f.path.clone(), i))
            .collect();

        // Update or add entries
        for result in results {
            let rel_path = result
                .path
                .strip_prefix(base_path)
                .unwrap_or(&result.path)
                .to_string_lossy()
                .to_string();

            let classes: Vec<String> = result.definitions.iter().map(|d| d.fqcn.clone()).collect();

            let entry = CachedFileEntry {
                path: rel_path.clone(),
                mtime: result.mtime,
                fingerprint: result.fingerprint,
                classes,
            };

            if let Some(&idx) = file_index.get(&rel_path) {
                data.files[idx] = entry;
            } else {
                file_index.insert(rel_path, data.files.len());
                data.files.push(entry);
            }
        }

        // Rebuild classmap from files
        let new_classmap: Vec<(String, String)> = data
            .files
            .iter()
            .flat_map(|file| {
                file.classes
                    .iter()
                    .map(move |class| (class.clone(), file.path.clone()))
            })
            .collect();
        data.classmap = new_classmap;
    }

    /// Save cache to disk.
    pub fn save(&self) -> std::io::Result<()> {
        let data = self.data.read();

        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&*data)
            .map_err(|e| std::io::Error::other(e.to_string()))?;

        let mut output = Vec::with_capacity(8 + bytes.len());
        output.extend_from_slice(Self::MAGIC);
        output.extend_from_slice(&bytes);

        if let Some(parent) = self.cache_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        std::fs::write(&self.cache_path, output)?;
        debug!("Saved autoloader cache to {:?}", self.cache_path);
        Ok(())
    }

    /// Get current classmap.
    #[must_use]
    pub fn get_classmap(&self) -> Vec<(String, String)> {
        self.data.read().classmap.clone()
    }

    /// Clear cache.
    pub fn clear(&self) {
        *self.data.write() = CachedClassmap::new();
        let _ = std::fs::remove_file(&self.cache_path);
    }
}

/// Autoloader generator with tree-sitter parsing and incremental updates.
#[derive(Debug)]
pub struct AutoloaderGenerator {
    /// Vendor directory path.
    vendor_dir: PathBuf,
    /// Optimization level.
    optimization_level: OptimizationLevel,
    /// Generated classmap (class -> path).
    classmap: AHashMap<String, PathBuf>,
    /// PSR-4 namespace mappings.
    psr4_map: HashMap<String, Vec<PathBuf>>,
    /// PSR-0 namespace mappings.
    psr0_map: HashMap<String, Vec<PathBuf>>,
    /// Files to always include.
    files: Vec<PathBuf>,
    /// Incremental cache.
    cache: Option<Arc<IncrementalCache>>,
    /// Scanner for PHP files.
    scanner: Scanner,
}

impl AutoloaderGenerator {
    /// Create new generator.
    #[must_use]
    pub fn new(vendor_dir: PathBuf) -> Self {
        Self {
            vendor_dir,
            optimization_level: OptimizationLevel::None,
            classmap: AHashMap::new(),
            psr4_map: HashMap::new(),
            psr0_map: HashMap::new(),
            files: Vec::new(),
            cache: None,
            scanner: Scanner::without_exclusions(),
        }
    }

    /// Create generator with optimization level.
    #[must_use]
    pub fn with_optimization(vendor_dir: PathBuf, level: OptimizationLevel) -> Self {
        let mut generator = Self::new(vendor_dir);
        generator.optimization_level = level;
        generator
    }

    /// Enable incremental caching.
    #[must_use]
    pub fn with_cache(mut self, cache_path: PathBuf) -> Self {
        self.cache = Some(Arc::new(IncrementalCache::load_or_create(cache_path)));
        self
    }

    /// Set exclude patterns for scanning.
    #[must_use]
    pub fn with_exclude_patterns(mut self, patterns: &[String]) -> Self {
        self.scanner = Scanner::new(ExcludePattern::from_patterns(patterns));
        self
    }

    /// Add autoload configuration from a package.
    pub fn add_package(&mut self, package_dir: &Path, config: &AutoloadConfig) {
        // Add PSR-4 mappings
        for (namespace, dirs) in &config.psr4.mappings {
            let paths: Vec<PathBuf> = dirs.iter().map(|d| package_dir.join(d)).collect();
            self.psr4_map
                .entry(namespace.clone())
                .or_default()
                .extend(paths);
        }

        // Add PSR-0 mappings
        for (namespace, dirs) in &config.psr0.mappings {
            let paths: Vec<PathBuf> = dirs.iter().map(|d| package_dir.join(d)).collect();
            self.psr0_map
                .entry(namespace.clone())
                .or_default()
                .extend(paths);
        }

        // Scan classmap directories based on optimization level
        if self.optimization_level >= OptimizationLevel::Optimized {
            // Collect paths first to avoid borrow issues
            let psr4_paths: Vec<PathBuf> = self
                .psr4_map
                .values()
                .flatten()
                .filter(|p| p.exists())
                .cloned()
                .collect();

            let psr0_paths: Vec<PathBuf> = self
                .psr0_map
                .values()
                .flatten()
                .filter(|p| p.exists())
                .cloned()
                .collect();

            // Scan PSR-4 directories for classes
            for path in psr4_paths {
                self.scan_directory_for_classes(&path);
            }

            // Scan PSR-0 directories
            for path in psr0_paths {
                self.scan_directory_for_classes(&path);
            }
        }

        // Always scan explicit classmap paths
        for path in &config.classmap.paths {
            let full_path = package_dir.join(path);
            if full_path.exists() {
                self.scan_directory_for_classes(&full_path);
            }
        }

        // Add files
        for file in &config.files.files {
            let full_path = package_dir.join(file);
            if full_path.exists() {
                self.files.push(full_path);
            }
        }
    }

    /// Scan directory for classes using tree-sitter parser.
    fn scan_directory_for_classes(&mut self, path: &Path) {
        let results = if let Some(ref cache) = self.cache {
            // Incremental scan - only changed files
            let all_files: Vec<PathBuf> = self
                .scanner
                .scan_directory(path)
                .into_iter()
                .map(|r| r.path)
                .collect();

            let changed_files = cache.find_changed_files(&all_files);

            if changed_files.is_empty() {
                debug!("No files changed in {:?}", path);
                // Load from cache
                for (class, file_path) in cache.get_classmap() {
                    let full_path = self.vendor_dir.join(&file_path);
                    self.classmap.insert(class, full_path);
                }
                return;
            }

            debug!("{} files changed in {:?}", changed_files.len(), path);

            // Scan only changed files
            let results: Vec<FileScanResult> = changed_files
                .iter()
                .filter_map(|p| self.scanner.scan_file(p))
                .collect();

            // Update cache
            cache.update(&results, &self.vendor_dir);

            results
        } else {
            // Full scan
            self.scanner.scan_directory(path)
        };

        // Add to classmap
        for result in &results {
            for def in &result.definitions {
                self.classmap.insert(def.fqcn.clone(), result.path.clone());
            }
        }
    }

    /// Generate all autoloader files.
    ///
    /// # Errors
    /// Returns error if generation fails.
    pub fn generate(&self) -> Result<()> {
        let autoload_dir = self.vendor_dir.join("composer");
        std::fs::create_dir_all(&autoload_dir).map_err(|e| Error::io(&autoload_dir, e))?;

        // Generate all PHP files
        self.generate_classloader(&autoload_dir)?;
        self.generate_autoload_real(&autoload_dir)?;
        self.generate_autoload_static(&autoload_dir)?;
        self.generate_autoload_psr4(&autoload_dir)?;
        self.generate_autoload_classmap(&autoload_dir)?;
        self.generate_autoload_files(&autoload_dir)?;
        self.generate_autoload_namespaces(&autoload_dir)?;
        self.generate_autoload(&self.vendor_dir)?;

        // Save cache if enabled
        if let Some(Err(e)) = self.cache.as_ref().map(|c| c.save()) {
            warn!("Failed to save autoloader cache: {}", e);
        }

        info!(
            optimization_level = ?self.optimization_level,
            psr4_namespaces = self.psr4_map.len(),
            psr0_namespaces = self.psr0_map.len(),
            classmap_entries = self.classmap.len(),
            files = self.files.len(),
            "autoloader generated"
        );

        Ok(())
    }

    /// Generate ClassLoader.php (Composer-compatible).
    fn generate_classloader(&self, dir: &Path) -> Result<()> {
        let path = dir.join("ClassLoader.php");
        let content = include_str!("templates/ClassLoader.php");
        std::fs::write(&path, content).map_err(|e| Error::io(&path, e))
    }

    /// Generate `autoload_real.php`.
    fn generate_autoload_real(&self, dir: &Path) -> Result<()> {
        let path = dir.join("autoload_real.php");
        let hash = self.generate_hash();

        let authoritative_flag = match self.optimization_level {
            OptimizationLevel::Authoritative => "true",
            _ => "false",
        };

        let content = format!(
            r"<?php

// autoload_real.php @generated by Libretto

class ComposerAutoloaderInit{hash}
{{
    private static $loader;

    public static function loadClassLoader($class)
    {{
        if ('Composer\Autoload\ClassLoader' === $class) {{
            require __DIR__ . '/ClassLoader.php';
        }}
    }}

    /**
     * @return \Composer\Autoload\ClassLoader
     */
    public static function getLoader()
    {{
        if (null !== self::$loader) {{
            return self::$loader;
        }}

        spl_autoload_register(array('ComposerAutoloaderInit{hash}', 'loadClassLoader'), true, true);
        self::$loader = $loader = new \Composer\Autoload\ClassLoader(\dirname(__DIR__));
        spl_autoload_unregister(array('ComposerAutoloaderInit{hash}', 'loadClassLoader'));

        require __DIR__ . '/autoload_static.php';
        call_user_func(\Composer\Autoload\ComposerStaticInit{hash}::getInitializer($loader));

        $loader->setClassMapAuthoritative({authoritative_flag});
        $loader->register(true);

        $filesToLoad = \Composer\Autoload\ComposerStaticInit{hash}::$files;
        $requireFile = \Closure::bind(static function ($fileIdentifier, $file) {{
            if (empty($GLOBALS['__composer_autoload_files'][$fileIdentifier])) {{
                $GLOBALS['__composer_autoload_files'][$fileIdentifier] = true;

                require $file;
            }}
        }}, null, null);
        foreach ($filesToLoad as $fileIdentifier => $file) {{
            $requireFile($fileIdentifier, $file);
        }}

        return $loader;
    }}
}}
"
        );

        std::fs::write(&path, content).map_err(|e| Error::io(&path, e))
    }

    /// Generate `autoload_static.php` with optimized data structures.
    fn generate_autoload_static(&self, dir: &Path) -> Result<()> {
        let path = dir.join("autoload_static.php");
        let hash = self.generate_hash();

        // Build prefix lengths for PSR-4
        let mut prefix_lengths: HashMap<char, HashMap<&str, usize>> = HashMap::new();
        for namespace in self.psr4_map.keys() {
            if let Some(first_char) = namespace.chars().next() {
                prefix_lengths
                    .entry(first_char)
                    .or_default()
                    .insert(namespace.as_str(), namespace.len());
            }
        }

        let mut prefix_lengths_php = String::from("array(\n");
        for (char, prefixes) in &prefix_lengths {
            prefix_lengths_php.push_str(&format!("        '{char}' => \n"));
            prefix_lengths_php.push_str("        array(\n");
            for (prefix, len) in prefixes {
                let escaped = prefix.replace('\\', "\\\\");
                prefix_lengths_php.push_str(&format!("            '{escaped}' => {len},\n"));
            }
            prefix_lengths_php.push_str("        ),\n");
        }
        prefix_lengths_php.push_str("    )");

        // PSR-4 directories
        let mut psr4_entries = String::new();
        for (namespace, paths) in &self.psr4_map {
            let escaped_ns = namespace.replace('\\', "\\\\");
            psr4_entries.push_str(&format!("        '{escaped_ns}' => array(\n"));
            for p in paths {
                let relative = self.make_relative_path(p);
                psr4_entries.push_str(&format!("            __DIR__ . '/..' . '{relative}',\n"));
            }
            psr4_entries.push_str("        ),\n");
        }

        // Classmap entries
        let mut classmap_entries = String::new();
        let mut sorted_classes: Vec<_> = self.classmap.iter().collect();
        sorted_classes.sort_by(|a, b| a.0.cmp(b.0));

        for (class, file_path) in sorted_classes {
            let escaped_class = class.replace('\\', "\\\\");
            let relative = self.make_relative_path(file_path);
            classmap_entries.push_str(&format!(
                "        '{escaped_class}' => __DIR__ . '/..' . '{relative}',\n"
            ));
        }

        // Files entries with deterministic identifiers
        let mut files_entries = String::new();
        for file_path in &self.files {
            let relative = self.make_relative_path(file_path);
            let identifier = self.generate_file_identifier(&relative);
            files_entries.push_str(&format!(
                "        '{identifier}' => __DIR__ . '/..' . '{relative}',\n"
            ));
        }

        let content = format!(
            r"<?php

// autoload_static.php @generated by Libretto

namespace Composer\Autoload;

class ComposerStaticInit{hash}
{{
    public static $files = array(
{files_entries}    );

    public static $prefixLengthsPsr4 = {prefix_lengths_php};

    public static $prefixDirsPsr4 = array(
{psr4_entries}    );

    public static $classMap = array(
{classmap_entries}    );

    public static function getInitializer(ClassLoader $loader)
    {{
        return \Closure::bind(function () use ($loader) {{
            $loader->prefixLengthsPsr4 = ComposerStaticInit{hash}::$prefixLengthsPsr4;
            $loader->prefixDirsPsr4 = ComposerStaticInit{hash}::$prefixDirsPsr4;
            $loader->classMap = ComposerStaticInit{hash}::$classMap;

        }}, null, ClassLoader::class);
    }}
}}
"
        );

        std::fs::write(&path, content).map_err(|e| Error::io(&path, e))
    }

    /// Generate `autoload_psr4.php`.
    fn generate_autoload_psr4(&self, dir: &Path) -> Result<()> {
        let path = dir.join("autoload_psr4.php");

        let mut entries = String::new();
        for (namespace, paths) in &self.psr4_map {
            let escaped_ns = namespace.replace('\\', "\\\\");
            entries.push_str(&format!("    '{escaped_ns}' => array(\n"));
            for p in paths {
                let relative = self.make_relative_path(p);
                entries.push_str(&format!("        $vendorDir . '{relative}',\n"));
            }
            entries.push_str("    ),\n");
        }

        let content = format!(
            r"<?php

// autoload_psr4.php @generated by Libretto

$vendorDir = dirname(__DIR__);
$baseDir = dirname($vendorDir);

return array(
{entries});
"
        );

        std::fs::write(&path, content).map_err(|e| Error::io(&path, e))
    }

    /// Generate `autoload_classmap.php`.
    fn generate_autoload_classmap(&self, dir: &Path) -> Result<()> {
        let path = dir.join("autoload_classmap.php");

        let mut entries = String::new();
        let mut sorted_classes: Vec<_> = self.classmap.iter().collect();
        sorted_classes.sort_by(|a, b| a.0.cmp(b.0));

        for (class, file_path) in sorted_classes {
            let escaped_class = class.replace('\\', "\\\\");
            let relative = self.make_relative_path(file_path);
            entries.push_str(&format!(
                "    '{escaped_class}' => $vendorDir . '{relative}',\n"
            ));
        }

        let content = format!(
            r"<?php

// autoload_classmap.php @generated by Libretto

$vendorDir = dirname(__DIR__);
$baseDir = dirname($vendorDir);

return array(
{entries});
"
        );

        std::fs::write(&path, content).map_err(|e| Error::io(&path, e))
    }

    /// Generate `autoload_files.php`.
    fn generate_autoload_files(&self, dir: &Path) -> Result<()> {
        let path = dir.join("autoload_files.php");

        let mut entries = String::new();
        for file_path in &self.files {
            let relative = self.make_relative_path(file_path);
            let identifier = self.generate_file_identifier(&relative);
            entries.push_str(&format!(
                "    '{identifier}' => $vendorDir . '{relative}',\n"
            ));
        }

        let content = format!(
            r"<?php

// autoload_files.php @generated by Libretto

$vendorDir = dirname(__DIR__);
$baseDir = dirname($vendorDir);

return array(
{entries});
"
        );

        std::fs::write(&path, content).map_err(|e| Error::io(&path, e))
    }

    /// Generate `autoload_namespaces.php` (for PSR-0).
    fn generate_autoload_namespaces(&self, dir: &Path) -> Result<()> {
        let path = dir.join("autoload_namespaces.php");

        let mut entries = String::new();
        for (namespace, paths) in &self.psr0_map {
            let escaped_ns = namespace.replace('\\', "\\\\");
            entries.push_str(&format!("    '{escaped_ns}' => array(\n"));
            for p in paths {
                let relative = self.make_relative_path(p);
                entries.push_str(&format!("        $vendorDir . '{relative}',\n"));
            }
            entries.push_str("    ),\n");
        }

        let content = format!(
            r"<?php

// autoload_namespaces.php @generated by Libretto

$vendorDir = dirname(__DIR__);
$baseDir = dirname($vendorDir);

return array(
{entries});
"
        );

        std::fs::write(&path, content).map_err(|e| Error::io(&path, e))
    }

    /// Generate main autoload.php.
    fn generate_autoload(&self, vendor_dir: &Path) -> Result<()> {
        let path = vendor_dir.join("autoload.php");
        let hash = self.generate_hash();

        let content = format!(
            r"<?php

// autoload.php @generated by Libretto

if (PHP_VERSION_ID < 80000) {{
    echo 'Libretto requires PHP 8.0 or higher.' . PHP_EOL;
    exit(1);
}}

require_once __DIR__ . '/composer/autoload_real.php';

return ComposerAutoloaderInit{hash}::getLoader();
"
        );

        std::fs::write(&path, content).map_err(|e| Error::io(&path, e))
    }

    /// Make path relative to vendor directory.
    fn make_relative_path(&self, path: &Path) -> String {
        let relative = path.strip_prefix(&self.vendor_dir).unwrap_or(path);

        // Ensure forward slashes and leading slash
        let path_str = relative.to_string_lossy().replace('\\', "/");
        if path_str.starts_with('/') {
            path_str
        } else {
            format!("/{path_str}")
        }
    }

    /// Generate deterministic hash for class names.
    fn generate_hash(&self) -> String {
        // Use vendor dir path for stable hash
        let hash = blake3::hash(self.vendor_dir.to_string_lossy().as_bytes());
        let bytes = hash.as_bytes();
        format!(
            "{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7]
        )
    }

    /// Generate deterministic file identifier.
    fn generate_file_identifier(&self, path: &str) -> String {
        let hash = blake3::hash(path.as_bytes());
        let bytes = hash.as_bytes();
        format!(
            "{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            bytes[0],
            bytes[1],
            bytes[2],
            bytes[3],
            bytes[4],
            bytes[5],
            bytes[6],
            bytes[7],
            bytes[8],
            bytes[9],
            bytes[10],
            bytes[11],
            bytes[12],
            bytes[13],
            bytes[14],
            bytes[15]
        )
    }

    /// Get statistics about the generated autoloader.
    #[must_use]
    pub fn stats(&self) -> AutoloaderStats {
        AutoloaderStats {
            psr4_namespaces: self.psr4_map.len(),
            psr0_namespaces: self.psr0_map.len(),
            classmap_entries: self.classmap.len(),
            files_count: self.files.len(),
            optimization_level: self.optimization_level,
        }
    }
}

/// Statistics about a generated autoloader.
#[derive(Debug, Clone)]
pub struct AutoloaderStats {
    /// Number of PSR-4 namespace mappings.
    pub psr4_namespaces: usize,
    /// Number of PSR-0 namespace mappings.
    pub psr0_namespaces: usize,
    /// Number of classmap entries.
    pub classmap_entries: usize,
    /// Number of files to autoload.
    pub files_count: usize,
    /// Optimization level used.
    pub optimization_level: OptimizationLevel,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autoload_config_default() {
        let config = AutoloadConfig::default();
        assert!(config.psr4.mappings.is_empty());
        assert!(config.classmap.paths.is_empty());
    }

    #[test]
    fn generator_creation() {
        let generator = AutoloaderGenerator::new(PathBuf::from("/tmp/vendor"));
        assert!(generator.classmap.is_empty());
    }

    #[test]
    fn optimization_levels() {
        assert_eq!(OptimizationLevel::from_int(0), OptimizationLevel::None);
        assert_eq!(OptimizationLevel::from_int(1), OptimizationLevel::Optimized);
        assert_eq!(
            OptimizationLevel::from_int(2),
            OptimizationLevel::Authoritative
        );
        assert_eq!(
            OptimizationLevel::from_int(99),
            OptimizationLevel::Authoritative
        );
    }

    #[test]
    fn cached_classmap_version() {
        let cache = CachedClassmap::new();
        assert_eq!(cache.version, CachedClassmap::VERSION);
    }

    #[test]
    fn generator_with_optimization() {
        let generator = AutoloaderGenerator::with_optimization(
            PathBuf::from("/tmp/vendor"),
            OptimizationLevel::Optimized,
        );
        assert_eq!(generator.optimization_level, OptimizationLevel::Optimized);
    }

    #[test]
    fn relative_path_generation() {
        let generator = AutoloaderGenerator::new(PathBuf::from("/home/user/project/vendor"));
        let result = generator
            .make_relative_path(Path::new("/home/user/project/vendor/autoload/src/Foo.php"));
        assert_eq!(result, "/autoload/src/Foo.php");
    }
}
