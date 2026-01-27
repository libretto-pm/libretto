//! Parallel file scanner using rayon and walkdir.
//!
//! Scans directories for PHP files and extracts class definitions in parallel.

use crate::parser::{ParserPool, PhpDefinition};
use dashmap::DashMap;
use rayon::prelude::*;
use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;
use walkdir::{DirEntry, WalkDir};

/// File scan result for a single PHP file.
#[derive(Debug, Clone)]
pub struct FileScanResult {
    /// Path to the PHP file.
    pub path: PathBuf,
    /// Definitions found in the file.
    pub definitions: Vec<PhpDefinition>,
    /// File modification time.
    pub mtime: u64,
    /// File size in bytes.
    pub size: u64,
    /// Content hash for change detection.
    pub content_hash: [u8; 32],
}

/// Scanner statistics.
#[derive(Debug, Default)]
pub struct ScanStats {
    /// Total files scanned.
    pub files_scanned: AtomicU64,
    /// Total definitions found.
    pub definitions_found: AtomicU64,
    /// Total bytes processed.
    pub bytes_processed: AtomicU64,
    /// Files skipped (excluded or not PHP).
    pub files_skipped: AtomicU64,
    /// Parse errors encountered.
    pub parse_errors: AtomicU64,
}

impl ScanStats {
    /// Create new stats tracker.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Get snapshot of current stats.
    #[must_use]
    pub fn snapshot(&self) -> ScanStatsSnapshot {
        ScanStatsSnapshot {
            files_scanned: self.files_scanned.load(Ordering::Relaxed),
            definitions_found: self.definitions_found.load(Ordering::Relaxed),
            bytes_processed: self.bytes_processed.load(Ordering::Relaxed),
            files_skipped: self.files_skipped.load(Ordering::Relaxed),
            parse_errors: self.parse_errors.load(Ordering::Relaxed),
        }
    }
}

/// Immutable snapshot of scan statistics.
#[derive(Debug, Clone)]
pub struct ScanStatsSnapshot {
    /// Total files scanned.
    pub files_scanned: u64,
    /// Total definitions found.
    pub definitions_found: u64,
    /// Total bytes processed.
    pub bytes_processed: u64,
    /// Files skipped.
    pub files_skipped: u64,
    /// Parse errors encountered.
    pub parse_errors: u64,
}

/// Exclude pattern for files/directories.
#[derive(Debug, Clone)]
pub struct ExcludePattern {
    patterns: Vec<Regex>,
}

impl ExcludePattern {
    /// Create from composer.json exclude patterns.
    #[must_use]
    pub fn from_patterns(patterns: &[String]) -> Self {
        let regexes = patterns
            .iter()
            .filter_map(|p| {
                // Convert glob-like patterns to regex
                let regex_str = p.replace('.', r"\.").replace('*', ".*").replace('?', ".");
                Regex::new(&format!("(?i){regex_str}")).ok()
            })
            .collect();

        Self { patterns: regexes }
    }

    /// Check if path should be excluded.
    #[must_use]
    pub fn should_exclude(&self, path: &Path) -> bool {
        let path_str = path.to_string_lossy();
        self.patterns.iter().any(|p| p.is_match(&path_str))
    }

    /// Empty exclude pattern (excludes nothing).
    #[must_use]
    pub fn empty() -> Self {
        Self {
            patterns: Vec::new(),
        }
    }
}

impl Default for ExcludePattern {
    fn default() -> Self {
        Self::empty()
    }
}

/// Parallel PHP file scanner.
#[derive(Debug)]
pub struct Scanner {
    /// Exclude patterns.
    exclude: ExcludePattern,
    /// Statistics.
    stats: ScanStats,
    /// Results map (path -> definitions).
    results: DashMap<PathBuf, FileScanResult>,
}

impl Scanner {
    /// Create a new scanner with exclude patterns.
    #[must_use]
    pub fn new(exclude: ExcludePattern) -> Self {
        Self {
            exclude,
            stats: ScanStats::new(),
            results: DashMap::new(),
        }
    }

    /// Create scanner with no exclusions.
    #[must_use]
    pub fn without_exclusions() -> Self {
        Self::new(ExcludePattern::empty())
    }

    /// Scan a directory for PHP files in parallel.
    pub fn scan_directory(&self, root: &Path) -> Vec<FileScanResult> {
        let php_files: Vec<PathBuf> = self.collect_php_files(root);

        // Process files in parallel using rayon
        let results: Vec<FileScanResult> = php_files
            .into_par_iter()
            .filter_map(|path| self.scan_file(&path))
            .collect();

        // Store in results map
        for result in &results {
            self.results.insert(result.path.clone(), result.clone());
        }

        results
    }

    /// Scan multiple directories in parallel.
    pub fn scan_directories(&self, roots: &[PathBuf]) -> Vec<FileScanResult> {
        let all_files: Vec<PathBuf> = roots
            .par_iter()
            .flat_map(|root| self.collect_php_files(root))
            .collect();

        let results: Vec<FileScanResult> = all_files
            .into_par_iter()
            .filter_map(|path| self.scan_file(&path))
            .collect();

        for result in &results {
            self.results.insert(result.path.clone(), result.clone());
        }

        results
    }

