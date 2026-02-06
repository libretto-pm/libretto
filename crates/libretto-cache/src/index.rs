//! Memory-mapped metadata index for fast lookups using rkyv zero-copy deserialization.

use crate::config::CacheEntryType;
use libretto_core::{Error, Result};
use parking_lot::RwLock;
use rkyv::{Archive, Deserialize, Serialize};
use serde::{Deserialize as SerdeDeserialize, Serialize as SerdeSerialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Index entry stored in the cache index.
/// Supports both serde (for JSON fallback) and rkyv (for zero-copy).
#[derive(
    Archive, Serialize, Deserialize, SerdeSerialize, SerdeDeserialize, Debug, Clone, PartialEq, Eq,
)]
#[rkyv(compare(PartialEq), derive(Debug))]
pub struct IndexEntry {
    /// Cache key (content hash as hex).
    pub key: String,
    /// Entry type (as u8 for compact storage).
    pub entry_type: u8,
    /// Size in bytes (stored/compressed).
    pub size: u64,
    /// Original (uncompressed) size.
    pub original_size: u64,
    /// Is entry compressed.
    pub compressed: bool,
    /// Creation timestamp (seconds since epoch).
    pub created_at: u64,
    /// Last access timestamp (seconds since epoch).
    pub accessed_at: u64,
    /// TTL in seconds.
    pub ttl_secs: u64,
    /// File path relative to cache root.
    pub path: String,
    /// Additional metadata (JSON string).
    pub metadata: String,
}

impl IndexEntry {
    /// Check if entry has expired.
    #[must_use]
    pub fn is_expired(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        now > self.created_at + self.ttl_secs
    }

    /// Get time until expiration.
    #[must_use]
    pub fn time_to_expiry(&self) -> Duration {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let expires_at = self.created_at + self.ttl_secs;
        if now >= expires_at {
            Duration::ZERO
        } else {
            Duration::from_secs(expires_at - now)
        }
    }

    /// Get entry type enum.
    #[must_use]
    pub const fn cache_entry_type(&self) -> CacheEntryType {
        match self.entry_type {
            0 => CacheEntryType::Package,
            1 => CacheEntryType::Metadata,
            2 => CacheEntryType::Repository,
            3 => CacheEntryType::DependencyGraph,
            4 => CacheEntryType::Autoloader,
            5 => CacheEntryType::VcsClone,
            _ => CacheEntryType::Package,
        }
    }
}

/// Archived index entry - zero-copy access methods.
impl ArchivedIndexEntry {
    /// Check if archived entry has expired.
    #[must_use]
    pub fn is_expired(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        now > self.created_at + self.ttl_secs
    }

    /// Get cache entry type from archived entry.
    #[must_use]
    pub const fn cache_entry_type(&self) -> CacheEntryType {
        match self.entry_type {
            0 => CacheEntryType::Package,
            1 => CacheEntryType::Metadata,
            2 => CacheEntryType::Repository,
            3 => CacheEntryType::DependencyGraph,
            4 => CacheEntryType::Autoloader,
            5 => CacheEntryType::VcsClone,
            _ => CacheEntryType::Package,
        }
    }
}

/// Wrapper for index data with rkyv serialization.
#[derive(Archive, Serialize, Deserialize, Debug, Clone)]
#[rkyv(derive(Debug))]
struct IndexData {
    /// Version for format compatibility.
    version: u32,
    /// All index entries.
    entries: Vec<IndexEntry>,
}

impl IndexData {
    const CURRENT_VERSION: u32 = 1;

    /// Create empty index data.
    #[allow(dead_code)]
    const fn new() -> Self {
        Self {
            version: Self::CURRENT_VERSION,
            entries: Vec::new(),
        }
    }

    /// Create index data with entries.
    const fn with_entries(entries: Vec<IndexEntry>) -> Self {
        Self {
            version: Self::CURRENT_VERSION,
            entries,
        }
    }
}

/// Magic bytes for rkyv format detection (8-byte aligned header).
/// Format: RKIV + 4 bytes padding to maintain 8-byte alignment for rkyv data.
const RKYV_MAGIC: &[u8; 8] = b"RKIV\0\0\0\0";

