//! SVN (Subversion) operations via command-line.

use crate::error::{Result, VcsError};
use crate::types::RepoStatus;
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::{debug, info};

/// SVN repository wrapper.
#[derive(Debug)]
pub struct SvnRepository {
    /// Repository path.
    path: PathBuf,
}

impl SvnRepository {
    /// Check if SVN is available.
    #[must_use]
    pub fn is_available() -> bool {
        Command::new("svn")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Open an existing SVN working copy.
    ///
    /// # Errors
    /// Returns error if path is not an SVN working copy.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();

        if !path.join(".svn").exists() {
            return Err(VcsError::NotRepository { path });
        }

        Ok(Self { path })
    }

    /// Checkout (clone) an SVN repository.
    ///
    /// # Errors
    /// Returns error if checkout fails.
    pub fn checkout(url: &str, dest: &Path, revision: Option<&str>) -> Result<Self> {
        debug!(url, dest = ?dest, revision, "svn checkout");

        // Ensure destination parent exists
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| VcsError::io(parent, e))?;
        }

        let mut cmd = Command::new("svn");
        cmd.arg("checkout");

        if let Some(rev) = revision {
            cmd.arg("-r").arg(rev);
        }

        cmd.arg(url).arg(dest);

        let output = cmd.output().map_err(|e| VcsError::Command {
            command: "svn checkout".to_string(),
            message: e.to_string(),
            exit_code: None,
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Self::parse_svn_error(&stderr, url));
        }

        info!(url, "svn checkout complete");
        Ok(Self {
            path: dest.to_path_buf(),
        })
    }

    /// Parse SVN error output.
    fn parse_svn_error(stderr: &str, url: &str) -> VcsError {
        let stderr_lower = stderr.to_lowercase();

        if stderr_lower.contains("not found") || stderr_lower.contains("doesn't exist") {
            return VcsError::RepositoryNotFound {
                url: url.to_string(),
            };
        }

        if stderr_lower.contains("authorization failed")
            || stderr_lower.contains("authentication failed")
        {
            return VcsError::AuthenticationFailed {
                url: url.to_string(),
                reason: stderr.to_string(),
            };
        }

        VcsError::Svn {
            message: stderr.to_string(),
        }
    }

    /// Get repository path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Update to latest or specific revision.
    ///
    /// # Errors
    /// Returns error if update fails.
    pub fn update(&self, revision: Option<&str>) -> Result<()> {
        debug!(path = ?self.path, revision, "svn update");

        let mut cmd = Command::new("svn");
        cmd.current_dir(&self.path).arg("update");

        if let Some(rev) = revision {
            cmd.arg("-r").arg(rev);
        }

        let output = cmd.output().map_err(|e| VcsError::Command {
            command: "svn update".to_string(),
            message: e.to_string(),
            exit_code: None,
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(VcsError::Svn {
                message: format!("update failed: {stderr}"),
            });
        }

        Ok(())
    }

    /// Switch to a different URL/branch.
    ///
    /// # Errors
    /// Returns error if switch fails.
    pub fn switch(&self, url: &str) -> Result<()> {
        let output = Command::new("svn")
            .current_dir(&self.path)
            .args(["switch", url])
            .output()
            .map_err(|e| VcsError::Command {
                command: "svn switch".to_string(),
                message: e.to_string(),
                exit_code: None,
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(VcsError::Svn {
                message: format!("switch failed: {stderr}"),
            });
        }

        Ok(())
    }

    /// Get current revision.
    ///
    /// # Errors
    /// Returns error if revision cannot be determined.
    pub fn revision(&self) -> Result<String> {
        let output = Command::new("svn")
            .current_dir(&self.path)
            .args(["info", "--show-item", "revision"])
            .output()
            .map_err(|e| VcsError::Command {
                command: "svn info".to_string(),
                message: e.to_string(),
                exit_code: None,
            })?;

        if !output.status.success() {
            return Err(VcsError::Svn {
                message: "failed to get revision".to_string(),
            });
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Get repository URL.
    ///
    /// # Errors
    /// Returns error if URL cannot be determined.
    pub fn url(&self) -> Result<String> {
        let output = Command::new("svn")
            .current_dir(&self.path)
            .args(["info", "--show-item", "url"])
            .output()
            .map_err(|e| VcsError::Command {
                command: "svn info".to_string(),
                message: e.to_string(),
                exit_code: None,
            })?;

        if !output.status.success() {
            return Err(VcsError::Svn {
                message: "failed to get url".to_string(),
            });
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Get repository status.
    ///
    /// # Errors
    /// Returns error if status cannot be determined.
    pub fn status(&self) -> Result<RepoStatus> {
        let output = Command::new("svn")
            .current_dir(&self.path)
            .args(["status", "--xml"])
            .output()
            .map_err(|e| VcsError::Command {
                command: "svn status".to_string(),
                message: e.to_string(),
                exit_code: None,
            })?;

        let mut status = RepoStatus {
            head: self.revision().unwrap_or_default(),
            ..Default::default()
        };

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            // Simple parsing - count modified entries
            status.modified = stdout.matches("<entry").count();
            status.is_dirty = status.modified > 0;
        }

        Ok(status)
    }

    /// Check if working copy has local modifications.
    ///
    /// # Errors
    /// Returns error if check fails.
    pub fn is_dirty(&self) -> Result<bool> {
        let output = Command::new("svn")
            .current_dir(&self.path)
            .args(["status", "-q"])
            .output()
            .map_err(|e| VcsError::Command {
                command: "svn status".to_string(),
                message: e.to_string(),
                exit_code: None,
            })?;

        Ok(!output.stdout.is_empty())
    }

    /// Check if a path is an SVN working copy.
    #[must_use]
    pub fn is_repository(path: &Path) -> bool {
        path.join(".svn").exists()
    }

    /// Export (non-versioned copy) of a revision.
    ///
    /// # Errors
    /// Returns error if export fails.
    pub fn export(url: &str, dest: &Path, revision: Option<&str>) -> Result<()> {
        let mut cmd = Command::new("svn");
        cmd.arg("export");

        if let Some(rev) = revision {
            cmd.arg("-r").arg(rev);
        }

        cmd.arg(url).arg(dest);

        let output = cmd.output().map_err(|e| VcsError::Command {
            command: "svn export".to_string(),
            message: e.to_string(),
            exit_code: None,
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(VcsError::Svn {
                message: format!("export failed: {stderr}"),
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_svn_repository() {
        let temp = tempfile::tempdir().unwrap();
        assert!(!SvnRepository::is_repository(temp.path()));
    }

    #[test]
    fn parse_error_not_found() {
        let err = SvnRepository::parse_svn_error(
            "svn: E170000: URL 'https://example.com/repo' doesn't exist",
            "https://example.com/repo",
        );
        assert!(matches!(err, VcsError::RepositoryNotFound { .. }));
    }
}
