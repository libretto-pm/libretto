//! Perforce (p4) operations via command-line.

use crate::error::{Result, VcsError};
use crate::types::RepoStatus;
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::{debug, info};

/// Perforce workspace wrapper.
#[derive(Debug)]
pub struct PerforceRepository {
    /// Workspace path.
    path: PathBuf,
}

impl PerforceRepository {
    /// Check if Perforce (p4) is available.
    #[must_use]
    pub fn is_available() -> bool {
        Command::new("p4")
            .arg("-V")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Open an existing Perforce workspace.
    ///
    /// # Errors
    /// Returns error if path is not a Perforce workspace.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();

        // Check if this is a Perforce workspace by looking for .p4config or checking p4 info
        let is_p4 = path.join(".p4config").exists() || Self::is_p4_workspace(&path);

        if !is_p4 {
            return Err(VcsError::NotRepository { path });
        }

        Ok(Self { path })
    }

    /// Check if a path is a Perforce workspace.
    fn is_p4_workspace(path: &Path) -> bool {
        Command::new("p4")
            .current_dir(path)
            .args(["info", "-s"])
            .output()
            .map(|o| o.status.success() && !o.stdout.is_empty())
            .unwrap_or(false)
    }

    /// Sync (clone equivalent) a Perforce depot path to a directory.
    ///
    /// # Arguments
    /// * `depot_path` - Perforce depot path (e.g., `//depot/project/...`)
    /// * `dest` - Local destination directory
    /// * `revision` - Optional changelist number or label
    ///
    /// # Errors
    /// Returns error if sync fails.
    pub fn sync(depot_path: &str, dest: &Path, revision: Option<&str>) -> Result<Self> {
        debug!(depot_path, dest = ?dest, revision, "p4 sync");

        // Ensure destination exists
        std::fs::create_dir_all(dest).map_err(|e| VcsError::io(dest, e))?;

        // Build the file spec with optional revision
        let file_spec = if let Some(rev) = revision {
            // Both changelist number and label use the same format
            format!("{depot_path}@{rev}")
        } else {
            depot_path.to_string()
        };

        let mut cmd = Command::new("p4");
        cmd.current_dir(dest).arg("sync").arg(&file_spec);

        let output = cmd.output().map_err(|e| VcsError::Command {
            command: "p4 sync".to_string(),
            message: e.to_string(),
            exit_code: None,
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Self::parse_p4_error(&stderr, depot_path));
        }

        info!(depot_path, "p4 sync complete");
        Ok(Self {
            path: dest.to_path_buf(),
        })
    }

    /// Clone a Perforce depot to a new workspace.
    ///
    /// This is a convenience method that creates a client workspace and syncs.
    ///
    /// # Errors
    /// Returns error if clone fails.
    pub fn clone(depot_path: &str, dest: &Path, revision: Option<&str>) -> Result<Self> {
        debug!(depot_path, dest = ?dest, "p4 clone (sync)");

        // For Perforce, "cloning" is essentially syncing to a workspace
        // A full implementation would create a client spec, but for simplicity
        // we'll just do a sync which works with existing client configurations
        Self::sync(depot_path, dest, revision)
    }

    /// Parse Perforce error output.
    fn parse_p4_error(stderr: &str, depot_path: &str) -> VcsError {
        let stderr_lower = stderr.to_lowercase();

        if stderr_lower.contains("no such file")
            || stderr_lower.contains("file(s) not in client view")
        {
            return VcsError::RepositoryNotFound {
                url: depot_path.to_string(),
            };
        }

        if stderr_lower.contains("password")
            || stderr_lower.contains("login")
            || stderr_lower.contains("authentication")
            || stderr_lower.contains("invalid ticket")
        {
            return VcsError::AuthenticationFailed {
                url: depot_path.to_string(),
                reason: stderr.to_string(),
            };
        }

        if stderr_lower.contains("connect") || stderr_lower.contains("tcp") {
            return VcsError::CloneFailed {
                url: depot_path.to_string(),
                reason: format!("connection failed: {stderr}"),
                retryable: true,
            };
        }

        VcsError::Perforce {
            message: stderr.to_string(),
        }
    }

