//! Atomic file operations with crash-safe guarantees.
//!
//! Provides:
//! - Exclusive file locking via fs2
//! - Atomic write via temp file + rename
//! - Integrity verification before commit
//! - Crash recovery

use crate::error::{LockfileError, Result};
use crate::hash::IntegrityHasher;
use fs2::FileExt;
use parking_lot::Mutex;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, trace, warn};

/// Lock file path constants.
const TEMP_SUFFIX: &str = ".tmp";
const LOCK_SUFFIX: &str = ".lck";
const BACKUP_SUFFIX: &str = ".backup";

/// File lock acquisition timeout.
const LOCK_TIMEOUT: Duration = Duration::from_secs(30);

/// Atomic file writer with exclusive locking.
///
/// Ensures crash-safe writes using:
/// 1. Acquire exclusive lock on .lck file
/// 2. Write to temporary file
/// 3. Verify integrity
/// 4. Atomic rename
/// 5. Release lock
#[derive(Debug)]
pub struct AtomicWriter {
    /// Target file path.
    target: PathBuf,
    /// Lock file path (kept for debugging/tracing purposes).
    #[allow(dead_code)]
    lock_file_path: PathBuf,
    /// Temp file path.
    temp_path: PathBuf,
    /// Backup path.
    backup_path: PathBuf,
    /// Lock file handle (keeps lock alive).
    _lock_file: Option<File>,
    /// Content to write.
    content: Option<Vec<u8>>,
    /// Expected hash after write.
    expected_hash: Option<[u8; 32]>,
    /// Whether to create backup.
    create_backup: bool,
}

impl AtomicWriter {
    /// Create a new atomic writer for the given path.
    ///
    /// # Errors
    /// Returns error if lock cannot be acquired.
    pub fn new(path: impl AsRef<Path>) -> Result<Self> {
        let target = path.as_ref().to_path_buf();
        let lock_file_path = target.with_extension(
            target
                .extension()
                .map(|e| format!("{}.{}", e.to_string_lossy(), &LOCK_SUFFIX[1..]))
                .unwrap_or_else(|| LOCK_SUFFIX[1..].to_string()),
        );
        let temp_path = target.with_extension(
            target
                .extension()
                .map(|e| format!("{}.{}", e.to_string_lossy(), &TEMP_SUFFIX[1..]))
                .unwrap_or_else(|| TEMP_SUFFIX[1..].to_string()),
        );
        let backup_path = target.with_extension(
            target
                .extension()
                .map(|e| format!("{}.{}", e.to_string_lossy(), &BACKUP_SUFFIX[1..]))
                .unwrap_or_else(|| BACKUP_SUFFIX[1..].to_string()),
        );

        debug!(target = %target.display(), "Creating atomic writer");

        // Acquire exclusive lock
        let lock_file = acquire_lock(&lock_file_path)?;

        Ok(Self {
            target,
            lock_file_path,
            temp_path,
            backup_path,
            _lock_file: Some(lock_file),
            content: None,
            expected_hash: None,
            create_backup: true,
        })
    }

    /// Set content to write.
    pub fn content(&mut self, content: impl Into<Vec<u8>>) -> &mut Self {
        let bytes = content.into();
        self.expected_hash = Some(IntegrityHasher::hash_bytes(&bytes));
        self.content = Some(bytes);
        self
    }

    /// Disable backup creation.
    pub fn no_backup(&mut self) -> &mut Self {
        self.create_backup = false;
        self
    }