/// Cache index for fast lookups.
pub struct CacheIndex {
    /// Path to index file.
    path: PathBuf,
    /// In-memory index (key -> entry).
    entries: RwLock<HashMap<String, IndexEntry>>,
    /// Dirty flag (needs flush).
    dirty: RwLock<bool>,
}

impl std::fmt::Debug for CacheIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CacheIndex")
            .field("path", &self.path)
            .field("entries_count", &self.entries.read().len())
            .field("dirty", &*self.dirty.read())
            .finish()
    }
}

impl CacheIndex {
    /// Create or open cache index.
    ///
    /// # Errors
    /// Returns error if index cannot be created or opened.
    pub fn open(path: PathBuf) -> Result<Self> {
        let index = Self {
            path,
            entries: RwLock::new(HashMap::new()),
            dirty: RwLock::new(false),
        };

        if index.path.exists() {
            index.load()?;
        }

        Ok(index)
    }

    /// Get entry by key.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<IndexEntry> {
        let entries = self.entries.read();
        entries.get(key).cloned()
    }

    /// Insert or update entry.
    pub fn insert(&self, entry: IndexEntry) {
        let mut entries = self.entries.write();
        entries.insert(entry.key.clone(), entry);
        *self.dirty.write() = true;
    }

    /// Remove entry.
    pub fn remove(&self, key: &str) -> Option<IndexEntry> {
        let mut entries = self.entries.write();
        let removed = entries.remove(key);
        if removed.is_some() {
            *self.dirty.write() = true;
        }
        removed
    }

    /// Check if key exists.
    #[must_use]
    pub fn contains(&self, key: &str) -> bool {
        self.entries.read().contains_key(key)
    }

    /// Get all keys.
    #[must_use]
    pub fn keys(&self) -> Vec<String> {
        self.entries.read().keys().cloned().collect()
    }

    /// Get all entries.
    #[must_use]
    pub fn entries(&self) -> Vec<IndexEntry> {
        self.entries.read().values().cloned().collect()
    }

    /// Get number of entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.read().len()
    }

    /// Check if index is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.read().is_empty()
    }

    /// Get total size of all entries.
    #[must_use]
    pub fn total_size(&self) -> u64 {
        self.entries.read().values().map(|e| e.size).sum()
    }

    /// Find expired entries.
    #[must_use]
    pub fn find_expired(&self) -> Vec<String> {
        self.entries
            .read()
            .iter()
            .filter(|(_, e)| e.is_expired())
            .map(|(k, _)| k.clone())
            .collect()
    }

    /// Find entries by type.
    #[must_use]
    pub fn find_by_type(&self, entry_type: CacheEntryType) -> Vec<IndexEntry> {
        let type_id = entry_type as u8;
        self.entries
            .read()
            .values()
            .filter(|e| e.entry_type == type_id)
            .cloned()
            .collect()
    }

    /// Find oldest entries (for LRU eviction).
    #[must_use]
    pub fn find_oldest(&self, count: usize) -> Vec<IndexEntry> {
        let entries = self.entries.read();
        let mut sorted: Vec<_> = entries.values().cloned().collect();
        sorted.sort_by_key(|e| e.accessed_at);
        sorted.into_iter().take(count).collect()
    }

    /// Update access time for entry.
    pub fn touch(&self, key: &str) {
        let mut entries = self.entries.write();
        if let Some(entry) = entries.get_mut(key) {
            entry.accessed_at = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            *self.dirty.write() = true;
        }
    }

    /// Clear all entries.
    pub fn clear(&self) {
        let mut entries = self.entries.write();
        entries.clear();
        *self.dirty.write() = true;
    }

    /// Load index from disk - tries rkyv first, falls back to JSON.
    fn load(&self) -> Result<()> {
        let mut file = File::open(&self.path).map_err(|e| Error::io(&self.path, e))?;

        let mut data = Vec::new();
        file.read_to_end(&mut data)
            .map_err(|e| Error::io(&self.path, e))?;

        if data.is_empty() {
            return Ok(());
        }

        // Check for rkyv magic bytes (8-byte header)
        if data.len() > 8 && &data[..4] == b"RKIV" {
            // This is rkyv format - must load as rkyv (skip 8-byte aligned header)
            match self.load_rkyv(&data[8..]) {
                Ok(entries) => {
                    let mut map = HashMap::with_capacity(entries.len());
                    for entry in entries {
                        map.insert(entry.key.clone(), entry);
                    }
                    *self.entries.write() = map;
                    return Ok(());
                }
                Err(e) => {
                    // Corrupted rkyv data - start fresh
                    tracing::warn!("Failed to load rkyv index: {}, starting fresh", e);
                    return Ok(());
                }
            }
        }

        // Try JSON (legacy format) - only if no magic bytes
        match sonic_rs::from_slice::<Vec<IndexEntry>>(&data) {
            Ok(entries) => {
                let mut map = HashMap::with_capacity(entries.len());
                for entry in entries {
                    map.insert(entry.key.clone(), entry);
                }
                *self.entries.write() = map;
                // Mark dirty to convert to rkyv on next flush
                *self.dirty.write() = true;
                Ok(())
            }
            Err(e) => {
                // Unknown format - start fresh
                tracing::warn!("Failed to load JSON index: {}, starting fresh", e);
                Ok(())
            }
        }
    }

    /// Load entries from rkyv format.
    fn load_rkyv(&self, data: &[u8]) -> std::result::Result<Vec<IndexEntry>, String> {
        // rkyv requires aligned data for zero-copy access
        // Use from_bytes which handles alignment internally
        let index_data: IndexData = rkyv::from_bytes::<IndexData, rkyv::rancor::Error>(data)
            .map_err(|e| format!("rkyv from_bytes failed: {e}"))?;

        if index_data.version != IndexData::CURRENT_VERSION {
            return Err("version mismatch".to_string());
        }

        Ok(index_data.entries)
    }

    /// Flush index to disk using rkyv format.
    ///
    /// # Errors
    /// Returns error if flush fails.
    pub fn flush(&self) -> Result<()> {
        if !*self.dirty.read() {
            return Ok(());
        }

        let entries: Vec<IndexEntry> = self.entries.read().values().cloned().collect();

        let index_data = IndexData::with_entries(entries);

        // Serialize with rkyv
        let serialized = rkyv::to_bytes::<rkyv::rancor::Error>(&index_data)
            .map_err(|e| Error::cache(format!("rkyv serialization failed: {e}")))?;

        // Prepend magic bytes
        let mut final_data = Vec::with_capacity(RKYV_MAGIC.len() + serialized.len());
        final_data.extend_from_slice(RKYV_MAGIC);
        final_data.extend_from_slice(&serialized);

        // Write atomically via temp file
        let temp_path = self.path.with_extension("tmp");

        if let Some(parent) = temp_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }

        let mut file = File::create(&temp_path).map_err(|e| Error::io(&temp_path, e))?;
        file.write_all(&final_data)
            .map_err(|e| Error::io(&temp_path, e))?;
        file.sync_all().map_err(|e| Error::io(&temp_path, e))?;

        // Atomic rename
        std::fs::rename(&temp_path, &self.path).map_err(|e| Error::io(&self.path, e))?;

        *self.dirty.write() = false;
        Ok(())
    }

    /// Export index as JSON for debugging.
    ///
    /// # Errors
    /// Returns error if export fails.
    pub fn export_json(&self) -> Result<String> {
        let entries: Vec<IndexEntry> = self.entries.read().values().cloned().collect();
        sonic_rs::to_string_pretty(&entries).map_err(|e| Error::cache(e.to_string()))
    }
}

