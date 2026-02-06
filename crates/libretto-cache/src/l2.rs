//! L2 disk-based content-addressable storage.
//!
//! Files are stored using their BLAKE3 hash as the filename, providing
//! content-addressable storage with automatic deduplication.

use crate::compression;
use crate::config::{CacheConfig, CacheEntryType};
use crate::index::{CacheIndex, IndexEntry, create_entry};
use libretto_core::{ContentHash, Error, Result};
use memmap2::MmapOptions;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tempfile::NamedTempFile;
use tracing::{debug, warn};

/// L2 disk-based cache.
pub struct L2Cache {
    /// Root directory for cache storage.
    root: PathBuf,
    /// Cache index.
    index: CacheIndex,
    /// Configuration.
    config: CacheConfig,
}

impl std::fmt::Debug for L2Cache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("L2Cache")
            .field("root", &self.root)
            .field("entries", &self.index.len())
            .finish()
    }
}

impl L2Cache {
    /// Create or open L2 cache at the given path.
    ///
    /// # Errors
    /// Returns error if cache directory cannot be created.
    pub fn open(root: PathBuf, config: CacheConfig) -> Result<Self> {
        fs::create_dir_all(&root).map_err(|e| Error::io(&root, e))?;

        // Create subdirectories for each entry type
        for entry_type in [
            CacheEntryType::Package,
            CacheEntryType::Metadata,
            CacheEntryType::Repository,
            CacheEntryType::DependencyGraph,
            CacheEntryType::Autoloader,
            CacheEntryType::VcsClone,
        ] {
            let subdir = root.join(entry_type.subdir());
            fs::create_dir_all(&subdir).map_err(|e| Error::io(&subdir, e))?;
        }

        let index = CacheIndex::open(root.join("index.bin"))?;

        Ok(Self {
            root,
            index,
            config,
        })
    }

    /// Get the root directory.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Get cache entry by content hash.
    ///
    /// # Errors
    /// Returns error if read fails.
    pub fn get(&self, hash: &ContentHash) -> Result<Option<Vec<u8>>> {
        let key = hash.to_hex();

        let entry = match self.index.get(&key) {
            Some(e) => e,
            None => return Ok(None),
        };

        if entry.is_expired() {
            debug!(key = %key, "cache entry expired");
            return Ok(None);
        }

        let path = self.root.join(&entry.path);
        if !path.exists() {
            warn!(path = %path.display(), "cache file missing");
            self.index.remove(&key);
            return Ok(None);
        }

        // Read file (use mmap for large files)
        let data = if entry.size > self.config.mmap_threshold {
            self.read_mmap(&path)?
        } else {
            fs::read(&path).map_err(|e| Error::io(&path, e))?
        };

        // Decompress if needed
        let data = if entry.compressed {
            if let Some(compressed_data) = compression::strip_magic(&data) {
                compression::decompress_with_hint(compressed_data, entry.original_size as usize)
                    .map_err(|e| Error::cache(format!("decompression failed: {e}")))?
            } else {
                data
            }
        } else {
            data
        };

        // Update access time
        self.index.touch(&key);

        Ok(Some(data))
    }

    /// Get raw entry without decompression (for L1 warming).
    ///
    /// # Errors
    /// Returns error if read fails.
    pub fn get_raw(&self, hash: &ContentHash) -> Result<Option<(Vec<u8>, IndexEntry)>> {
        let key = hash.to_hex();

        let entry = match self.index.get(&key) {
            Some(e) => e,
            None => return Ok(None),
        };

        if entry.is_expired() {
            return Ok(None);
        }

        let path = self.root.join(&entry.path);
        if !path.exists() {
            self.index.remove(&key);
            return Ok(None);
        }

        let data = fs::read(&path).map_err(|e| Error::io(&path, e))?;
        self.index.touch(&key);

        Ok(Some((data, entry)))
    }

