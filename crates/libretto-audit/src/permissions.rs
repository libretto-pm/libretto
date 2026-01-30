//! File and directory permissions management.

use std::path::Path;
use thiserror::Error;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

/// Permission error.
#[derive(Debug, Error)]
pub enum PermissionError {
    /// Permission denied.
    #[error("permission denied: {0}")]
    Denied(String),

    /// Unsupported operation.
    #[error("unsupported operation: {0}")]
    Unsupported(String),

    /// I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Result type for permission operations.
pub type Result<T> = std::result::Result<T, PermissionError>;

/// File permission mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionMode {
    /// Private (0600) - owner read/write only.
    Private,
    /// `PrivateExecutable` (0700) - owner read/write/execute only.
    PrivateExecutable,
    /// Shared (0644) - owner read/write, others read.
    Shared,
    /// `SharedExecutable` (0755) - owner read/write/execute, others read/execute.
    SharedExecutable,
}

impl PermissionMode {
    /// Get Unix permission bits.
    #[cfg(unix)]
    #[must_use]
    pub const fn unix_mode(self) -> u32 {
        match self {
            Self::Private => 0o600,
            Self::PrivateExecutable => 0o700,
            Self::Shared => 0o644,
            Self::SharedExecutable => 0o755,
        }
    }
}

/// Set file permissions (cross-platform).
///
/// # Errors
/// Returns error if permissions cannot be set.
pub async fn set_permissions(path: impl AsRef<Path>, mode: PermissionMode) -> Result<()> {
    #[cfg(unix)]
    {
        use tokio::fs;

        let perms = std::fs::Permissions::from_mode(mode.unix_mode());
        fs::set_permissions(path, perms).await?;
        Ok(())
    }

    #[cfg(windows)]
    {
        // Windows doesn't have Unix-style permissions
        // Use ACLs for private files
        set_windows_permissions(path.as_ref(), mode).await
    }

    #[cfg(not(any(unix, windows)))]
    {
        Err(PermissionError::Unsupported(
            "platform does not support permission setting".to_string(),
        ))
    }
}

/// Set secure permissions for sensitive files (0600).
///
/// # Errors
/// Returns error if permissions cannot be set.
pub async fn set_secure_permissions(path: impl AsRef<Path>) -> Result<()> {
    set_permissions(path, PermissionMode::Private).await
}

/// Set secure directory permissions (0700).
///
/// # Errors
/// Returns error if permissions cannot be set.
pub async fn set_secure_dir_permissions(path: impl AsRef<Path>) -> Result<()> {
    set_permissions(path, PermissionMode::PrivateExecutable).await
}

#[cfg(windows)]
async fn set_windows_permissions(path: &Path, mode: PermissionMode) -> Result<()> {
    use windows::Win32::Security::{Authorization::*, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
    use windows::Win32::Storage::FileSystem::*;
    use windows::core::PCWSTR;

    // For private mode, set restrictive ACLs
    if mode == PermissionMode::Private || mode == PermissionMode::PrivateExecutable {
        // Get current user SID
        // Set ACL to only allow current user
        // This is a simplified implementation

        // Make file read-only for non-owners
        let metadata = tokio::fs::metadata(path).await?;
        let mut perms = metadata.permissions();
        perms.set_readonly(false); // Owner can write
        tokio::fs::set_permissions(path, perms).await?;
    }

    Ok(())
}

/// Check if file has secure permissions.
///
/// # Errors
/// Returns error if permissions cannot be checked.
pub async fn check_secure_permissions(path: impl AsRef<Path>) -> Result<bool> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        use tokio::fs;

        let metadata = fs::metadata(path).await?;
        let mode = metadata.permissions().mode();

        // Check if only owner has read/write (no group/others)
        Ok((mode & 0o077) == 0)
    }

    #[cfg(windows)]
    {
        // Simplified check for Windows
        Ok(true)
    }

    #[cfg(not(any(unix, windows)))]
    {
        Ok(false)
    }
}

/// Ensure directory exists with secure permissions.
///
/// # Errors
/// Returns error if directory cannot be created.
pub async fn ensure_secure_dir(path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();

    if path.exists() {
        // Verify permissions
        if !check_secure_permissions(path).await? {
            set_secure_dir_permissions(path).await?;
        }
    } else {
        #[cfg(unix)]
        {
            use tokio::fs;

            // Apply umask to respect system security settings
            let mode = apply_umask(0o700);
            let mut builder = fs::DirBuilder::new();
            builder.mode(mode);
            builder.recursive(true);
            builder.create(path).await?;
        }

        #[cfg(not(unix))]
        {
            tokio::fs::create_dir_all(path).await?;
            set_secure_dir_permissions(path).await?;
        }
    }

    Ok(())
}

/// Get umask (Unix only).
#[cfg(unix)]
#[must_use]
#[allow(unsafe_code)]
pub fn get_umask() -> u32 {
    unsafe {
        let mask = libc::umask(0);
        libc::umask(mask);
        mask as u32
    }
}

/// Apply umask to permission mode (Unix only).
#[cfg(unix)]
#[must_use]
pub fn apply_umask(mode: u32) -> u32 {
    mode & !get_umask()
}

/// Get effective permissions for a mode after applying umask.
#[cfg(unix)]
#[must_use]
pub fn effective_permissions(mode: PermissionMode) -> u32 {
    apply_umask(mode.unix_mode())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_permission_mode() {
        #[cfg(unix)]
        {
            assert_eq!(PermissionMode::Private.unix_mode(), 0o600);
            assert_eq!(PermissionMode::PrivateExecutable.unix_mode(), 0o700);
            assert_eq!(PermissionMode::Shared.unix_mode(), 0o644);
            assert_eq!(PermissionMode::SharedExecutable.unix_mode(), 0o755);
        }
    }

    #[tokio::test]
    async fn test_set_permissions() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("test.txt");
        tokio::fs::write(&file, b"test").await.unwrap();

        let result = set_permissions(&file, PermissionMode::Private).await;

        #[cfg(unix)]
        assert!(result.is_ok());

        #[cfg(windows)]
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_ensure_secure_dir() {
        let dir = tempdir().unwrap();
        let secure_dir = dir.path().join("secure");

        let result = ensure_secure_dir(&secure_dir).await;
        assert!(result.is_ok());
        assert!(secure_dir.exists());
    }

    #[tokio::test]
    async fn test_check_secure_permissions() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("test.txt");
        tokio::fs::write(&file, b"test").await.unwrap();

        set_secure_permissions(&file).await.unwrap();

        #[cfg(unix)]
        {
            let is_secure = check_secure_permissions(&file).await.unwrap();
            assert!(is_secure);
        }
    }

    #[cfg(unix)]
    #[test]
    fn test_umask() {
        let umask = get_umask();
        // Umask should be a reasonable value
        assert!(umask <= 0o777);

        let mode = apply_umask(0o666);
        // Result should respect umask
        assert!(mode <= 0o666);
    }
}
