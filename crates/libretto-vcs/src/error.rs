//! VCS error types with rich context for debugging and recovery.

use std::path::PathBuf;
use thiserror::Error;

/// VCS-specific error types with detailed context.
#[derive(Error, Debug)]
pub enum VcsError {
    /// Git operation failed.
    #[error("git error: {message}")]
    Git {
        /// Error message.
        message: String,
        /// Optional source error.
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    /// SVN operation failed.
    #[error("svn error: {message}")]
    Svn {
        /// Error message.
        message: String,
    },

    /// Mercurial operation failed.
    #[error("mercurial error: {message}")]
    Mercurial {
        /// Error message.
        message: String,
    },

    /// Fossil operation failed.
    #[error("fossil error: {message}")]
    Fossil {
        /// Error message.
        message: String,
    },

    /// Perforce operation failed.
    #[error("perforce error: {message}")]
    Perforce {
        /// Error message.
        message: String,
    },

    /// Clone operation failed.
    #[error("clone failed for {url}: {reason}")]
    CloneFailed {
        /// Repository URL.
        url: String,
        /// Failure reason.
        reason: String,
        /// Whether this error is retryable.
        retryable: bool,
    },

    /// Checkout failed.
    #[error("checkout failed for ref '{reference}': {reason}")]
    CheckoutFailed {
        /// Reference that failed.
        reference: String,
        /// Failure reason.
        reason: String,
    },

    /// Fetch operation failed.
    #[error("fetch failed for remote '{remote}': {reason}")]
    FetchFailed {
        /// Remote name.
        remote: String,
        /// Failure reason.
        reason: String,
        /// Whether this error is retryable.
        retryable: bool,
    },

    /// Authentication failed.
    #[error("authentication failed for {url}: {reason}")]
    AuthenticationFailed {
        /// Repository URL.
        url: String,
        /// Failure reason.
        reason: String,
    },

    /// SSH key error.
    #[error("ssh key error: {message}")]
    SshKey {
        /// Error message.
        message: String,
        /// Path to the key if available.
        key_path: Option<PathBuf>,
    },

    /// Host key verification failed.
    #[error("host key verification failed for {host}")]
    HostKeyVerification {
        /// Host that failed verification.
        host: String,
    },

    /// SSL/TLS certificate error.
    #[error("certificate error for {host}: {reason}")]
    Certificate {
        /// Host with certificate issue.
        host: String,
        /// Failure reason.
        reason: String,
    },

    /// Repository not found.
    #[error("repository not found: {url}")]
    RepositoryNotFound {
        /// Repository URL.
        url: String,
    },

    /// Reference not found.
    #[error("reference not found: {reference}")]
    ReferenceNotFound {
        /// Reference that was not found.
        reference: String,
    },

    /// Invalid URL.
    #[error("invalid vcs url: {url}")]
    InvalidUrl {
        /// The invalid URL.
        url: String,
        /// Reason it's invalid.
        reason: String,
    },

    /// Not a repository.
    #[error("not a repository: {path}")]
    NotRepository {
        /// Path that is not a repository.
        path: PathBuf,
    },

    /// Submodule error.
    #[error("submodule error: {message}")]
    Submodule {
        /// Error message.
        message: String,
        /// Submodule path if available.
        submodule_path: Option<PathBuf>,
    },

    /// LFS error.
    #[error("git lfs error: {message}")]
    Lfs {
        /// Error message.
        message: String,
    },

    /// Worktree error.
    #[error("worktree error: {message}")]
    Worktree {
        /// Error message.
        message: String,
    },

    /// Sparse checkout error.
    #[error("sparse checkout error: {message}")]
    SparseCheckout {
        /// Error message.
        message: String,
    },

    /// IO error.
    #[error("io error at {path}: {message}")]
    Io {
        /// File path.
        path: PathBuf,
        /// Error message.
        message: String,
    },

    /// Command execution failed.
    #[error("command '{command}' failed: {message}")]
    Command {
        /// Command that failed.
        command: String,
        /// Error message.
        message: String,
        /// Exit code if available.
        exit_code: Option<i32>,
    },

    /// VCS tool not available.
    #[error("{vcs_type} is not installed or not in PATH")]
    ToolNotAvailable {
        /// VCS type (git, svn, hg, fossil).
        vcs_type: String,
    },

    /// Timeout.
    #[error("operation timed out after {seconds}s")]
    Timeout {
        /// Timeout in seconds.
        seconds: u64,
    },

    /// Local modifications prevent operation.
    #[error("local modifications in {path}")]
    LocalModifications {
        /// Repository path.
        path: PathBuf,
    },

    /// Merge conflict.
    #[error("merge conflict in {path}")]
    MergeConflict {
        /// Path with conflict.
        path: PathBuf,
    },

    /// Signature verification failed.
    #[error("signature verification failed: {reason}")]
    SignatureVerification {
        /// Failure reason.
        reason: String,
    },
}

impl VcsError {
    /// Create a Git error from a message.
    #[must_use]
    pub fn git(message: impl Into<String>) -> Self {
        Self::Git {
            message: message.into(),
            source: None,
        }
    }

    /// Create a Git error with a source.
    #[must_use]
    pub fn git_with_source(
        message: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::Git {
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }

    /// Create an IO error.
    #[must_use]
    pub fn io(path: impl Into<PathBuf>, err: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            message: err.to_string(),
        }
    }

    /// Create a clone failed error.
    #[must_use]
    pub fn clone_failed(
        url: impl Into<String>,
        reason: impl Into<String>,
        retryable: bool,
    ) -> Self {
        Self::CloneFailed {
            url: url.into(),
            reason: reason.into(),
            retryable,
        }
    }

    /// Check if this error is retryable.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::CloneFailed { retryable, .. } | Self::FetchFailed { retryable, .. } => *retryable,
            Self::Timeout { .. } => true,
            Self::Git { message, .. } => {
                // Network-related git errors are retryable
                message.contains("network")
                    || message.contains("timeout")
                    || message.contains("connection")
                    || message.contains("temporary")
            }
            _ => false,
        }
    }

    /// Check if this is a "not found" error.
    #[must_use]
    pub const fn is_not_found(&self) -> bool {
        matches!(
            self,
            Self::RepositoryNotFound { .. } | Self::ReferenceNotFound { .. }
        )
    }

    /// Check if this is an authentication error.
    #[must_use]
    pub const fn is_auth_error(&self) -> bool {
        matches!(
            self,
            Self::AuthenticationFailed { .. }
                | Self::SshKey { .. }
                | Self::HostKeyVerification { .. }
        )
    }
}

impl From<std::io::Error> for VcsError {
    fn from(err: std::io::Error) -> Self {
        Self::Io {
            path: PathBuf::new(),
            message: err.to_string(),
        }
    }
}

impl From<VcsError> for libretto_core::Error {
    fn from(err: VcsError) -> Self {
        Self::Vcs(err.to_string())
    }
}

/// Result type for VCS operations.
pub type Result<T> = std::result::Result<T, VcsError>;