    /// Get workspace path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Update (sync) to latest or specific revision.
    ///
    /// # Errors
    /// Returns error if sync fails.
    pub fn update(&self, revision: Option<&str>) -> Result<()> {
        debug!(path = ?self.path, revision, "p4 sync");

        let mut cmd = Command::new("p4");
        cmd.current_dir(&self.path).arg("sync");

        if let Some(rev) = revision {
            cmd.arg(format!("...@{rev}"));
        }

        let output = cmd.output().map_err(|e| VcsError::Command {
            command: "p4 sync".to_string(),
            message: e.to_string(),
            exit_code: None,
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(VcsError::Perforce {
                message: format!("sync failed: {stderr}"),
            });
        }

        Ok(())
    }

    /// Get current changelist number.
    ///
    /// # Errors
    /// Returns error if changelist cannot be determined.
    pub fn changelist(&self) -> Result<String> {
        let output = Command::new("p4")
            .current_dir(&self.path)
            .args(["changes", "-m", "1", "...#have"])
            .output()
            .map_err(|e| VcsError::Command {
                command: "p4 changes".to_string(),
                message: e.to_string(),
                exit_code: None,
            })?;

        if !output.status.success() {
            return Err(VcsError::Perforce {
                message: "failed to get changelist".to_string(),
            });
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        // Parse "Change 12345 on ..." format
        if let Some(line) = stdout.lines().next()
            && let Some(cl) = line.strip_prefix("Change ")
            && let Some(num) = cl.split_whitespace().next()
        {
            return Ok(num.to_string());
        }

        Ok("unknown".to_string())
    }

    /// Get client/workspace name.
    ///
    /// # Errors
    /// Returns error if client name cannot be determined.
    pub fn client_name(&self) -> Result<String> {
        let output = Command::new("p4")
            .current_dir(&self.path)
            .args(["set", "P4CLIENT"])
            .output()
            .map_err(|e| VcsError::Command {
                command: "p4 set".to_string(),
                message: e.to_string(),
                exit_code: None,
            })?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        // Parse "P4CLIENT=clientname (set)" format
        if let Some(value) = stdout.strip_prefix("P4CLIENT=") {
            let name = value.split_whitespace().next().unwrap_or("unknown");
            return Ok(name.to_string());
        }

        Ok("unknown".to_string())
    }

    /// Get repository status.
    ///
    /// # Errors
    /// Returns error if status cannot be determined.
    pub fn status(&self) -> Result<RepoStatus> {
        let mut status = RepoStatus {
            head: self.changelist().unwrap_or_default(),
            ..Default::default()
        };

        // Get opened files (modified/added)
        let output = Command::new("p4")
            .current_dir(&self.path)
            .args(["opened"])
            .output()
            .map_err(|e| VcsError::Command {
                command: "p4 opened".to_string(),
                message: e.to_string(),
                exit_code: None,
            })?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            status.modified = stdout.lines().count();
            status.is_dirty = status.modified > 0;
        }

        // Check for pending changes
        let output = Command::new("p4")
            .current_dir(&self.path)
            .args([
                "changes",
                "-s",
                "pending",
                "-c",
                &self.client_name().unwrap_or_default(),
            ])
            .output();

        if let Ok(o) = output
            && o.status.success()
        {
            let stdout = String::from_utf8_lossy(&o.stdout);
            status.staged = stdout.lines().count();
        }

        Ok(status)
    }

    /// Check if workspace has opened files.
    ///
    /// # Errors
    /// Returns error if check fails.
    pub fn is_dirty(&self) -> Result<bool> {
        let output = Command::new("p4")
            .current_dir(&self.path)
            .args(["opened"])
            .output()
            .map_err(|e| VcsError::Command {
                command: "p4 opened".to_string(),
                message: e.to_string(),
                exit_code: None,
            })?;

        Ok(output.status.success() && !output.stdout.is_empty())
    }

    /// Check if a path is a Perforce workspace.
    #[must_use]
    pub fn is_repository(path: &Path) -> bool {
        path.join(".p4config").exists() || Self::is_p4_workspace(path)
    }

    /// Print (export) files from depot without workspace.
    ///
    /// # Errors
    /// Returns error if print fails.
    pub fn print_to_dir(depot_path: &str, dest: &Path, revision: Option<&str>) -> Result<()> {
        std::fs::create_dir_all(dest).map_err(|e| VcsError::io(dest, e))?;

        let file_spec = if let Some(rev) = revision {
            format!("{depot_path}@{rev}")
        } else {
            depot_path.to_string()
        };

        let mut cmd = Command::new("p4");
        cmd.current_dir(dest)
            .arg("print")
            .arg("-o")
            .arg("...")
            .arg(&file_spec);

        let output = cmd.output().map_err(|e| VcsError::Command {
            command: "p4 print".to_string(),
            message: e.to_string(),
            exit_code: None,
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(VcsError::Perforce {
                message: format!("print failed: {stderr}"),
            });
        }

        Ok(())
    }

    /// Get info about the Perforce server connection.
    ///
    /// # Errors
    /// Returns error if info cannot be retrieved.
    pub fn info(&self) -> Result<PerforceInfo> {
        let output = Command::new("p4")
            .current_dir(&self.path)
            .args(["info"])
            .output()
            .map_err(|e| VcsError::Command {
                command: "p4 info".to_string(),
                message: e.to_string(),
                exit_code: None,
            })?;

        if !output.status.success() {
            return Err(VcsError::Perforce {
                message: "failed to get info".to_string(),
            });
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut info = PerforceInfo::default();

        for line in stdout.lines() {
            if let Some((key, value)) = line.split_once(": ") {
                match key {
                    "User name" => info.user = value.to_string(),
                    "Client name" => info.client = value.to_string(),
                    "Client root" => info.root = Some(PathBuf::from(value)),
                    "Server address" => info.server = value.to_string(),
                    "Server version" => info.server_version = Some(value.to_string()),
                    _ => {}
                }
            }
        }

        Ok(info)
    }
}

/// Perforce server/client information.
#[derive(Debug, Default)]
pub struct PerforceInfo {
    /// Username.
    pub user: String,
    /// Client/workspace name.
    pub client: String,
    /// Client root path.
    pub root: Option<PathBuf>,
    /// Server address.
    pub server: String,
    /// Server version.
    pub server_version: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_p4_repository() {
        let temp = tempfile::tempdir().unwrap();
        assert!(!PerforceRepository::is_repository(temp.path()));
    }

    #[test]
    fn parse_error_not_found() {
        let err = PerforceRepository::parse_p4_error(
            "//depot/unknown/... - no such file(s)",
            "//depot/unknown/...",
        );
        assert!(matches!(err, VcsError::RepositoryNotFound { .. }));
    }

    #[test]
    fn parse_error_auth() {
        let err = PerforceRepository::parse_p4_error(
            "Perforce password (P4PASSWD) invalid or unset.",
            "//depot/project/...",
        );
        assert!(matches!(err, VcsError::AuthenticationFailed { .. }));
    }
}