    /// Execute the atomic write.
    ///
    /// # Errors
    /// Returns error if write fails at any stage.
    pub fn commit(mut self) -> Result<WriteResult> {
        let content = self.content.take().ok_or(LockfileError::NoContent)?;
        let expected_hash = self.expected_hash.take().ok_or(LockfileError::NoContent)?;

        debug!(
            target = %self.target.display(),
            temp = %self.temp_path.display(),
            "Starting atomic write"
        );

        // Create parent directory if needed
        if let Some(parent) = self.target.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent).map_err(|e| LockfileError::io(&self.target, e))?;
            }
        }

        // Write to temp file
        {
            let mut temp_file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&self.temp_path)
                .map_err(|e| LockfileError::io(&self.temp_path, e))?;

            temp_file
                .write_all(&content)
                .map_err(|e| LockfileError::io(&self.temp_path, e))?;

            // Ensure data is flushed to disk
            temp_file
                .sync_all()
                .map_err(|e| LockfileError::io(&self.temp_path, e))?;
        }

        // Verify integrity of temp file
        let actual_hash = IntegrityHasher::hash_file(&self.temp_path)
            .map_err(|e| LockfileError::io(&self.temp_path, e))?;

        if actual_hash != expected_hash {
            // Clean up temp file
            let _ = fs::remove_file(&self.temp_path);
            return Err(LockfileError::IntegrityError {
                expected: crate::hash::bytes_to_hex(&expected_hash),
                actual: crate::hash::bytes_to_hex(&actual_hash),
            });
        }

        trace!("Temp file integrity verified");

        // Create backup of existing file
        let had_existing = self.target.exists();
        if had_existing && self.create_backup {
            fs::copy(&self.target, &self.backup_path)
                .map_err(|e| LockfileError::io(&self.backup_path, e))?;
            trace!(backup = %self.backup_path.display(), "Created backup");
        }

        // Atomic rename
        fs::rename(&self.temp_path, &self.target)
            .map_err(|e| LockfileError::io(&self.target, e))?;

        // Sync parent directory (for POSIX crash safety)
        #[cfg(unix)]
        if let Some(parent) = self.target.parent() {
            if let Ok(dir) = File::open(parent) {
                let _ = dir.sync_all();
            }
        }

        debug!(target = %self.target.display(), "Atomic write completed");

        // Clean up backup after successful write (optional)
        // We keep the backup for now in case user wants to recover

        Ok(WriteResult {
            path: self.target.clone(),
            bytes_written: content.len(),
            hash: crate::hash::bytes_to_hex(&expected_hash),
            had_existing,
        })
    }

    /// Abort the write and clean up.
    pub fn abort(self) {
        debug!(target = %self.target.display(), "Aborting atomic write");
        // Clean up temp file if it exists
        let _ = fs::remove_file(&self.temp_path);
        // Lock will be released when _lock_file is dropped
    }
}

impl Drop for AtomicWriter {
    fn drop(&mut self) {
        // Clean up temp file if still exists (indicates incomplete write)
        if self.temp_path.exists() {
            warn!(temp = %self.temp_path.display(), "Cleaning up orphaned temp file");
            let _ = fs::remove_file(&self.temp_path);
        }
        // Lock file handle dropped here, releasing lock
    }
}

/// Result of a successful atomic write.
#[derive(Debug)]
pub struct WriteResult {
    /// Path that was written.
    pub path: PathBuf,
    /// Number of bytes written.
    pub bytes_written: usize,
    /// BLAKE3 hash of content.
    pub hash: String,
    /// Whether there was an existing file.
    pub had_existing: bool,
}

/// Atomic file reader with shared locking.
#[derive(Debug)]
pub struct AtomicReader {
    /// Target file path.
    target: PathBuf,
    /// Lock file handle.
    _lock_file: Option<File>,
}

impl AtomicReader {
    /// Create a new atomic reader.
    ///
    /// # Errors
    /// Returns error if lock cannot be acquired.
    pub fn new(path: impl AsRef<Path>) -> Result<Self> {
        let target = path.as_ref().to_path_buf();
        let lock_file_path = target.with_extension(
            target
                .extension()
                .map(|e| format!("{}.{}", e.to_string_lossy(), &LOCK_SUFFIX[1..]))
                .unwrap_or_else(|| LOCK_SUFFIX[1..].to_string()),
        );

        // Acquire shared lock
        let lock_file = acquire_shared_lock(&lock_file_path)?;

        Ok(Self {
            target,
            _lock_file: Some(lock_file),
        })
    }

    /// Read the file content.
    ///
    /// # Errors
    /// Returns error if file cannot be read.
    pub fn read(&self) -> Result<Vec<u8>> {
        let mut file = File::open(&self.target).map_err(|e| LockfileError::io(&self.target, e))?;
        let mut content = Vec::new();
        file.read_to_end(&mut content)
            .map_err(|e| LockfileError::io(&self.target, e))?;
        Ok(content)
    }

    /// Read as string.
    ///
    /// # Errors
    /// Returns error if file cannot be read or is not valid UTF-8.
    pub fn read_string(&self) -> Result<String> {
        let bytes = self.read()?;
        String::from_utf8(bytes).map_err(|e| LockfileError::InvalidUtf8(e.to_string()))
    }