impl Drop for CacheIndex {
    fn drop(&mut self) {
        // Best-effort flush on drop
        let _ = self.flush();
    }
}

/// Create a new index entry.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn create_entry(
    key: String,
    entry_type: CacheEntryType,
    size: u64,
    original_size: u64,
    compressed: bool,
    ttl: Duration,
    path: String,
    metadata: String,
) -> IndexEntry {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    IndexEntry {
        key,
        entry_type: entry_type as u8,
        size,
        original_size,
        compressed,
        created_at: now,
        accessed_at: now,
        ttl_secs: ttl.as_secs(),
        path,
        metadata,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_basic_operations() {
        let dir = tempfile::tempdir().unwrap();
        let index = CacheIndex::open(dir.path().join("index")).unwrap();

        let entry = create_entry(
            "test-key".to_string(),
            CacheEntryType::Package,
            1000,
            2000,
            true,
            Duration::from_secs(3600),
            "packages/test".to_string(),
            "{}".to_string(),
        );

        index.insert(entry.clone());
        assert!(index.contains("test-key"));
        assert_eq!(index.len(), 1);

        let retrieved = index.get("test-key").unwrap();
        assert_eq!(retrieved.size, 1000);

        index.remove("test-key");
        assert!(!index.contains("test-key"));
    }

    #[test]
    fn index_expiration() {
        let entry = IndexEntry {
            key: "expired".to_string(),
            entry_type: 0,
            size: 100,
            original_size: 100,
            compressed: false,
            created_at: 0, // Epoch - definitely expired
            accessed_at: 0,
            ttl_secs: 1,
            path: "test".to_string(),
            metadata: "{}".to_string(),
        };

        assert!(entry.is_expired());
    }

    #[test]
    fn index_rkyv_persistence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.bin");

        {
            let index = CacheIndex::open(path.clone()).unwrap();
            let entry = create_entry(
                "persist-key".to_string(),
                CacheEntryType::Metadata,
                500,
                500,
                false,
                Duration::from_secs(3600),
                "meta/test".to_string(),
                r#"{"test": true}"#.to_string(),
            );
            index.insert(entry);
            assert!(
                index.contains("persist-key"),
                "entry should be present after insert"
            );
            index.flush().unwrap();

            // Verify file was written with rkyv format
            assert!(path.exists(), "index file should exist after flush");
            let data = std::fs::read(&path).unwrap();
            assert!(&data[..4] == b"RKIV", "file should have RKIV magic header");
        }

        {
            let index = CacheIndex::open(path.clone()).unwrap();
            assert!(
                index.contains("persist-key"),
                "persist-key should exist after reload"
            );
            let entry = index.get("persist-key").unwrap();
            assert_eq!(entry.size, 500);
            assert_eq!(entry.metadata, r#"{"test": true}"#);
        }
    }

    #[test]
    fn index_find_by_type() {
        let dir = tempfile::tempdir().unwrap();
        let index = CacheIndex::open(dir.path().join("index")).unwrap();

        index.insert(create_entry(
            "pkg1".to_string(),
            CacheEntryType::Package,
            100,
            100,
            false,
            Duration::from_secs(3600),
            "packages/pkg1".to_string(),
            "{}".to_string(),
        ));

        index.insert(create_entry(
            "meta1".to_string(),
            CacheEntryType::Metadata,
            50,
            50,
            false,
            Duration::from_secs(3600),
            "metadata/meta1".to_string(),
            "{}".to_string(),
        ));

        let packages = index.find_by_type(CacheEntryType::Package);
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].key, "pkg1");

        let metadata = index.find_by_type(CacheEntryType::Metadata);
        assert_eq!(metadata.len(), 1);
        assert_eq!(metadata[0].key, "meta1");
    }

    #[test]
    fn index_find_oldest() {
        let dir = tempfile::tempdir().unwrap();
        let index = CacheIndex::open(dir.path().join("index")).unwrap();

        // Insert entries with different access times
        let mut entry1 = create_entry(
            "old".to_string(),
            CacheEntryType::Package,
            100,
            100,
            false,
            Duration::from_secs(3600),
            "packages/old".to_string(),
            "{}".to_string(),
        );
        entry1.accessed_at = 1000;

        let mut entry2 = create_entry(
            "new".to_string(),
            CacheEntryType::Package,
            100,
            100,
            false,
            Duration::from_secs(3600),
            "packages/new".to_string(),
            "{}".to_string(),
        );
        entry2.accessed_at = 2000;

        index.insert(entry1);
        index.insert(entry2);

        let oldest = index.find_oldest(1);
        assert_eq!(oldest.len(), 1);
        assert_eq!(oldest[0].key, "old");
    }
}
