//! Mercurial (hg) operations via command-line.

use crate::error::{Result, VcsError};
use crate::types::{RepoStatus, VcsRef};
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::{debug, info};

/// Mercurial repository wrapper.
#[derive(Debug)]
pub struct HgRepository {
    /// Repository path.
    path: PathBuf,
}

impl HgRepository {
    /// Check if Mercurial is available.
    #[must_use]
    pub fn is_available() -> bool {
        Command::new("hg")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Open an existing Mercurial repository.
    ///
    /// # Errors
    /// Returns error if path is not a Mercurial repository.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();

        if !path.join(".hg").exists() {
            return Err(VcsError::NotRepository { path });
        }

        Ok(Self { path })
    }

    /// Clone a Mercurial repository.
    ///
    /// # Errors
    /// Returns error if clone fails.
    pub fn clone(url: &str, dest: &Path, reference: Option<&VcsRef>) -> Result<Self> {
        debug!(url, dest = ?dest, reference = ?reference, "hg clone");

        // Ensure destination parent exists
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| VcsError::io(parent, e))?;
        }

        let mut cmd = Command::new("hg");
        cmd.arg("clone");

        if let Some(git_ref) = reference {
            match git_ref {
                VcsRef::Branch(branch) => {
                    cmd.arg("--branch").arg(branch);
                }
                VcsRef::Tag(tag) => {
                    cmd.arg("--rev").arg(tag);
                }
                VcsRef::Commit(rev) => {
                    cmd.arg("--rev").arg(rev);
                }
                VcsRef::Default => {}
            }
        }

        cmd.arg(url).arg(dest);

        let output = cmd.output().map_err(|e| VcsError::Command {
            command: "hg clone".to_string(),
            message: e.to_string(),
            exit_code: None,
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Self::parse_hg_error(&stderr, url));
        }

        info!(url, "hg clone complete");
        Ok(Self {
            path: dest.to_path_buf(),
        })
    }

    /// Parse Mercurial error output.
    fn parse_hg_error(stderr: &str, url: &str) -> VcsError {
        let stderr_lower = stderr.to_lowercase();

        if stderr_lower.contains("not found") || stderr_lower.contains("does not exist") {
            return VcsError::RepositoryNotFound {
                url: url.to_string(),
            };
        }

        if stderr_lower.contains("authorization")
            || stderr_lower.contains("authentication")
            || stderr_lower.contains("permission denied")
        {
            return VcsError::AuthenticationFailed {
                url: url.to_string(),
                reason: stderr.to_string(),
            };
        }

        VcsError::Mercurial {
            message: stderr.to_string(),
        }
    }

    /// Get repository path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Pull and update.
    ///
    /// # Errors
    /// Returns error if pull fails.
    pub fn pull(&self, remote: Option<&str>) -> Result<()> {
        debug!(path = ?self.path, remote, "hg pull");

        let mut cmd = Command::new("hg");
        cmd.current_dir(&self.path).arg("pull").arg("--update");

        if let Some(r) = remote {
            cmd.arg(r);
        }

        let output = cmd.output().map_err(|e| VcsError::Command {
            command: "hg pull".to_string(),
            message: e.to_string(),
            exit_code: None,
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(VcsError::Mercurial {
                message: format!("pull failed: {stderr}"),
            });
        }

        Ok(())
    }

    /// Update to a specific revision.
    ///
    /// # Errors
    /// Returns error if update fails.
    pub fn update(&self, reference: &VcsRef) -> Result<()> {
        debug!(path = ?self.path, reference = ?reference, "hg update");

        let output = Command::new("hg")
            .current_dir(&self.path)
            .args(["update", reference.as_str()])
            .output()
            .map_err(|e| VcsError::Command {
                command: "hg update".to_string(),
                message: e.to_string(),
                exit_code: None,
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(VcsError::CheckoutFailed {
                reference: reference.as_str().to_string(),
                reason: stderr.to_string(),
            });
        }

        Ok(())
    }

    /// Get current changeset ID.
    ///
    /// # Errors
    /// Returns error if ID cannot be determined.
    pub fn identify(&self) -> Result<String> {
        let output = Command::new("hg")
            .current_dir(&self.path)
            .args(["identify", "-i"])
            .output()
            .map_err(|e| VcsError::Command {
                command: "hg identify".to_string(),
                message: e.to_string(),
                exit_code: None,
            })?;

        if !output.status.success() {
            return Err(VcsError::Mercurial {
                message: "failed to get changeset id".to_string(),
            });
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Get current branch.
    ///
    /// # Errors
    /// Returns error if branch cannot be determined.
    pub fn branch(&self) -> Result<String> {
        let output = Command::new("hg")
            .current_dir(&self.path)
            .arg("branch")
            .output()
            .map_err(|e| VcsError::Command {
                command: "hg branch".to_string(),
                message: e.to_string(),
                exit_code: None,
            })?;

        if !output.status.success() {
            return Err(VcsError::Mercurial {
                message: "failed to get branch".to_string(),
            });
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Get repository status.
    ///
    /// # Errors
    /// Returns error if status cannot be determined.
    pub fn status(&self) -> Result<RepoStatus> {
        let output = Command::new("hg")
            .current_dir(&self.path)
            .arg("status")
            .output()
            .map_err(|e| VcsError::Command {
                command: "hg status".to_string(),
                message: e.to_string(),
                exit_code: None,
            })?;

        let mut status = RepoStatus {
            head: self.identify().unwrap_or_default(),
            ..Default::default()
        };

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                if line.starts_with('M') || line.starts_with('A') || line.starts_with('R') {
                    status.modified += 1;
                } else if line.starts_with('?') {
                    status.untracked += 1;
                }
            }
            status.is_dirty = status.modified > 0;
        }

        Ok(status)
    }

    /// Check if repository has local modifications.
    ///
    /// # Errors
    /// Returns error if check fails.
    pub fn is_dirty(&self) -> Result<bool> {
        let output = Command::new("hg")
            .current_dir(&self.path)
            .args(["status", "-q"])
            .output()
            .map_err(|e| VcsError::Command {
                command: "hg status".to_string(),
                message: e.to_string(),
                exit_code: None,
            })?;

        Ok(!output.stdout.is_empty())
    }

    /// Check if a path is a Mercurial repository.
    #[must_use]
    pub fn is_repository(path: &Path) -> bool {
        path.join(".hg").exists()
    }

    /// Archive (export) to a destination.
    ///
    /// # Errors
    /// Returns error if archive fails.
    pub fn archive(&self, dest: &Path, revision: Option<&str>) -> Result<()> {
        let mut cmd = Command::new("hg");
        cmd.current_dir(&self.path).arg("archive");

        if let Some(rev) = revision {
            cmd.arg("--rev").arg(rev);
        }

        cmd.arg(dest);

        let output = cmd.output().map_err(|e| VcsError::Command {
            command: "hg archive".to_string(),
            message: e.to_string(),
            exit_code: None,
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(VcsError::Mercurial {
                message: format!("archive failed: {stderr}"),
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_hg_repository() {
        let temp = tempfile::tempdir().unwrap();
        assert!(!HgRepository::is_repository(temp.path()));
    }

    #[test]
    fn parse_error_not_found() {
        let err = HgRepository::parse_hg_error(
            "abort: repository https://example.com/repo not found",
            "https://example.com/repo",
        );
        assert!(matches!(err, VcsError::RepositoryNotFound { .. }));
    }
}