    /// Check if the file exists.
    #[must_use]
    pub fn exists(&self) -> bool {
        self.target.exists()
    }

    /// Get file metadata.
    ///
    /// # Errors
    /// Returns error if metadata cannot be read.
    pub fn metadata(&self) -> Result<fs::Metadata> {
        fs::metadata(&self.target).map_err(|e| LockfileError::io(&self.target, e))
    }
}

/// Acquire exclusive lock with timeout.
fn acquire_lock(path: &Path) -> Result<File> {
    use std::io::ErrorKind;

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent).map_err(|e| LockfileError::io(path, e))?;
        }
    }

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|e| LockfileError::io(path, e))?;

    // Try to acquire lock with timeout
    let start = std::time::Instant::now();
    loop {
        // Explicitly use fs2::FileExt trait method to avoid std File method shadowing
        match FileExt::try_lock_exclusive(&file) {
            Ok(()) => {
                debug!(path = %path.display(), "Acquired exclusive lock");
                return Ok(file);
            }
            Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                if start.elapsed() > LOCK_TIMEOUT {
                    return Err(LockfileError::LockTimeout {
                        path: path.to_path_buf(),
                        timeout: LOCK_TIMEOUT,
                    });
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => {
                return Err(LockfileError::io(path, e));
            }
        }
    }
}

/// Acquire shared lock with timeout.
fn acquire_shared_lock(path: &Path) -> Result<File> {
    use std::io::ErrorKind;

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent).map_err(|e| LockfileError::io(path, e))?;
        }
    }

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|e| LockfileError::io(path, e))?;

    let start = std::time::Instant::now();
    loop {
        // Explicitly use fs2::FileExt trait method to avoid std File method shadowing
        match FileExt::try_lock_shared(&file) {
            Ok(()) => {
                debug!(path = %path.display(), "Acquired shared lock");
                return Ok(file);
            }
            Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                if start.elapsed() > LOCK_TIMEOUT {
                    return Err(LockfileError::LockTimeout {
                        path: path.to_path_buf(),
                        timeout: LOCK_TIMEOUT,
                    });
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => {
                return Err(LockfileError::io(path, e));
            }
        }
    }
}

/// Transaction for multiple atomic operations.
#[derive(Debug)]
pub struct Transaction {
    /// Operations to perform.
    operations: Vec<TransactionOp>,
    /// Lock files held.
    locks: Vec<(PathBuf, File)>,
    /// Completed operations (for rollback).
    completed: Vec<TransactionOp>,
    /// Mutex for transaction state.
    state: Arc<Mutex<TransactionState>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TransactionState {
    Pending,
    Committed,
    RolledBack,
}

#[derive(Debug, Clone)]
enum TransactionOp {
    Write {
        path: PathBuf,
        content: Vec<u8>,
        backup_path: Option<PathBuf>,
    },
    Delete {
        path: PathBuf,
        backup_path: Option<PathBuf>,
    },
}

impl Transaction {
    /// Create a new transaction.
    #[must_use]
    pub fn new() -> Self {
        Self {
            operations: Vec::new(),
            locks: Vec::new(),
            completed: Vec::new(),
            state: Arc::new(Mutex::new(TransactionState::Pending)),
        }
    }

    /// Add a write operation.
    pub fn write(&mut self, path: impl AsRef<Path>, content: impl Into<Vec<u8>>) -> &mut Self {
        self.operations.push(TransactionOp::Write {
            path: path.as_ref().to_path_buf(),
            content: content.into(),
            backup_path: None,
        });
        self
    }

    /// Add a delete operation.
    pub fn delete(&mut self, path: impl AsRef<Path>) -> &mut Self {
        self.operations.push(TransactionOp::Delete {
            path: path.as_ref().to_path_buf(),
            backup_path: None,
        });
        self
    }

