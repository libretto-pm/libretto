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

mod fast_parser;
mod parser;
mod scanner;

pub use fast_parser::FastScanner;

pub use parser::{DefinitionKind, PhpDefinition, PhpParser};
pub use scanner::{ExcludePattern, FileScanResult, Scanner, build_classmap, build_namespace_map};

use ahash::{AHashMap, AHashSet};
use libretto_core::{Error, Result};
use parking_lot::RwLock;
use rayon::prelude::*;
use rkyv::{Archive, Deserialize, Serialize};
use serde::{Deserialize as SerdeDeserialize, Serialize as SerdeSerialize};
use std::collections::{BTreeMap, HashMap};
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
    /// Generated classmap (class -> path). Using `BTreeMap` for pre-sorted iteration.
    classmap: BTreeMap<String, PathBuf>,
    /// PSR-4 namespace mappings.
    psr4_map: HashMap<String, Vec<PathBuf>>,
    /// PSR-0 namespace mappings.
    psr0_map: HashMap<String, Vec<PathBuf>>,
    /// Files to always include.
    files: Vec<PathBuf>,
    /// Global exclude patterns for scanning.
    global_exclude: ExcludePattern,
    /// Incremental cache.
    cache: Option<Arc<IncrementalCache>>,
    /// Scanner for PHP files.
    scanner: Scanner,
    /// Pending directories to scan (collected during `add_package`, scanned in finalize).
    pending_scan_jobs: Vec<ScanJob>,
    /// PSR compliance warnings collected during autoload generation.
    psr_warnings: Vec<String>,
    /// Whether `finalize()` has been called.
    finalized: bool,
}

#[derive(Debug, Clone)]
struct ScanJob {
    dir: PathBuf,
    base: PathBuf,
    exclude: Arc<ExcludePattern>,
    kind: ScanKind,
    enforce_psr_compliance: bool,
}

#[derive(Debug, Clone)]
enum ScanKind {
    Psr4 { namespace: String, root: PathBuf },
    Psr0 { namespace: String, root: PathBuf },
    Classmap,
}

impl ScanKind {
    fn dedup_key(&self) -> String {
        match self {
            Self::Psr4 { namespace, root } => {
                format!("psr4:{namespace}:{}", root.to_string_lossy())
            }
            Self::Psr0 { namespace, root } => {
                format!("psr0:{namespace}:{}", root.to_string_lossy())
            }
            Self::Classmap => "classmap".to_string(),
        }
    }
}

