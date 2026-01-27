//! Error types for the downloader.

use std::path::PathBuf;
use thiserror::Error;

/// Downloader-specific error types.
#[derive(Error, Debug)]
pub enum DownloadError {
    /// Network/HTTP error.
    #[error("network error: {message}")]
    Network {
        /// Error message.
        message: String,
        /// HTTP status code if available.
        status_code: Option<u16>,
        /// Whether the error is retryable.
        retryable: bool,
    },

    /// Connection error.
    #[error("connection failed: {0}")]
    Connection(String),

    /// Timeout error.
    #[error("timeout: {0}")]
    Timeout(String),

    /// I/O error with path context.
    #[error("I/O error at {path}: {message}")]
    Io {
        /// File path.
        path: PathBuf,
        /// Error message.
        message: String,
    },

    /// Checksum verification failed.
    #[error("checksum mismatch for '{name}': expected {expected}, got {actual}")]
    ChecksumMismatch {
        /// Package/file name.
        name: String,
        /// Expected checksum.
        expected: String,
        /// Actual computed checksum.
        actual: String,
    },

    /// Archive extraction error.
    #[error("archive error: {0}")]
    Archive(String),

    /// VCS (Git/SVN/Hg) error.
    #[error("VCS error: {0}")]
    Vcs(String),

    /// Authentication error.
    #[error("authentication failed for {domain}: {message}")]
    Authentication {
        /// Domain that rejected auth.
        domain: String,
        /// Error message.
        message: String,
    },

    /// Rate limited by server.
    #[error("rate limited by {domain}, retry after {retry_after:?}")]
    RateLimited {
        /// Domain that rate limited.
        domain: String,
        /// Suggested retry delay.
        retry_after: Option<std::time::Duration>,
    },

    /// Server returned an error.
    #[error("server error {status}: {message}")]
    ServerError {
        /// HTTP status code.
        status: u16,
        /// Error message.
        message: String,
    },

    /// Resource not found.
    #[error("not found: {url}")]
    NotFound {
        /// URL that was not found.
        url: String,
    },

    /// Download was cancelled.
    #[error("download cancelled")]
    Cancelled,

    /// Resume not supported by server.
    #[error("server does not support resume (no Accept-Ranges header)")]
    ResumeNotSupported,

    /// Invalid URL.
    #[error("invalid URL: {0}")]
    InvalidUrl(String),

    /// Unsupported protocol.
    #[error("unsupported protocol: {0}")]
    UnsupportedProtocol(String),

    /// Configuration error.
    #[error("configuration error: {0}")]
    Config(String),

    /// All mirrors failed.
    #[error("all mirrors failed for {package}")]
    AllMirrorsFailed {
        /// Package name.
        package: String,
        /// Errors from each mirror.
        errors: Vec<String>,
    },

    /// Maximum retries exceeded.
    #[error("max retries ({retries}) exceeded for {url}: {last_error}")]
    MaxRetriesExceeded {
        /// URL that failed.
        url: String,
        /// Number of retries attempted.
        retries: u32,
        /// Last error message.
        last_error: String,
    },
}

impl DownloadError {
    /// Create a network error.
    #[must_use]
    pub fn network(message: impl Into<String>) -> Self {
        Self::Network {
            message: message.into(),
            status_code: None,
            retryable: true,
        }
    }

    /// Create a network error with status code.
    #[must_use]
    pub fn network_with_status(message: impl Into<String>, status: u16) -> Self {
        let retryable = matches!(status, 408 | 429 | 500 | 502 | 503 | 504);
        Self::Network {
            message: message.into(),
            status_code: Some(status),
            retryable,
        }
    }

    /// Create an I/O error with path context.
    #[must_use]
    pub fn io(path: impl Into<PathBuf>, err: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            message: err.to_string(),
        }
    }

    /// Check if this error is retryable.
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        match self {
            Self::Network { retryable, .. } => *retryable,
            Self::Connection(_) | Self::Timeout(_) | Self::RateLimited { .. } => true,
            Self::ServerError { status, .. } => matches!(status, 500 | 502 | 503 | 504),
            _ => false,
        }
    }

    /// Check if this is a "not found" error.
    #[must_use]
    pub const fn is_not_found(&self) -> bool {
        matches!(self, Self::NotFound { .. })
            || matches!(
                self,
                Self::Network {
                    status_code: Some(404),
                    ..
                }
            )
    }

    /// Convert from reqwest error.
    #[must_use]
    pub fn from_reqwest(err: reqwest::Error) -> Self {
        if err.is_timeout() {
            return Self::Timeout(err.to_string());
        }
        if err.is_connect() {
            return Self::Connection(err.to_string());
        }
        if let Some(status) = err.status() {
            let code = status.as_u16();
            if code == 404 {
                return Self::NotFound {
                    url: err.url().map(|u| u.to_string()).unwrap_or_default(),
                };
            }
            if code == 401 || code == 403 {
                return Self::Authentication {
                    domain: err
                        .url()
                        .and_then(|u| u.host_str())
                        .unwrap_or("unknown")
                        .to_string(),
                    message: format!("HTTP {code}"),
                };
            }
            if code == 429 {
                return Self::RateLimited {
                    domain: err
                        .url()
                        .and_then(|u| u.host_str())
                        .unwrap_or("unknown")
                        .to_string(),
                    retry_after: None,
                };
            }
            return Self::network_with_status(err.to_string(), code);
        }
        Self::network(err.to_string())
    }
}

/// Result type for download operations.
pub type Result<T> = std::result::Result<T, DownloadError>;

impl From<std::io::Error> for DownloadError {
    fn from(err: std::io::Error) -> Self {
        Self::Io {
            path: PathBuf::new(),
            message: err.to_string(),
        }
    }
}

impl From<reqwest::Error> for DownloadError {
    fn from(err: reqwest::Error) -> Self {
        Self::from_reqwest(err)
    }
}

impl From<url::ParseError> for DownloadError {
    fn from(err: url::ParseError) -> Self {
        Self::InvalidUrl(err.to_string())
    }
}

impl From<DownloadError> for libretto_core::Error {
    fn from(err: DownloadError) -> Self {
        match err {
            DownloadError::Network { message, .. } => libretto_core::Error::Network(message),
            DownloadError::ChecksumMismatch {
                name,
                expected,
                actual,
            } => libretto_core::Error::ChecksumMismatch {
                name,
                expected,
                actual,
            },
            DownloadError::Archive(msg) => libretto_core::Error::Archive(msg),
            DownloadError::Vcs(msg) => libretto_core::Error::Vcs(msg),
            DownloadError::Io { path, message } => libretto_core::Error::Io { path, message },
            other => libretto_core::Error::Network(other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retryable_errors() {
        assert!(DownloadError::Connection("test".into()).is_retryable());
        assert!(DownloadError::Timeout("test".into()).is_retryable());
        assert!(DownloadError::network_with_status("test", 503).is_retryable());
        assert!(!DownloadError::network_with_status("test", 404).is_retryable());
        assert!(!DownloadError::NotFound { url: "test".into() }.is_retryable());
    }

    #[test]
    fn not_found_detection() {
        assert!(DownloadError::NotFound { url: "test".into() }.is_not_found());
        assert!(DownloadError::network_with_status("test", 404).is_not_found());
        assert!(!DownloadError::network_with_status("test", 500).is_not_found());
    }
}