    /// Execute all operations atomically.
    ///
    /// # Errors
    /// Returns error if any operation fails. All completed operations
    /// will be rolled back.
    pub fn commit(mut self) -> Result<()> {
        let mut state = self.state.lock();
        if *state != TransactionState::Pending {
            return Err(LockfileError::TransactionState(
                "Transaction already completed".to_string(),
            ));
        }

        debug!(
            "Committing transaction with {} operations",
            self.operations.len()
        );

        // Acquire all locks first
        for op in &self.operations {
            let path = match op {
                TransactionOp::Write { path, .. } => path,
                TransactionOp::Delete { path, .. } => path,
            };
            let lock_path = path.with_extension("lock.lck");
            let lock = acquire_lock(&lock_path)?;
            self.locks.push((lock_path, lock));
        }

        // Execute operations
        for op in self.operations.clone() {
            match &op {
                TransactionOp::Write { path, content, .. } => {
                    // Create backup
                    let backup_path = if path.exists() {
                        let backup = path.with_extension("lock.txn.backup");
                        fs::copy(path, &backup).map_err(|e| LockfileError::io(path, e))?;
                        Some(backup)
                    } else {
                        None
                    };

                    // Write via temp file
                    let temp_path = path.with_extension("lock.txn.tmp");
                    fs::write(&temp_path, content).map_err(|e| LockfileError::io(&temp_path, e))?;
                    fs::rename(&temp_path, path).map_err(|e| LockfileError::io(path, e))?;

                    self.completed.push(TransactionOp::Write {
                        path: path.clone(),
                        content: content.clone(),
                        backup_path,
                    });
                }
                TransactionOp::Delete { path, .. } => {
                    if path.exists() {
                        // Create backup
                        let backup = path.with_extension("lock.txn.backup");
                        fs::rename(path, &backup).map_err(|e| LockfileError::io(path, e))?;

                        self.completed.push(TransactionOp::Delete {
                            path: path.clone(),
                            backup_path: Some(backup),
                        });
                    }
                }
            }
        }

        // Clean up backups
        for op in &self.completed {
            match op {
                TransactionOp::Write {
                    backup_path: Some(backup),
                    ..
                }
                | TransactionOp::Delete {
                    backup_path: Some(backup),
                    ..
                } => {
                    let _ = fs::remove_file(backup);
                }
                _ => {}
            }
        }

        *state = TransactionState::Committed;
        debug!("Transaction committed successfully");
        Ok(())
    }

