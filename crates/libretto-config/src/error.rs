//! Error types for configuration management.

// False positive warnings from thiserror macro expansion
#![allow(unused_assignments)]

use miette::Diagnostic;
use std::path::PathBuf;
use thiserror::Error;

/// Configuration error type with rich diagnostics.
#[derive(Error, Debug, Diagnostic)]
pub enum ConfigError {
    /// Configuration file not found.
    #[error("configuration file not found: {path}")]
    #[diagnostic(code(config::not_found), help("create the file or check the path"))]
    NotFound {
        /// Path that was not found.
        path: PathBuf,
    },

    /// Invalid JSON syntax.
    #[error("invalid JSON in {path}: {message}")]
    #[diagnostic(
        code(config::invalid_json),
        help("check JSON syntax at line {line}, column {column}")
    )]
    InvalidJson {
        /// File path.
        path: PathBuf,
        /// Error message.
        message: String,
        /// Line number (1-indexed).
        line: usize,
        /// Column number (1-indexed).
        column: usize,
    },

    /// Missing required field.
    #[error("missing required field '{field}' in {path}")]
    #[diagnostic(
        code(config::missing_field),
        help("add the '{field}' field to your configuration")
    )]
    MissingField {
        /// Field name.
        field: String,
        /// File path.
        path: PathBuf,
    },

    /// Invalid field type.
    #[error("invalid type for '{field}' in {path}: expected {expected}, got {actual}")]
    #[diagnostic(code(config::invalid_type))]
    InvalidType {
        /// Field name.
        field: String,
        /// Expected type.
        expected: &'static str,
        /// Actual type.
        actual: String,
        /// File path.
        path: PathBuf,
    },

    /// Invalid field value.
    #[error("invalid value for '{field}': {message}")]
    #[diagnostic(code(config::invalid_value), help("{hint}"))]
    InvalidValue {
        /// Field name.
        field: String,
        /// Error message.
        message: String,
        /// Help hint.
        hint: String,
    },

    /// Value out of range.
    #[error("value for '{field}' out of range: {value} (must be {min}..{max})")]
    #[diagnostic(code(config::out_of_range))]
    OutOfRange {
        /// Field name.
        field: String,
        /// Provided value.
        value: String,
        /// Minimum value.
        min: String,
        /// Maximum value.
        max: String,
    },

    /// Invalid URL.
    #[error("invalid URL for '{field}': {url}")]
    #[diagnostic(
        code(config::invalid_url),
        help("provide a valid URL starting with http:// or https://")
    )]
    InvalidUrl {
        /// Field name.
        field: String,
        /// Invalid URL.
        url: String,
    },

    /// Invalid path.
    #[error("invalid path for '{field}': {path}")]
    #[diagnostic(code(config::invalid_path))]
    InvalidPath {
        /// Field name.
        field: String,
        /// Invalid path.
        path: String,
    },

    /// Unknown configuration key.
    #[error("unknown configuration key '{key}'")]
    #[diagnostic(code(config::unknown_key), help("valid keys are: {valid_keys}"))]
    UnknownKey {
        /// Unknown key.
        key: String,
        /// Valid keys.
        valid_keys: String,
    },

    /// IO error.
    #[error("IO error at {path}: {message}")]
    #[diagnostic(code(config::io_error))]
    Io {
        /// File path.
        path: PathBuf,
        /// Error message.
        message: String,
    },

    /// File watcher error.
    #[error("file watcher error: {0}")]
    #[diagnostic(code(config::watch_error))]
    WatchError(String),

    /// Authentication error.
    #[error("authentication error for {domain}: {message}")]
    #[diagnostic(code(config::auth_error))]
    AuthError {
        /// Domain.
        domain: String,
        /// Error message.
        message: String,
    },

    /// Keyring error.
    #[error("keyring error: {0}")]
    #[diagnostic(code(config::keyring_error))]
    KeyringError(String),

    /// Environment variable error.
    #[error("invalid environment variable {var}: {message}")]
    #[diagnostic(code(config::env_error))]
    EnvError {
        /// Variable name.
        var: String,
        /// Error message.
        message: String,
    },

    /// Configuration merge conflict.
    #[error("configuration conflict for '{key}': {message}")]
    #[diagnostic(code(config::merge_conflict))]
    MergeConflict {
        /// Configuration key.
        key: String,
        /// Error message.
        message: String,
    },

    /// Validation error with multiple issues.
    #[error("configuration validation failed with {count} error(s)")]
    #[diagnostic(code(config::validation_failed))]
    ValidationFailed {
        /// Number of errors.
        count: usize,
        /// Individual errors.
        errors: Vec<String>,
    },

    /// Circular reference in configuration.
    #[error("circular reference detected: {path}")]
    #[diagnostic(code(config::circular_reference))]
    CircularReference {
        /// Reference path.
        path: String,
    },

    /// Permission denied.
    #[error("permission denied: {path}")]
    #[diagnostic(code(config::permission_denied), help("check file permissions"))]
    PermissionDenied {
        /// File path.
        path: PathBuf,
    },

    /// Generic error for other cases.
    #[error("{0}")]
    #[diagnostic(code(config::other))]
    Other(String),
}

