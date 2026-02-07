//! File and directory permissions management.

use std::path::Path;
use thiserror::Error;
#[cfg(windows)]
use tokio::process::Command;

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
    let path = path.as_ref();
    set_permissions(path, PermissionMode::Private).await?;

    if !check_secure_permissions(path).await? {
        return Err(PermissionError::Denied(format!(
            "failed to enforce secure permissions on {}",
            path.display()
        )));
    }

    Ok(())
}

/// Set secure directory permissions (0700).
///
/// # Errors
/// Returns error if permissions cannot be set.
pub async fn set_secure_dir_permissions(path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    set_permissions(path, PermissionMode::PrivateExecutable).await?;

    if !check_secure_permissions(path).await? {
        return Err(PermissionError::Denied(format!(
            "failed to enforce secure directory permissions on {}",
            path.display()
        )));
    }

    Ok(())
}

#[cfg(windows)]
async fn set_windows_permissions(path: &Path, mode: PermissionMode) -> Result<()> {
    let owner_sid = current_user_sid().await?;
    let path_arg = path.display().to_string();

    // Remove inheritance and overwrite ACLs with explicit trusted principals.
    let mut cmd = Command::new("icacls");
    cmd.arg(&path_arg)
        .arg("/inheritance:r")
        .arg("/grant:r")
        .arg(format!("*{owner_sid}:(F)"))
        .arg("/grant")
        .arg("*S-1-5-18:(F)") // NT AUTHORITY\SYSTEM
        .arg("/grant")
        .arg("*S-1-5-32-544:(F)"); // BUILTIN\Administrators

    if mode == PermissionMode::Shared || mode == PermissionMode::SharedExecutable {
        let users_perm = if mode == PermissionMode::Shared {
            "R"
        } else {
            "RX"
        };
        cmd.arg("/grant")
            .arg(format!("*S-1-5-32-545:({users_perm})")); // BUILTIN\Users
    }

    let output = cmd.output().await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(PermissionError::Denied(format!(
            "failed to set Windows ACLs for {}: {}",
            path.display(),
            stderr.trim()
        )));
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
        let path = path.as_ref();
        let acl = windows_acl(path).await?;

        // Secure == only owner/system/admin/owner-rights have allow rules.
        for rule in &acl.rules {
            if !rule
                .access_type
                .as_deref()
                .is_some_and(|t| t.eq_ignore_ascii_case("Allow"))
            {
                continue;
            }

            let Some(sid) = rule.sid.as_deref() else {
                return Ok(false);
            };

            if !is_trusted_sid(sid, acl.owner_sid.as_deref()) {
                return Ok(false);
            }
        }

        Ok(true)
    }

    #[cfg(not(any(unix, windows)))]
    {
        Ok(false)
    }
}

#[cfg(windows)]
fn is_trusted_sid(sid: &str, owner_sid: Option<&str>) -> bool {
    sid.eq_ignore_ascii_case("S-1-5-18") // SYSTEM
        || sid.eq_ignore_ascii_case("S-1-5-32-544") // Administrators
        || sid.eq_ignore_ascii_case("S-1-3-4") // OWNER RIGHTS
        || owner_sid.is_some_and(|o| sid.eq_ignore_ascii_case(o))
}

#[cfg(windows)]
#[derive(Debug, serde::Deserialize)]
struct WindowsAclInfo {
    #[serde(rename = "ownerSid")]
    owner_sid: Option<String>,
    rules: Vec<WindowsAclRule>,
}

#[cfg(windows)]
#[derive(Debug, serde::Deserialize)]
struct WindowsAclRule {
    sid: Option<String>,
    #[serde(rename = "type")]
    access_type: Option<String>,
}

#[cfg(windows)]
async fn current_user_sid() -> Result<String> {
    let script = "[System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value";
    let output = Command::new("powershell")
        .arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-Command")
        .arg(script)
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(PermissionError::Unsupported(format!(
            "failed to resolve current Windows user SID: {}",
            stderr.trim()
        )));
    }

    let sid = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if sid.is_empty() {
        return Err(PermissionError::Unsupported(
            "failed to resolve current Windows user SID".to_string(),
        ));
    }

    Ok(sid)
}

#[cfg(windows)]
async fn windows_acl(path: &Path) -> Result<WindowsAclInfo> {
    let script = r"
$target = $env:LIBRETTO_AUDIT_PATH
$acl = Get-Acl -LiteralPath $target
$ownerSid = $null
try {
  $ownerSid = (New-Object System.Security.Principal.NTAccount($acl.Owner)).Translate([System.Security.Principal.SecurityIdentifier]).Value
} catch {
  try { $ownerSid = $acl.Owner.Translate([System.Security.Principal.SecurityIdentifier]).Value } catch {}
}
$rules = foreach ($r in $acl.Access) {
  $sid = $null
  try { $sid = $r.IdentityReference.Translate([System.Security.Principal.SecurityIdentifier]).Value } catch { $sid = $r.IdentityReference.Value }
  [PSCustomObject]@{
    sid = $sid
    type = $r.AccessControlType.ToString()
  }
}
[PSCustomObject]@{
  ownerSid = $ownerSid
  rules = @($rules)
} | ConvertTo-Json -Compress -Depth 6
";

    let output = Command::new("powershell")
        .arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-Command")
        .arg(script)
        .env("LIBRETTO_AUDIT_PATH", path.as_os_str())
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(PermissionError::Unsupported(format!(
            "failed to read Windows ACL for {}: {}",
            path.display(),
            stderr.trim()
        )));
    }

    let json = String::from_utf8_lossy(&output.stdout);
    sonic_rs::from_str::<WindowsAclInfo>(&json).map_err(|e| {
        PermissionError::Unsupported(format!(
            "failed to parse ACL details for {}: {}",
            path.display(),
            e
        ))
    })
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

        #[cfg(windows)]
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
