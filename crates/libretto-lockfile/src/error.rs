//! Error types for lock file operations.

use std::path::PathBuf;
use std::time::Duration;
use thiserror::Error;

/// Lock file operation errors.
#[derive(Error, Debug)]
pub enum LockfileError {
    /// IO error with path context.
    #[error("IO error at {path}: {message}")]
    Io {
        /// File path.
        path: PathBuf,
        /// Error message.
        message: String,
    },

    /// JSON parsing error.
    #[error("JSON error: {0}")]
    Json(#[from] sonic_rs::Error),

    /// Lock file not found.
    #[error("Lock file not found: {0}")]
    NotFound(PathBuf),

    /// Lock acquisition timeout.
    #[error("Failed to acquire lock on {path} within {timeout:?}")]
    LockTimeout {
        /// Lock file path.
        path: PathBuf,
        /// Timeout duration.
        timeout: Duration,
    },

    /// Content integrity error.
    #[error("Integrity check failed: expected {expected}, got {actual}")]
    IntegrityError {
        /// Expected hash.
        expected: String,
        /// Actual hash.
        actual: String,
    },

    /// No content provided for write.
    #[error("No content provided for atomic write")]
    NoContent,

    /// Invalid UTF-8 content.
    #[error("Invalid UTF-8: {0}")]
    InvalidUtf8(String),

    /// Lock file validation error.
    #[error("Validation error: {0}")]
    Validation(String),

    /// Content hash mismatch (lock file drift).
    #[error(
        "Lock file is out of date: content-hash mismatch (expected {expected}, lock has {actual})"
    )]
    ContentHashMismatch {
        /// Expected hash from composer.json.
        expected: String,
        /// Actual hash in lock file.
        actual: String,
    },

    /// Manual edit detected.
    #[error("Lock file appears to be manually edited: {0}")]
    ManualEdit(String),

    /// Transaction state error.
    #[error("Transaction error: {0}")]
    TransactionState(String),

    /// Version migration error.
    #[error("Migration error: {0}")]
    Migration(String),

    /// Invalid lock file structure.
    #[error("Invalid lock file: {0}")]
    InvalidStructure(String),

    /// Package not found in lock file.
    #[error("Package '{name}' not found in lock file")]
    PackageNotFound {
        /// Package name.
        name: String,
    },

    /// Duplicate package in lock file.
    #[error("Duplicate package '{name}' in lock file")]
    DuplicatePackage {
        /// Package name.
        name: String,
    },

    /// Circular dependency detected.
    #[error("Circular dependency detected: {0}")]
    CircularDependency(String),

    /// Missing required field.
    #[error("Missing required field: {0}")]
    MissingField(String),

    /// Serialization error.
    #[error("Serialization error: {0}")]
    Serialization(String),

    /// Incompatible lock file version.
    #[error("Incompatible lock file version: {version} (supported: {supported})")]
    IncompatibleVersion {
        /// Found version.
        version: String,
        /// Supported versions.
        supported: String,
    },
}

impl LockfileError {
    /// Create an IO error with path context.
    #[must_use]
    pub fn io(path: impl Into<PathBuf>, err: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            message: err.to_string(),
        }
    }

    /// Create a validation error.
    #[must_use]
    pub fn validation(msg: impl Into<String>) -> Self {
        Self::Validation(msg.into())
    }

    /// Create an invalid structure error.
    #[must_use]
    pub fn invalid(msg: impl Into<String>) -> Self {
        Self::InvalidStructure(msg.into())
    }

    /// Create a missing field error.
    #[must_use]
    pub fn missing(field: impl Into<String>) -> Self {
        Self::MissingField(field.into())
    }

    /// Check if this is a "not found" error.
    #[must_use]
    pub fn is_not_found(&self) -> bool {
        matches!(self, Self::NotFound(_))
            || matches!(self, Self::Io { message, .. } if message.contains("not found") || message.contains("No such file"))
    }

    /// Check if this is a lock timeout error.
    #[must_use]
    pub fn is_timeout(&self) -> bool {
        matches!(self, Self::LockTimeout { .. })
    }

    /// Check if this is a drift/out-of-date error.
    #[must_use]
    pub fn is_drift(&self) -> bool {
        matches!(self, Self::ContentHashMismatch { .. })
    }
}

/// Result type for lock file operations.
pub type Result<T> = std::result::Result<T, LockfileError>;

/// Extension trait for Result to add context.
pub trait ResultExt<T> {
    /// Add path context to an error.
    fn with_path(self, path: impl Into<PathBuf>) -> Result<T>;

    /// Add validation context.
    fn validate(self, msg: impl Into<String>) -> Result<T>;
}

impl<T, E: std::error::Error> ResultExt<T> for std::result::Result<T, E> {
    fn with_path(self, path: impl Into<PathBuf>) -> Result<T> {
        self.map_err(|e| LockfileError::Io {
            path: path.into(),
            message: e.to_string(),
        })
    }

    fn validate(self, msg: impl Into<String>) -> Result<T> {
        self.map_err(|e| LockfileError::Validation(format!("{}: {}", msg.into(), e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = LockfileError::NotFound(PathBuf::from("/path/to/lock"));
        assert!(err.to_string().contains("/path/to/lock"));
        assert!(err.is_not_found());

        let err = LockfileError::ContentHashMismatch {
            expected: "abc".to_string(),
            actual: "def".to_string(),
        };
        assert!(err.is_drift());

        let err = LockfileError::LockTimeout {
            path: PathBuf::from("/lock"),
            timeout: Duration::from_secs(30),
        };
        assert!(err.is_timeout());
    }

    #[test]
    fn test_io_error() {
        let err = LockfileError::io(
            "/some/path",
            std::io::Error::new(std::io::ErrorKind::NotFound, "file not found"),
        );
        assert!(err.is_not_found());
    }
}