    /// Store data in cache.
    ///
    /// # Errors
    /// Returns error if write fails.
    pub fn put(
        &self,
        data: &[u8],
        entry_type: CacheEntryType,
        ttl: Option<Duration>,
        metadata: Option<String>,
    ) -> Result<ContentHash> {
        let hash = ContentHash::from_bytes(data);
        let key = hash.to_hex();

        // Check if already cached
        if self.index.contains(&key) {
            debug!(key = %key, "already cached");
            self.index.touch(&key);
            return Ok(hash);
        }

        let ttl = ttl.unwrap_or_else(|| entry_type.ttl(&self.config));

        // Compress if beneficial
        let (final_data, compressed) =
            if self.config.compression_enabled && compression::should_compress(data) {
                let compressed = compression::compress(data, self.config.compression_level)
                    .map_err(|e| Error::cache(format!("compression failed: {e}")))?;

                // Only use compressed if it's actually smaller
                if compressed.len() < data.len() {
                    (compression::with_magic(compressed), true)
                } else {
                    (data.to_vec(), false)
                }
            } else {
                (data.to_vec(), false)
            };

        // Determine storage path
        let subdir = entry_type.subdir();
        let filename = format!("{}.bin", &key[..16]); // Use first 16 chars of hash
        let relative_path = format!("{subdir}/{filename}");
        let full_path = self.root.join(&relative_path);

        // Atomic write via temp file
        self.write_atomic(&full_path, &final_data)?;

        // Create index entry
        let entry = create_entry(
            key,
            entry_type,
            final_data.len() as u64,
            data.len() as u64,
            compressed,
            ttl,
            relative_path,
            metadata.unwrap_or_else(|| "{}".to_string()),
        );

        self.index.insert(entry);

        debug!(
            hash = %hash,
            size = data.len(),
            compressed_size = final_data.len(),
            "cached to L2"
        );

        Ok(hash)
    }

    /// Store raw data with known hash (for downloads).
    ///
    /// # Errors
    /// Returns error if write fails.
    pub fn put_with_hash(
        &self,
        hash: ContentHash,
        data: &[u8],
        entry_type: CacheEntryType,
        ttl: Option<Duration>,
        metadata: Option<String>,
    ) -> Result<()> {
        let key = hash.to_hex();

        if self.index.contains(&key) {
            self.index.touch(&key);
            return Ok(());
        }

        let ttl = ttl.unwrap_or_else(|| entry_type.ttl(&self.config));

        let (final_data, compressed) =
            if self.config.compression_enabled && compression::should_compress(data) {
                let compressed = compression::compress(data, self.config.compression_level)
                    .map_err(|e| Error::cache(format!("compression failed: {e}")))?;

                if compressed.len() < data.len() {
                    (compression::with_magic(compressed), true)
                } else {
                    (data.to_vec(), false)
                }
            } else {
                (data.to_vec(), false)
            };

        let subdir = entry_type.subdir();
        let filename = format!("{}.bin", &key[..16]);
        let relative_path = format!("{subdir}/{filename}");
        let full_path = self.root.join(&relative_path);

        self.write_atomic(&full_path, &final_data)?;

        let entry = create_entry(
            key,
            entry_type,
            final_data.len() as u64,
            data.len() as u64,
            compressed,
            ttl,
            relative_path,
            metadata.unwrap_or_else(|| "{}".to_string()),
        );

        self.index.insert(entry);
        Ok(())
    }

    /// Check if hash exists in cache.
    #[must_use]
    pub fn contains(&self, hash: &ContentHash) -> bool {
        let key = hash.to_hex();
        if let Some(entry) = self.index.get(&key) {
            if entry.is_expired() {
                return false;
            }
            let path = self.root.join(&entry.path);
            return path.exists();
        }
        false
    }

    /// Remove entry by hash.
    ///
    /// # Errors
    /// Returns error if removal fails.
    pub fn remove(&self, hash: &ContentHash) -> Result<bool> {
        let key = hash.to_hex();

        if let Some(entry) = self.index.remove(&key) {
            let path = self.root.join(&entry.path);
            if path.exists() {
                fs::remove_file(&path).map_err(|e| Error::io(&path, e))?;
            }
            return Ok(true);
        }
        Ok(false)
    }

    /// Get all keys of a specific type.
    #[must_use]
    pub fn keys_by_type(&self, entry_type: CacheEntryType) -> Vec<String> {
        self.index
            .find_by_type(entry_type)
            .into_iter()
            .map(|e| e.key)
            .collect()
    }

    /// Clear entries by type.
    ///
    /// # Errors
    /// Returns error if clearing fails.
    pub fn clear_by_type(&self, entry_type: CacheEntryType) -> Result<usize> {
        let entries = self.index.find_by_type(entry_type);
        let mut removed = 0;

        for entry in entries {
            let path = self.root.join(&entry.path);
            if path.exists() {
                fs::remove_file(&path).ok();
            }
            self.index.remove(&entry.key);
            removed += 1;
        }

        Ok(removed)
    }

    /// Clear all entries.
    ///
    /// # Errors
    /// Returns error if clearing fails.
    pub fn clear(&self) -> Result<()> {
        // Remove all data directories
        for entry_type in [
            CacheEntryType::Package,
            CacheEntryType::Metadata,
            CacheEntryType::Repository,
            CacheEntryType::DependencyGraph,
            CacheEntryType::Autoloader,
            CacheEntryType::VcsClone,
        ] {
            let subdir = self.root.join(entry_type.subdir());
            if subdir.exists() {
                fs::remove_dir_all(&subdir).ok();
                fs::create_dir_all(&subdir).map_err(|e| Error::io(&subdir, e))?;
            }
        }

        self.index.clear();
        Ok(())
    }