    /// Rollback all completed operations.
    fn rollback(&mut self) {
        let mut state = self.state.lock();
        if *state != TransactionState::Pending {
            return;
        }

        warn!(
            "Rolling back transaction with {} completed operations",
            self.completed.len()
        );

        for op in self.completed.drain(..).rev() {
            match op {
                TransactionOp::Write {
                    path,
                    backup_path: Some(backup),
                    ..
                } => {
                    // Restore from backup
                    let _ = fs::rename(&backup, &path);
                }
                TransactionOp::Write {
                    path,
                    backup_path: None,
                    ..
                } => {
                    // Was a new file, delete it
                    let _ = fs::remove_file(&path);
                }
                TransactionOp::Delete {
                    path,
                    backup_path: Some(backup),
                } => {
                    // Restore deleted file
                    let _ = fs::rename(&backup, &path);
                }
                TransactionOp::Delete {
                    backup_path: None, ..
                } => {
                    // File didn't exist, nothing to restore
                }
            }
        }

        *state = TransactionState::RolledBack;
    }
}

impl Default for Transaction {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Transaction {
    fn drop(&mut self) {
        let state = *self.state.lock();
        if state == TransactionState::Pending && !self.completed.is_empty() {
            // Transaction was not committed, rollback
            drop(self.state.lock()); // Release lock before rollback
            self.rollback();
        }
    }
}

/// Recover from crashed atomic operations.
///
/// Cleans up orphaned temp files and restores from backups if needed.
pub fn recover(directory: &Path) -> Result<RecoveryResult> {
    let mut result = RecoveryResult::default();

    if !directory.exists() {
        return Ok(result);
    }

    for entry in fs::read_dir(directory).map_err(|e| LockfileError::io(directory, e))? {
        let entry = entry.map_err(|e| LockfileError::io(directory, e))?;
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

        // Clean up temp files
        if name.ends_with(TEMP_SUFFIX) || name.ends_with(".txn.tmp") {
            debug!(path = %path.display(), "Removing orphaned temp file");
            fs::remove_file(&path).map_err(|e| LockfileError::io(&path, e))?;
            result.temp_files_cleaned += 1;
        }

        // Clean up lock files (should auto-release but clean stale ones)
        if name.ends_with(LOCK_SUFFIX) {
            // Try to acquire lock - if successful, file is stale
            let file = OpenOptions::new().read(true).write(true).open(&path);
            if let Ok(file) = file {
                // Explicitly use fs2::FileExt trait method
                if FileExt::try_lock_exclusive(&file).is_ok() {
                    debug!(path = %path.display(), "Removing stale lock file");
                    drop(file);
                    fs::remove_file(&path).map_err(|e| LockfileError::io(&path, e))?;
                    result.lock_files_cleaned += 1;
                }
            }
        }

        // Handle backup files
        if name.ends_with(BACKUP_SUFFIX) || name.ends_with(".txn.backup") {
            // Check if original file exists
            let original_name = name
                .trim_end_matches(BACKUP_SUFFIX)
                .trim_end_matches(".txn.backup");
            let original_path = directory.join(original_name);

            if !original_path.exists() {
                // Restore from backup
                debug!(
                    backup = %path.display(),
                    original = %original_path.display(),
                    "Restoring from backup"
                );
                fs::rename(&path, &original_path).map_err(|e| LockfileError::io(&path, e))?;
                result.files_restored += 1;
            } else {
                // Original exists, backup is stale
                debug!(path = %path.display(), "Removing stale backup");
                fs::remove_file(&path).map_err(|e| LockfileError::io(&path, e))?;
                result.backups_cleaned += 1;
            }
        }
    }

    if result.has_changes() {
        debug!(
            temp = result.temp_files_cleaned,
            locks = result.lock_files_cleaned,
            backups = result.backups_cleaned,
            restored = result.files_restored,
            "Recovery completed"
        );
    }

    Ok(result)
}

/// Result of recovery operation.
#[derive(Debug, Default)]
pub struct RecoveryResult {
    /// Number of temp files cleaned.
    pub temp_files_cleaned: usize,
    /// Number of lock files cleaned.
    pub lock_files_cleaned: usize,
    /// Number of backup files cleaned.
    pub backups_cleaned: usize,
    /// Number of files restored from backup.
    pub files_restored: usize,
}

impl RecoveryResult {
    /// Check if any cleanup was performed.
    #[must_use]
    pub fn has_changes(&self) -> bool {
        self.temp_files_cleaned > 0
            || self.lock_files_cleaned > 0
            || self.backups_cleaned > 0
            || self.files_restored > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_atomic_write() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.lock");

        let mut writer = AtomicWriter::new(&path).unwrap();
        writer.content(b"hello world");
        let result = writer.commit().unwrap();

        assert_eq!(result.bytes_written, 11);
        assert!(!result.had_existing);
        assert_eq!(fs::read_to_string(&path).unwrap(), "hello world");
    }

    #[test]
    fn test_atomic_write_overwrites() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.lock");

        // First write
        fs::write(&path, "old content").unwrap();

        // Atomic overwrite
        let mut writer = AtomicWriter::new(&path).unwrap();
        writer.content(b"new content");
        let result = writer.commit().unwrap();

        assert!(result.had_existing);
        assert_eq!(fs::read_to_string(&path).unwrap(), "new content");

        // Backup should exist
        let backup = path.with_extension("lock.backup");
        assert!(backup.exists());
        assert_eq!(fs::read_to_string(&backup).unwrap(), "old content");
    }

    #[test]
    fn test_atomic_reader() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.lock");

        fs::write(&path, "test content").unwrap();

        let reader = AtomicReader::new(&path).unwrap();
        assert!(reader.exists());
        assert_eq!(reader.read_string().unwrap(), "test content");
    }

    #[test]
    fn test_transaction_commit() {
        let dir = TempDir::new().unwrap();
        let path1 = dir.path().join("file1.txt");
        let path2 = dir.path().join("file2.txt");

        let mut txn = Transaction::new();
        txn.write(&path1, b"content1");
        txn.write(&path2, b"content2");
        txn.commit().unwrap();

        assert_eq!(fs::read_to_string(&path1).unwrap(), "content1");
        assert_eq!(fs::read_to_string(&path2).unwrap(), "content2");
    }

    #[test]
    fn test_recovery() {
        let dir = TempDir::new().unwrap();

        // Create orphaned files
        fs::write(dir.path().join("test.tmp"), "orphan").unwrap();
        fs::write(dir.path().join("data.lock.lck"), "").unwrap();

        let result = recover(dir.path()).unwrap();

        assert_eq!(result.temp_files_cleaned, 1);
        assert!(!dir.path().join("test.tmp").exists());
    }
}