impl ConfigError {
    /// Create an IO error with context.
    #[must_use]
    #[allow(unused_assignments)]
    pub fn io(path: impl Into<PathBuf>, err: std::io::Error) -> Self {
        let path = path.into();
        if err.kind() == std::io::ErrorKind::NotFound {
            return Self::NotFound { path };
        }
        if err.kind() == std::io::ErrorKind::PermissionDenied {
            return Self::PermissionDenied { path };
        }
        Self::Io {
            path,
            message: err.to_string(),
        }
    }

    /// Create a JSON parse error with location.
    #[must_use]
    pub fn json(path: impl Into<PathBuf>, err: &sonic_rs::Error) -> Self {
        Self::InvalidJson {
            path: path.into(),
            message: err.to_string(),
            line: err.line(),
            column: err.column(),
        }
    }

    /// Create an invalid value error.
    #[must_use]
    pub fn invalid_value(
        field: impl Into<String>,
        message: impl Into<String>,
        hint: impl Into<String>,
    ) -> Self {
        Self::InvalidValue {
            field: field.into(),
            message: message.into(),
            hint: hint.into(),
        }
    }

    /// Create an out of range error.
    #[must_use]
    pub fn out_of_range<T: std::fmt::Display>(
        field: impl Into<String>,
        value: T,
        min: T,
        max: T,
    ) -> Self {
        Self::OutOfRange {
            field: field.into(),
            value: value.to_string(),
            min: min.to_string(),
            max: max.to_string(),
        }
    }

    /// Check if error is transient (may succeed on retry).
    #[must_use]
    pub const fn is_transient(&self) -> bool {
        matches!(self, Self::Io { .. } | Self::WatchError(_))
    }

    /// Check if error is a not found error.
    #[must_use]
    pub const fn is_not_found(&self) -> bool {
        matches!(self, Self::NotFound { .. })
    }

    /// Check if error is a permission error.
    #[must_use]
    pub const fn is_permission_denied(&self) -> bool {
        matches!(self, Self::PermissionDenied { .. })
    }
}

impl From<std::io::Error> for ConfigError {
    #[allow(unused_assignments)]
    fn from(err: std::io::Error) -> Self {
        Self::Io {
            path: PathBuf::new(),
            message: err.to_string(),
        }
    }
}

impl From<sonic_rs::Error> for ConfigError {
    #[allow(unused_assignments)]
    fn from(err: sonic_rs::Error) -> Self {
        Self::InvalidJson {
            path: PathBuf::new(),
            message: err.to_string(),
            line: err.line(),
            column: err.column(),
        }
    }
}

impl From<notify::Error> for ConfigError {
    fn from(err: notify::Error) -> Self {
        Self::WatchError(err.to_string())
    }
}

impl From<url::ParseError> for ConfigError {
    #[allow(unused_assignments)]
    fn from(err: url::ParseError) -> Self {
        Self::InvalidUrl {
            field: String::new(),
            url: err.to_string(),
        }
    }
}

impl From<ConfigError> for libretto_core::Error {
    fn from(err: ConfigError) -> Self {
        Self::config(err.to_string())
    }
}

/// Result type for configuration operations.
pub type Result<T> = std::result::Result<T, ConfigError>;