    /// Remove expired entries.
    ///
    /// # Errors
    /// Returns error if removal fails.
    pub fn remove_expired(&self) -> Result<usize> {
        let expired = self.index.find_expired();
        let mut removed = 0;

        for key in expired {
            if let Some(entry) = self.index.remove(&key) {
                let path = self.root.join(&entry.path);
                if path.exists() {
                    fs::remove_file(&path).ok();
                }
                removed += 1;
            }
        }

        Ok(removed)
    }

    /// Evict oldest entries to free space.
    ///
    /// # Errors
    /// Returns error if eviction fails.
    pub fn evict_lru(&self, target_bytes: u64) -> Result<usize> {
        let mut freed = 0u64;
        let mut removed = 0;

        // Get entries sorted by access time
        let oldest = self.index.find_oldest(100);

        for entry in oldest {
            if freed >= target_bytes {
                break;
            }

            let path = self.root.join(&entry.path);
            if path.exists() {
                fs::remove_file(&path).ok();
            }
            self.index.remove(&entry.key);

            freed += entry.size;
            removed += 1;
        }

        debug!(removed, freed, "evicted LRU entries");
        Ok(removed)
    }

    /// Get total disk usage.
    #[must_use]
    pub fn disk_usage(&self) -> u64 {
        self.index.total_size()
    }

    /// Get number of entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// Check if cache is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// Flush index to disk.
    ///
    /// # Errors
    /// Returns error if flush fails.
    pub fn flush(&self) -> Result<()> {
        self.index.flush()
    }

    /// Get all entries (for warming L1).
    #[must_use]
    pub fn entries(&self) -> Vec<IndexEntry> {
        self.index.entries()
    }

    fn read_mmap(&self, path: &Path) -> Result<Vec<u8>> {
        let file = File::open(path).map_err(|e| Error::io(path, e))?;

        // SAFETY: File is opened read-only and we hold the file handle
        let mmap = unsafe {
            MmapOptions::new()
                .map(&file)
                .map_err(|e| Error::io(path, e))?
        };

        Ok(mmap.to_vec())
    }

    fn write_atomic(&self, path: &Path, data: &[u8]) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }

        // Create temp file in same directory for atomic rename
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let mut temp = NamedTempFile::new_in(parent).map_err(|e| Error::io(parent, e))?;

        temp.write_all(data).map_err(|e| Error::io(path, e))?;
        temp.flush().map_err(|e| Error::io(path, e))?;

        // Persist and rename atomically
        temp.persist(path).map_err(|e| Error::io(path, e.error))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l2_basic_operations() {
        let dir = tempfile::tempdir().unwrap();
        let config = CacheConfig::default();
        let cache = L2Cache::open(dir.path().join("cache"), config).unwrap();

        let data = b"test data for caching";
        let hash = cache
            .put(data, CacheEntryType::Package, None, None)
            .unwrap();

        assert!(cache.contains(&hash));

        let retrieved = cache.get(&hash).unwrap().unwrap();
        assert_eq!(retrieved, data);

        cache.remove(&hash).unwrap();
        assert!(!cache.contains(&hash));
    }

    #[test]
    fn l2_compression() {
        let dir = tempfile::tempdir().unwrap();
        let config = CacheConfig {
            compression_enabled: true,
            compression_level: 3,
            ..Default::default()
        };
        let cache = L2Cache::open(dir.path().join("cache"), config).unwrap();

        // Highly compressible data
        let data = vec![0u8; 10000];
        let hash = cache
            .put(&data, CacheEntryType::Package, None, None)
            .unwrap();

        // Verify retrieval works with decompression
        let retrieved = cache.get(&hash).unwrap().unwrap();
        assert_eq!(retrieved, data);
    }

    #[test]
    fn l2_clear_by_type() {
        let dir = tempfile::tempdir().unwrap();
        let config = CacheConfig::default();
        let cache = L2Cache::open(dir.path().join("cache"), config).unwrap();

        cache
            .put(b"package data", CacheEntryType::Package, None, None)
            .unwrap();
        cache
            .put(b"metadata", CacheEntryType::Metadata, None, None)
            .unwrap();

        assert_eq!(cache.len(), 2);

        cache.clear_by_type(CacheEntryType::Package).unwrap();
        assert_eq!(cache.len(), 1);
    }
}