impl AutoloaderGenerator {
    /// Create new generator.
    #[must_use]
    pub fn new(vendor_dir: PathBuf) -> Self {
        Self {
            vendor_dir,
            optimization_level: OptimizationLevel::None,
            classmap: BTreeMap::new(),
            psr4_map: HashMap::new(),
            psr0_map: HashMap::new(),
            files: Vec::new(),
            global_exclude: ExcludePattern::empty(),
            cache: None,
            scanner: Scanner::without_exclusions(),
            pending_scan_jobs: Vec::new(),
            psr_warnings: Vec::new(),
            finalized: false,
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
        let exclude = ExcludePattern::from_patterns(patterns);
        self.global_exclude = exclude.clone();
        self.scanner = Scanner::new(exclude);
        self
    }

    /// Add autoload configuration from a package.
    ///
    /// This collects paths for scanning but doesn't scan immediately.
    /// Call `finalize()` after all packages are added to perform the batch scan.
    pub fn add_package(&mut self, package_dir: &Path, config: &AutoloadConfig) {
        let base = package_dir.to_path_buf();
        let enforce_psr_compliance = !package_dir.starts_with(&self.vendor_dir);
        let mut exclude = ExcludePattern::from_patterns(&config.exclude.patterns);
        exclude.extend_from(&self.global_exclude);
        let exclude = Arc::new(exclude);

        // Add PSR-4 mappings
        for (namespace, dirs) in &config.psr4.mappings {
            let paths: Vec<PathBuf> = dirs.iter().map(|d| package_dir.join(d)).collect();

            // Queue PSR-4 paths for scanning if optimized
            if self.optimization_level >= OptimizationLevel::Optimized {
                for path in &paths {
                    if path.exists() {
                        self.pending_scan_jobs.push(ScanJob {
                            dir: path.clone(),
                            base: base.clone(),
                            exclude: exclude.clone(),
                            kind: ScanKind::Psr4 {
                                namespace: namespace.clone(),
                                root: path.clone(),
                            },
                            enforce_psr_compliance,
                        });
                    }
                }
            }

            self.psr4_map
                .entry(namespace.clone())
                .or_default()
                .extend(paths);
        }

        // Add PSR-0 mappings
        for (namespace, dirs) in &config.psr0.mappings {
            let paths: Vec<PathBuf> = dirs.iter().map(|d| package_dir.join(d)).collect();

            // Queue PSR-0 paths for scanning if optimized
            if self.optimization_level >= OptimizationLevel::Optimized {
                for path in &paths {
                    if path.exists() {
                        self.pending_scan_jobs.push(ScanJob {
                            dir: path.clone(),
                            base: base.clone(),
                            exclude: exclude.clone(),
                            kind: ScanKind::Psr0 {
                                namespace: namespace.clone(),
                                root: path.clone(),
                            },
                            enforce_psr_compliance,
                        });
                    }
                }
            }

            self.psr0_map
                .entry(namespace.clone())
                .or_default()
                .extend(paths);
        }

        // Always queue explicit classmap paths for scanning
        for path in &config.classmap.paths {
            let full_path = package_dir.join(path);
            if full_path.exists() {
                self.pending_scan_jobs.push(ScanJob {
                    dir: full_path,
                    base: base.clone(),
                    exclude: exclude.clone(),
                    kind: ScanKind::Classmap,
                    enforce_psr_compliance,
                });
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

    /// Finalize package collection and perform batch scanning.
    ///
    /// This deduplicates directories and scans all of them in parallel.
    /// Must be called before `generate()`.
    pub fn finalize(&mut self) {
        if self.finalized {
            return;
        }
        self.finalized = true;

        if self.pending_scan_jobs.is_empty() {
            return;
        }

        // Deduplicate directories using canonical paths
        let unique_jobs: Vec<ScanJob> = {
            let mut seen: AHashSet<(PathBuf, String)> =
                AHashSet::with_capacity(self.pending_scan_jobs.len());
            self.pending_scan_jobs
                .drain(..)
                .filter(|job| {
                    // Use canonical path for deduplication to handle symlinks
                    let canonical = job.dir.canonicalize().unwrap_or_else(|_| job.dir.clone());
                    seen.insert((canonical, job.kind.dedup_key()))
                })
                .collect()
        };

        debug!(
            "Scanning {} unique directories for classes",
            unique_jobs.len()
        );

        // Scan all directories in parallel and collect results
        let all_results: Vec<(ScanJob, Vec<fast_parser::FastScanResult>)> = unique_jobs
            .into_par_iter()
            .map(|job| {
                let results = fast_parser::FastScanner::scan_directory_with_exclude(
                    &job.dir,
                    &job.base,
                    job.exclude.as_ref(),
                );
                (job, results)
            })
            .collect();

        // Build classmap from results (BTreeMap is already sorted)
        let mut warnings = AHashSet::new();
        for (job, results) in all_results {
            for result in results {
                let classes = Self::prefer_file_stem_classes(result.classes, &result.path);
                for class in classes {
                    match &job.kind {
                        ScanKind::Classmap => {
                            self.classmap.insert(class, result.path.clone());
                        }
                        ScanKind::Psr4 { namespace, root } => {
                            if !job.enforce_psr_compliance {
                                self.classmap.insert(class, result.path.clone());
                                continue;
                            }
                            if Self::psr4_class_matches(namespace, root, &result.path, &class) {
                                self.classmap.insert(class, result.path.clone());
                            } else {
                                warnings.insert(Self::format_psr_warning(
                                    "psr-4",
                                    &class,
                                    &result.path,
                                    namespace,
                                    root,
                                    &self.vendor_dir,
                                ));
                            }
                        }
                        ScanKind::Psr0 { namespace, root } => {
                            if !job.enforce_psr_compliance {
                                self.classmap.insert(class, result.path.clone());
                                continue;
                            }
                            if Self::psr0_class_matches(namespace, root, &result.path, &class) {
                                self.classmap.insert(class, result.path.clone());
                            } else {
                                warnings.insert(Self::format_psr_warning(
                                    "psr-0",
                                    &class,
                                    &result.path,
                                    namespace,
                                    root,
                                    &self.vendor_dir,
                                ));
                            }
                        }
                    }
                }
            }
        }
        self.psr_warnings = warnings.into_iter().collect();
        self.psr_warnings.sort_unstable();

        if self.optimization_level >= OptimizationLevel::Optimized {
            self.add_composer_runtime_classes();
        }

        debug!("Found {} classes", self.classmap.len());
    }

    /// Prefer classes whose short name matches the file stem when a file
    /// yields multiple declarations.
    ///
    /// Composer's optimized classmap behavior effectively prioritizes
    /// filename-matching declarations for PSR paths. This also guards against
    /// scanner artifacts in expressions like `$node->class instanceof ...`.
    fn prefer_file_stem_classes(classes: Vec<String>, path: &Path) -> Vec<String> {
        if classes.len() <= 1 {
            return classes;
        }

        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            return classes;
        };

        let matching: Vec<String> = classes
            .iter()
            .filter(|class| class.rsplit('\\').next().is_some_and(|short| short == stem))
            .cloned()
            .collect();

        if matching.is_empty() {
            classes
        } else {
            matching
        }
    }

    /// Add Composer runtime classes that are generated in `vendor/composer`.
    fn add_composer_runtime_classes(&mut self) {
        let installed_versions_path = self.vendor_dir.join("composer/InstalledVersions.php");
        if installed_versions_path.exists() {
            self.classmap.insert(
                "Composer\\InstalledVersions".to_string(),
                installed_versions_path,
            );
        }
    }

    /// Get PSR compliance warnings collected during generation.
    #[must_use]
    pub fn warnings(&self) -> &[String] {
        &self.psr_warnings
    }

    fn psr4_class_matches(namespace: &str, root: &Path, file_path: &Path, class: &str) -> bool {
        let relative_class = if namespace.is_empty() {
            class
        } else {
            let Some(rest) = class.strip_prefix(namespace) else {
                return false;
            };
            if rest.is_empty() {
                return false;
            }
            rest
        };

        let expected = format!("{}.php", relative_class.replace('\\', "/"));
        let Ok(relative_path) = file_path.strip_prefix(root) else {
            return false;
        };
        Self::normalize_path(relative_path) == expected
    }

    fn psr0_class_matches(namespace: &str, root: &Path, file_path: &Path, class: &str) -> bool {
        let relative_class = if namespace.is_empty() {
            class
        } else {
            let Some(rest) = class.strip_prefix(namespace) else {
                return false;
            };
            if rest.is_empty() {
                return false;
            }
            rest
        };

        let expected = format!("{}.php", Self::psr0_suffix_to_path(relative_class));
        let Ok(relative_path) = file_path.strip_prefix(root) else {
            return false;
        };
        Self::normalize_path(relative_path) == expected
    }

    fn psr0_suffix_to_path(relative_class: &str) -> String {
        let logical = relative_class.replace('\\', "/");
        if let Some((namespace_part, class_part)) = logical.rsplit_once('/') {
            if namespace_part.is_empty() {
                class_part.replace('_', "/")
            } else {
                format!("{namespace_part}/{}", class_part.replace('_', "/"))
            }
        } else {
            logical.replace('_', "/")
        }
    }

    fn format_psr_warning(
        standard: &str,
        class: &str,
        class_path: &Path,
        namespace: &str,
        mapping_path: &Path,
        vendor_dir: &Path,
    ) -> String {
        let project_root = vendor_dir.parent().unwrap_or_else(|| Path::new("."));
        let class_display = Self::display_project_path(class_path, project_root);
        let mut mapping_display = Self::display_project_path(mapping_path, project_root);
        if mapping_display != "./" {
            mapping_display = mapping_display.trim_end_matches('/').to_string();
        }
        format!(
            "Class {class} located in {class_display} does not comply with {standard} autoloading standard (rule: {namespace} => {mapping_display}). Skipping."
        )
    }

    fn display_project_path(path: &Path, project_root: &Path) -> String {
        if let Ok(relative) = path.strip_prefix(project_root) {
            let relative = Self::normalize_path(relative);
            let relative = relative.trim_end_matches('/');
            if relative.is_empty() {
                "./".to_string()
            } else {
                format!("./{relative}")
            }
        } else {
            Self::normalize_path(path).trim_end_matches('/').to_string()
        }
    }

    fn normalize_path(path: &Path) -> String {
        path.to_string_lossy().replace('\\', "/")
    }

    /// Generate all autoloader files.
    ///
    /// # Errors
    /// Returns error if generation fails.
    pub fn generate(&mut self) -> Result<()> {
        // Ensure finalize has been called
        if !self.finalized {
            self.finalize();
        }

        let autoload_dir = self.vendor_dir.join("composer");
        std::fs::create_dir_all(&autoload_dir).map_err(|e| Error::io(&autoload_dir, e))?;

        // Pre-compute shared data for parallel generation
        let hash = self.generate_hash();
        let vendor_dir = self.vendor_dir.clone();

        // Generate files in parallel where possible
        // Group 1: Files that have no dependencies on each other
        let classloader_content = include_str!("templates/ClassLoader.php").to_string();
        let autoload_real_content = self.build_autoload_real(&hash);
        let autoload_static_content = self.build_autoload_static(&hash);
        let autoload_psr4_content = self.build_autoload_psr4();
        let autoload_classmap_content = self.build_autoload_classmap();
        let autoload_files_content = self.build_autoload_files();
        let autoload_namespaces_content = self.build_autoload_namespaces();
        let autoload_content = self.build_autoload(&hash);

        // Write all files in parallel
        let files_to_write: Vec<(PathBuf, String)> = vec![
            (autoload_dir.join("ClassLoader.php"), classloader_content),
            (
                autoload_dir.join("autoload_real.php"),
                autoload_real_content,
            ),
            (
                autoload_dir.join("autoload_static.php"),
                autoload_static_content,
            ),
            (
                autoload_dir.join("autoload_psr4.php"),
                autoload_psr4_content,
            ),
            (
                autoload_dir.join("autoload_classmap.php"),
                autoload_classmap_content,
            ),
            (
                autoload_dir.join("autoload_files.php"),
                autoload_files_content,
            ),
            (
                autoload_dir.join("autoload_namespaces.php"),
                autoload_namespaces_content,
            ),
            (vendor_dir.join("autoload.php"), autoload_content),
        ];

        // Write files in parallel
        files_to_write
            .into_par_iter()
            .try_for_each(|(path, content)| {
                std::fs::write(&path, content).map_err(|e| Error::io(&path, e))
            })?;

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

    /// Build `autoload_real.php` content.
    fn build_autoload_real(&self, hash: &str) -> String {
        let authoritative_flag = match self.optimization_level {
            OptimizationLevel::Authoritative => "true",
            _ => "false",
        };

        format!(
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
        )
    }

    /// Build `autoload_static.php` content with optimized data structures.
    fn build_autoload_static(&self, hash: &str) -> String {
        // Build PSR-4 prefix lengths and split fallback dirs (empty prefix)
        let mut prefix_lengths: BTreeMap<char, BTreeMap<String, usize>> = BTreeMap::new();
        let mut psr4_regular: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut psr4_fallback: Vec<String> = Vec::new();
        for (namespace, paths) in &self.psr4_map {
            if namespace.is_empty() {
                for p in paths {
                    psr4_fallback.push(self.make_relative_path(p));
                }
                continue;
            }

            if let Some(first_char) = namespace.chars().next() {
                prefix_lengths
                    .entry(first_char)
                    .or_default()
                    .insert(namespace.clone(), namespace.len());
            }

            let entry = psr4_regular.entry(namespace.clone()).or_default();
            for p in paths {
                entry.push(self.make_relative_path(p));
            }
        }

        // Build PSR-0 prefixes and split fallback dirs (empty prefix)
        let mut psr0_grouped: BTreeMap<char, BTreeMap<String, Vec<String>>> = BTreeMap::new();
        let mut psr0_fallback: Vec<String> = Vec::new();
        for (namespace, paths) in &self.psr0_map {
            if namespace.is_empty() {
                for p in paths {
                    psr0_fallback.push(self.make_relative_path(p));
                }
                continue;
            }

            if let Some(first_char) = namespace.chars().next() {
                let by_prefix = psr0_grouped.entry(first_char).or_default();
                let entry = by_prefix.entry(namespace.clone()).or_default();
                for p in paths {
                    entry.push(self.make_relative_path(p));
                }
            }
        }

        let mut prefix_lengths_php = String::with_capacity(prefix_lengths.len() * 100);
        prefix_lengths_php.push_str("array(\n");
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

        // PSR-4 directories - pre-allocate
        let mut psr4_entries = String::with_capacity(psr4_regular.len() * 200);
        for (namespace, paths) in &psr4_regular {
            let escaped_ns = namespace.replace('\\', "\\\\");
            psr4_entries.push_str(&format!("        '{escaped_ns}' => array(\n"));
            for p in paths {
                psr4_entries.push_str(&format!("            __DIR__ . '/..' . '{p}',\n"));
            }
            psr4_entries.push_str("        ),\n");
        }

        // PSR-4 fallback dirs
        let mut psr4_fallback_entries = String::with_capacity(psr4_fallback.len() * 60);
        for p in &psr4_fallback {
            psr4_fallback_entries.push_str(&format!("        __DIR__ . '/..' . '{p}',\n"));
        }

        // PSR-0 prefixes
        let mut psr0_prefixes_php = String::with_capacity(psr0_grouped.len() * 160);
        psr0_prefixes_php.push_str("array(\n");
        for (char, prefixes) in &psr0_grouped {
            psr0_prefixes_php.push_str(&format!("        '{char}' => \n"));
            psr0_prefixes_php.push_str("        array(\n");
            for (namespace, paths) in prefixes {
                let escaped_ns = namespace.replace('\\', "\\\\");
                psr0_prefixes_php.push_str(&format!("            '{escaped_ns}' => array(\n"));
                for p in paths {
                    psr0_prefixes_php
                        .push_str(&format!("                __DIR__ . '/..' . '{p}',\n"));
                }
                psr0_prefixes_php.push_str("            ),\n");
            }
            psr0_prefixes_php.push_str("        ),\n");
        }
        psr0_prefixes_php.push_str("    )");

        // PSR-0 fallback dirs
        let mut psr0_fallback_entries = String::with_capacity(psr0_fallback.len() * 60);
        for p in &psr0_fallback {
            psr0_fallback_entries.push_str(&format!("        __DIR__ . '/..' . '{p}',\n"));
        }

        // Classmap entries - BTreeMap is already sorted, no need to sort again
        // Pre-allocate with estimated 80 bytes per entry
        let mut classmap_entries = String::with_capacity(self.classmap.len() * 80);
        for (class, file_path) in &self.classmap {
            let escaped_class = class.replace('\\', "\\\\");
            let relative = self.make_relative_path(file_path);
            classmap_entries.push_str(&format!(
                "        '{escaped_class}' => __DIR__ . '/..' . '{relative}',\n"
            ));
        }

        // Files entries with deterministic identifiers
        let mut files_entries = String::with_capacity(self.files.len() * 100);
        for file_path in &self.files {
            let relative = self.make_relative_path(file_path);
            let identifier = self.generate_file_identifier(&relative);
            files_entries.push_str(&format!(
                "        '{identifier}' => __DIR__ . '/..' . '{relative}',\n"
            ));
        }

        format!(
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

    public static $fallbackDirsPsr4 = array(
{psr4_fallback_entries}    );

    public static $prefixesPsr0 = {psr0_prefixes_php};

    public static $fallbackDirsPsr0 = array(
{psr0_fallback_entries}    );

    public static $classMap = array(
{classmap_entries}    );

    public static function getInitializer(ClassLoader $loader)
    {{
        return \Closure::bind(function () use ($loader) {{
            $loader->prefixLengthsPsr4 = ComposerStaticInit{hash}::$prefixLengthsPsr4;
            $loader->prefixDirsPsr4 = ComposerStaticInit{hash}::$prefixDirsPsr4;
            $loader->fallbackDirsPsr4 = ComposerStaticInit{hash}::$fallbackDirsPsr4;
            $loader->prefixesPsr0 = ComposerStaticInit{hash}::$prefixesPsr0;
            $loader->fallbackDirsPsr0 = ComposerStaticInit{hash}::$fallbackDirsPsr0;
            $loader->classMap = ComposerStaticInit{hash}::$classMap;

        }}, null, ClassLoader::class);
    }}
}}
"
        )
    }

    /// Build `autoload_psr4.php` content.
    fn build_autoload_psr4(&self) -> String {
        let mut entries = String::with_capacity(self.psr4_map.len() * 150);
        for (namespace, paths) in &self.psr4_map {
            let escaped_ns = namespace.replace('\\', "\\\\");
            entries.push_str(&format!("    '{escaped_ns}' => array(\n"));
            for p in paths {
                let relative = self.make_relative_path(p);
                entries.push_str(&format!("        $vendorDir . '{relative}',\n"));
            }
            entries.push_str("    ),\n");
        }

        format!(
            r"<?php

// autoload_psr4.php @generated by Libretto

$vendorDir = dirname(__DIR__);
$baseDir = dirname($vendorDir);

return array(
{entries});
"
        )
    }

    /// Build `autoload_classmap.php` content.
    fn build_autoload_classmap(&self) -> String {
        // BTreeMap is already sorted, no need to sort again
        let mut entries = String::with_capacity(self.classmap.len() * 60);
        for (class, file_path) in &self.classmap {
            let escaped_class = class.replace('\\', "\\\\");
            let relative = self.make_relative_path(file_path);
            entries.push_str(&format!(
                "    '{escaped_class}' => $vendorDir . '{relative}',\n"
            ));
        }

        format!(
            r"<?php

// autoload_classmap.php @generated by Libretto

$vendorDir = dirname(__DIR__);
$baseDir = dirname($vendorDir);

return array(
{entries});
"
        )
    }

    /// Build `autoload_files.php` content.
    fn build_autoload_files(&self) -> String {
        let mut entries = String::with_capacity(self.files.len() * 80);
        for file_path in &self.files {
            let relative = self.make_relative_path(file_path);
            let identifier = self.generate_file_identifier(&relative);
            entries.push_str(&format!(
                "    '{identifier}' => $vendorDir . '{relative}',\n"
            ));
        }

        format!(
            r"<?php

// autoload_files.php @generated by Libretto

$vendorDir = dirname(__DIR__);
$baseDir = dirname($vendorDir);

return array(
{entries});
"
        )
    }

    /// Build `autoload_namespaces.php` content (for PSR-0).
    fn build_autoload_namespaces(&self) -> String {
        let mut entries = String::with_capacity(self.psr0_map.len() * 150);
        for (namespace, paths) in &self.psr0_map {
            let escaped_ns = namespace.replace('\\', "\\\\");
            entries.push_str(&format!("    '{escaped_ns}' => array(\n"));
            for p in paths {
                let relative = self.make_relative_path(p);
                entries.push_str(&format!("        $vendorDir . '{relative}',\n"));
            }
            entries.push_str("    ),\n");
        }

        format!(
            r"<?php

// autoload_namespaces.php @generated by Libretto

$vendorDir = dirname(__DIR__);
$baseDir = dirname($vendorDir);

return array(
{entries});
"
        )
    }

    /// Build main autoload.php content.
    fn build_autoload(&self, hash: &str) -> String {
        format!(
            r"<?php

// autoload.php @generated by Libretto

if (PHP_VERSION_ID < 80000) {{
    echo 'Libretto requires PHP 8.0 or higher.' . PHP_EOL;
    exit(1);
}}

require_once __DIR__ . '/composer/autoload_real.php';

return ComposerAutoloaderInit{hash}::getLoader();
"
        )
    }

    /// Make path relative to vendor directory.
    fn make_relative_path(&self, path: &Path) -> String {
        // Try to strip vendor dir prefix first
        if let Ok(relative) = path.strip_prefix(&self.vendor_dir) {
            let path_str = relative.to_string_lossy().replace('\\', "/");
            if path_str.starts_with('/') {
                return path_str;
            }
            return format!("/{path_str}");
        }

        // Path is outside vendor dir (e.g., root project's app/ directory)
        // Make it relative to vendor's parent (project root)
        let vendor_parent = self.vendor_dir.parent().unwrap_or_else(|| Path::new("."));
        if let Ok(relative) = path.strip_prefix(vendor_parent) {
            let path_str = relative.to_string_lossy().replace('\\', "/");
            // Clean up leading ./ or just .
            let clean_path = path_str.trim_start_matches("./").trim_start_matches('.');
            if clean_path.is_empty() {
                return String::new();
            }
            return format!("/../{clean_path}");
        }

        // Fallback: use path as-is with leading slash
        let path_str = path.to_string_lossy().replace('\\', "/");
        let clean_path = path_str.trim_start_matches("./").trim_start_matches('.');
        format!("/../{clean_path}")
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
    use tempfile::tempdir;

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

    #[test]
    fn prefer_file_stem_classes_prefers_matching_short_name() {
        let classes = vec![
            "Symfony\\Component\\Routing\\Loader\\PhpFileLoader".to_string(),
            "Symfony\\Component\\Routing\\Loader\\ProtectedPhpFileLoader".to_string(),
        ];

        let filtered =
            AutoloaderGenerator::prefer_file_stem_classes(classes, Path::new("PhpFileLoader.php"));

        assert_eq!(
            filtered,
            vec!["Symfony\\Component\\Routing\\Loader\\PhpFileLoader"]
        );
    }

    #[test]
    fn prefer_file_stem_classes_keeps_all_when_no_match() {
        let classes = vec![
            "Vendor\\Package\\Foo".to_string(),
            "Vendor\\Package\\Bar".to_string(),
        ];

        let filtered =
            AutoloaderGenerator::prefer_file_stem_classes(classes.clone(), Path::new("Baz.php"));

        assert_eq!(filtered, classes);
    }

    #[test]
    fn autoload_static_includes_psr0_prefixes_in_initializer() {
        let tmp = tempdir().expect("create temp dir");
        let vendor = tmp.path().join("vendor");
        let package_dir = vendor.join("simplesoftwareio/simple-qrcode");
        std::fs::create_dir_all(package_dir.join("src")).expect("create package dir");

        let mut generator = AutoloaderGenerator::new(vendor);
        let mut config = AutoloadConfig::default();
        config.psr0.mappings.insert(
            "SimpleSoftwareIO\\QrCode\\".to_string(),
            vec!["src".to_string()],
        );
        generator.add_package(&package_dir, &config);

        let content = generator.build_autoload_static("testhash");
        assert!(content.contains("public static $prefixesPsr0 = array("));
        assert!(content.contains("'SimpleSoftwareIO\\\\QrCode\\\\' => array("));
        assert!(
            content.contains("$loader->prefixesPsr0 = ComposerStaticInittesthash::$prefixesPsr0;")
        );
    }

    #[test]
    fn autoload_static_includes_fallback_dirs() {
        let tmp = tempdir().expect("create temp dir");
        let vendor = tmp.path().join("vendor");
        let package_dir = vendor.join("vendor/example-fallback");
        std::fs::create_dir_all(package_dir.join("src")).expect("create package dir");

        let mut generator = AutoloaderGenerator::new(vendor);
        let mut config = AutoloadConfig::default();
        config
            .psr4
            .mappings
            .insert(String::new(), vec!["src".to_string()]);
        config
            .psr0
            .mappings
            .insert(String::new(), vec!["src".to_string()]);
        generator.add_package(&package_dir, &config);

        let content = generator.build_autoload_static("fallbackhash");
        assert!(content.contains("public static $fallbackDirsPsr4 = array("));
        assert!(content.contains("public static $fallbackDirsPsr0 = array("));
        assert!(content.contains(
            "$loader->fallbackDirsPsr4 = ComposerStaticInitfallbackhash::$fallbackDirsPsr4;"
        ));
        assert!(content.contains(
            "$loader->fallbackDirsPsr0 = ComposerStaticInitfallbackhash::$fallbackDirsPsr0;"
        ));
    }

    #[test]
    fn non_compliant_psr4_class_is_skipped_and_reported() {
        let tmp = tempdir().expect("create temp dir");
        let project_root = tmp.path().to_path_buf();
        let vendor = project_root.join("vendor");
        std::fs::create_dir_all(&vendor).expect("create vendor dir");

        let app_dir = project_root.join("app");
        std::fs::create_dir_all(&app_dir).expect("create app dir");
        std::fs::write(
            app_dir.join("NotificationSeenLast.php"),
            "<?php\nclass NotificationSeenLast {}\n",
        )
        .expect("write php file");

        let mut generator =
            AutoloaderGenerator::with_optimization(vendor, OptimizationLevel::Optimized);
        let mut config = AutoloadConfig::default();
        config
            .psr4
            .mappings
            .insert("App\\".to_string(), vec!["app".to_string()]);
        generator.add_package(project_root.as_path(), &config);
        generator.finalize();

        assert!(
            !generator.classmap.contains_key("NotificationSeenLast"),
            "non-compliant class should be skipped"
        );
        assert!(
            generator
                .warnings()
                .iter()
                .any(|w| w.contains("NotificationSeenLast")
                    && w.contains("psr-4 autoloading standard")
                    && w.contains("App\\ => ./app")),
            "expected a composer-style PSR-4 warning, got {:?}",
            generator.warnings()
        );
    }

    #[test]
    fn compliant_psr4_class_is_kept_without_warning() {
        let tmp = tempdir().expect("create temp dir");
        let project_root = tmp.path().to_path_buf();
        let vendor = project_root.join("vendor");
        std::fs::create_dir_all(&vendor).expect("create vendor dir");

        let model_dir = project_root.join("app/Models");
        std::fs::create_dir_all(&model_dir).expect("create model dir");
        std::fs::write(
            model_dir.join("User.php"),
            "<?php\nnamespace App\\Models;\nclass User {}\n",
        )
        .expect("write php file");

        let mut generator =
            AutoloaderGenerator::with_optimization(vendor, OptimizationLevel::Optimized);
        let mut config = AutoloadConfig::default();
        config
            .psr4
            .mappings
            .insert("App\\".to_string(), vec!["app".to_string()]);
        generator.add_package(project_root.as_path(), &config);
        generator.finalize();

        assert!(
            generator.classmap.contains_key("App\\Models\\User"),
            "compliant class should be in classmap"
        );
        assert!(
            generator.warnings().is_empty(),
            "no warnings expected, got {:?}",
            generator.warnings()
        );
    }

    #[test]
    fn add_composer_runtime_classes_adds_installed_versions() {
        let tmp = tempdir().expect("create temp dir");
        let vendor = tmp.path().join("vendor");
        let composer_dir = vendor.join("composer");
        std::fs::create_dir_all(&composer_dir).expect("create composer dir");

        let installed_versions = composer_dir.join("InstalledVersions.php");
        std::fs::write(&installed_versions, "<?php\n").expect("write installed versions file");

        let mut generator = AutoloaderGenerator::new(vendor.clone());
        generator.add_composer_runtime_classes();

        assert_eq!(
            generator.classmap.get("Composer\\InstalledVersions"),
            Some(&installed_versions)
        );
    }
}