    /// Scan a single file.
    pub fn scan_file(&self, path: &Path) -> Option<FileScanResult> {
        if self.exclude.should_exclude(path) {
            self.stats.files_skipped.fetch_add(1, Ordering::Relaxed);
            return None;
        }

        let metadata = match std::fs::metadata(path) {
            Ok(m) => m,
            Err(_) => {
                self.stats.parse_errors.fetch_add(1, Ordering::Relaxed);
                return None;
            }
        };

        let mtime = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let size = metadata.len();

        // Read file and compute hash
        let content = match std::fs::read(path) {
            Ok(c) => c,
            Err(_) => {
                self.stats.parse_errors.fetch_add(1, Ordering::Relaxed);
                return None;
            }
        };

        let content_hash = *blake3::hash(&content).as_bytes();

        // Parse the file
        let mut parser = ParserPool::get();
        let definitions = parser.parse_bytes(&content);

        self.stats.files_scanned.fetch_add(1, Ordering::Relaxed);
        self.stats
            .definitions_found
            .fetch_add(definitions.len() as u64, Ordering::Relaxed);
        self.stats
            .bytes_processed
            .fetch_add(size, Ordering::Relaxed);

        Some(FileScanResult {
            path: path.to_path_buf(),
            definitions,
            mtime,
            size,
            content_hash,
        })
    }

    /// Get scan statistics.
    #[must_use]
    pub fn stats(&self) -> ScanStatsSnapshot {
        self.stats.snapshot()
    }

    /// Get results for a specific path.
    #[must_use]
    pub fn get_result(&self, path: &Path) -> Option<FileScanResult> {
        self.results.get(path).map(|r| r.clone())
    }

    /// Get all results.
    #[must_use]
    pub fn all_results(&self) -> Vec<FileScanResult> {
        self.results.iter().map(|r| r.value().clone()).collect()
    }

    /// Clear all results.
    pub fn clear(&self) {
        self.results.clear();
    }

    fn collect_php_files(&self, root: &Path) -> Vec<PathBuf> {
        if !root.exists() {
            return Vec::new();
        }

        WalkDir::new(root)
            .follow_links(true)
            .into_iter()
            .filter_entry(|e| !self.should_skip_entry(e))
            .filter_map(|e| e.ok())
            .filter(|e| self.is_php_file(e))
            .map(|e| e.path().to_path_buf())
            .collect()
    }

    fn should_skip_entry(&self, entry: &DirEntry) -> bool {
        let path = entry.path();

        // Skip hidden directories
        if let Some(name) = path.file_name() {
            let name_str = name.to_string_lossy();
            if name_str.starts_with('.') {
                return true;
            }
            // Skip common non-PHP directories
            if entry.file_type().is_dir() {
                let skip_dirs = ["node_modules", "vendor", ".git", "__pycache__", "target"];
                if skip_dirs.contains(&name_str.as_ref()) {
                    return true;
                }
            }
        }

        self.exclude.should_exclude(path)
    }

    fn is_php_file(&self, entry: &DirEntry) -> bool {
        if !entry.file_type().is_file() {
            return false;
        }

        entry
            .path()
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("php"))
    }
}

impl Default for Scanner {
    fn default() -> Self {
        Self::without_exclusions()
    }
}

/// Build a class map from scan results.
#[must_use]
pub fn build_classmap(results: &[FileScanResult]) -> ahash::AHashMap<String, PathBuf> {
    let mut map = ahash::AHashMap::with_capacity(results.iter().map(|r| r.definitions.len()).sum());

    for result in results {
        for def in &result.definitions {
            map.insert(def.fqcn.clone(), result.path.clone());
        }
    }

    map
}

/// Build a namespace-to-directory map for PSR-4 validation.
#[must_use]
pub fn build_namespace_map(results: &[FileScanResult]) -> ahash::AHashMap<String, Vec<PathBuf>> {
    let mut map: ahash::AHashMap<String, Vec<PathBuf>> = ahash::AHashMap::new();

    for result in results {
        for def in &result.definitions {
            if let Some(ref ns) = def.namespace {
                map.entry(ns.clone()).or_default().push(result.path.clone());
            }
        }
    }

    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exclude_pattern_glob() {
        let pattern =
            ExcludePattern::from_patterns(&["*Test.php".to_string(), "tests/*".to_string()]);

        assert!(pattern.should_exclude(Path::new("UserTest.php")));
        assert!(pattern.should_exclude(Path::new("tests/bootstrap.php")));
        assert!(!pattern.should_exclude(Path::new("User.php")));
    }

    #[test]
    fn scanner_creation() {
        let scanner = Scanner::without_exclusions();
        let stats = scanner.stats();
        assert_eq!(stats.files_scanned, 0);
    }

    #[test]
    fn build_classmap_empty() {
        let results = vec![];
        let map = build_classmap(&results);
        assert!(map.is_empty());
    }
}
