//! VCS (Version Control System) source handlers.
//!
//! Supports Git, SVN, and Mercurial repositories.

use crate::error::{DownloadError, Result};
use crate::source::VcsRef;
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;
use tracing::{debug, info};
use url::Url;

/// Git repository handler.
#[derive(Debug, Clone)]
pub struct GitHandler {
    /// Shallow clone depth (None for full clone).
    depth: Option<u32>,
    /// Fetch submodules.
    recursive: bool,
}

impl Default for GitHandler {
    fn default() -> Self {
        Self {
            depth: Some(1),
            recursive: false,
        }
    }
}

impl GitHandler {
    /// Create a new Git handler.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Enable full clone (no shallow).
    #[must_use]
    pub const fn full_clone(mut self) -> Self {
        self.depth = None;
        self
    }

    /// Set clone depth.
    #[must_use]
    pub const fn with_depth(mut self, depth: u32) -> Self {
        self.depth = Some(depth);
        self
    }

    /// Enable recursive submodule fetching.
    #[must_use]
    pub const fn recursive(mut self) -> Self {
        self.recursive = true;
        self
    }

    /// Clone a repository to the destination path.
    ///
    /// # Errors
    /// Returns error if clone fails.
    pub async fn clone(&self, url: &Url, dest: &Path, reference: &VcsRef) -> Result<GitResult> {
        debug!(url = %url, dest = ?dest, ref = ?reference, "cloning git repository");

        // Ensure destination directory exists
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| DownloadError::io(parent, e))?;
        }

        // Build clone command
        let mut cmd = Command::new("git");
        cmd.arg("clone").arg("--single-branch").arg("--no-tags");

        if let Some(depth) = self.depth {
            cmd.arg("--depth").arg(depth.to_string());
        }

        // Add branch/tag for clone
        match reference {
            VcsRef::Branch(branch) => {
                cmd.arg("--branch").arg(branch);
            }
            VcsRef::Tag(tag) => {
                cmd.arg("--branch").arg(tag);
            }
            VcsRef::Commit(_) => {
                // For commits, we need a deeper clone
                if self.depth.is_some() {
                    // Remove depth for commit checkout
                    cmd.args(["--depth", "100"]);
                }
            }
        }

        if self.recursive {
            cmd.arg("--recurse-submodules");
        }

        cmd.arg("--")
            .arg(url.as_str())
            .arg(dest)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let output = cmd
            .output()
            .await
            .map_err(|e| DownloadError::Vcs(format!("failed to run git: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(DownloadError::Vcs(format!("git clone failed: {stderr}")));
        }

        // If we need to checkout a specific commit
        if let VcsRef::Commit(sha) = reference {
            self.checkout_commit(dest, sha).await?;
        }

        // Get the actual commit SHA
        let commit = self.get_head_commit(dest).await?;

        info!(url = %url, commit = %commit, "git clone complete");

        Ok(GitResult {
            path: dest.to_path_buf(),
            commit,
        })
    }

    /// Checkout a specific commit.
    async fn checkout_commit(&self, repo: &Path, sha: &str) -> Result<()> {
        // First fetch the specific commit
        let fetch_output = Command::new("git")
            .current_dir(repo)
            .args(["fetch", "--depth", "1", "origin", sha])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| DownloadError::Vcs(format!("failed to run git fetch: {e}")))?;

        if !fetch_output.status.success() {
            // Try without depth
            let fetch_output = Command::new("git")
                .current_dir(repo)
                .args(["fetch", "origin", sha])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .await
                .map_err(|e| DownloadError::Vcs(format!("failed to run git fetch: {e}")))?;

            if !fetch_output.status.success() {
                let stderr = String::from_utf8_lossy(&fetch_output.stderr);
                return Err(DownloadError::Vcs(format!("git fetch failed: {stderr}")));
            }
        }

        // Checkout the commit
        let checkout_output = Command::new("git")
            .current_dir(repo)
            .args(["checkout", sha])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| DownloadError::Vcs(format!("failed to run git checkout: {e}")))?;

        if !checkout_output.status.success() {
            let stderr = String::from_utf8_lossy(&checkout_output.stderr);
            return Err(DownloadError::Vcs(format!("git checkout failed: {stderr}")));
        }

        Ok(())
    }

    /// Get the HEAD commit SHA.
    async fn get_head_commit(&self, repo: &Path) -> Result<String> {
        let output = Command::new("git")
            .current_dir(repo)
            .args(["rev-parse", "HEAD"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| DownloadError::Vcs(format!("failed to run git rev-parse: {e}")))?;

        if !output.status.success() {
            return Err(DownloadError::Vcs("failed to get HEAD commit".into()));
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Check if git is available.
    pub async fn is_available() -> bool {
        Command::new("git")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

/// Result of a Git operation.
#[derive(Debug, Clone)]
pub struct GitResult {
    /// Path to cloned repository.
    pub path: std::path::PathBuf,
    /// Commit SHA.
    pub commit: String,
}

/// Subversion handler.
#[derive(Debug, Clone, Default)]
pub struct SvnHandler {
    /// Checkout depth.
    depth: Option<String>,
}

impl SvnHandler {
    /// Create a new SVN handler.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Checkout a repository.
    ///
    /// # Errors
    /// Returns error if checkout fails.
    pub async fn checkout(
        &self,
        url: &Url,
        dest: &Path,
        revision: Option<&str>,
    ) -> Result<SvnResult> {
        debug!(url = %url, dest = ?dest, revision = ?revision, "checking out svn repository");

        // Ensure destination directory exists
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| DownloadError::io(parent, e))?;
        }

        let mut cmd = Command::new("svn");
        cmd.arg("checkout").arg("--non-interactive");

        if let Some(rev) = revision {
            cmd.arg("-r").arg(rev);
        }

        if let Some(ref depth) = self.depth {
            cmd.arg("--depth").arg(depth);
        }

        cmd.arg(url.as_str())
            .arg(dest)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let output = cmd
            .output()
            .await
            .map_err(|e| DownloadError::Vcs(format!("failed to run svn: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(DownloadError::Vcs(format!("svn checkout failed: {stderr}")));
        }

        // Get revision info
        let revision = self.get_revision(dest).await?;

        info!(url = %url, revision = %revision, "svn checkout complete");

        Ok(SvnResult {
            path: dest.to_path_buf(),
            revision,
        })
    }

    /// Get the current revision.
    async fn get_revision(&self, repo: &Path) -> Result<String> {
        let output = Command::new("svn")
            .current_dir(repo)
            .args(["info", "--show-item", "revision"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| DownloadError::Vcs(format!("failed to run svn info: {e}")))?;

        if !output.status.success() {
            return Err(DownloadError::Vcs("failed to get svn revision".into()));
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Check if svn is available.
    pub async fn is_available() -> bool {
        Command::new("svn")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

/// Result of an SVN operation.
#[derive(Debug, Clone)]
pub struct SvnResult {
    /// Path to checked out repository.
    pub path: std::path::PathBuf,
    /// Revision number.
    pub revision: String,
}

/// Mercurial handler.
#[derive(Debug, Clone, Default)]
pub struct HgHandler;

impl HgHandler {
    /// Create a new Mercurial handler.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Clone a repository.
    ///
    /// # Errors
    /// Returns error if clone fails.
    pub async fn clone(&self, url: &Url, dest: &Path, revision: Option<&str>) -> Result<HgResult> {
        debug!(url = %url, dest = ?dest, revision = ?revision, "cloning hg repository");

        // Ensure destination directory exists
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| DownloadError::io(parent, e))?;
        }

        let mut cmd = Command::new("hg");
        cmd.arg("clone");

        if let Some(rev) = revision {
            cmd.arg("-r").arg(rev);
        }

        cmd.arg(url.as_str())
            .arg(dest)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let output = cmd
            .output()
            .await
            .map_err(|e| DownloadError::Vcs(format!("failed to run hg: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(DownloadError::Vcs(format!("hg clone failed: {stderr}")));
        }

        // Get changeset
        let changeset = self.get_changeset(dest).await?;

        info!(url = %url, changeset = %changeset, "hg clone complete");

        Ok(HgResult {
            path: dest.to_path_buf(),
            changeset,
        })
    }

    /// Get the current changeset.
    async fn get_changeset(&self, repo: &Path) -> Result<String> {
        let output = Command::new("hg")
            .current_dir(repo)
            .args(["id", "-i"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| DownloadError::Vcs(format!("failed to run hg id: {e}")))?;

        if !output.status.success() {
            return Err(DownloadError::Vcs("failed to get hg changeset".into()));
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Check if hg is available.
    pub async fn is_available() -> bool {
        Command::new("hg")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

/// Result of an Hg operation.
#[derive(Debug, Clone)]
pub struct HgResult {
    /// Path to cloned repository.
    pub path: std::path::PathBuf,
    /// Changeset ID.
    pub changeset: String,
}

/// Copy local path source.
///
/// # Errors
/// Returns error if copy fails.
pub async fn copy_path(src: &Path, dest: &Path, symlink: bool) -> Result<()> {
    debug!(src = ?src, dest = ?dest, symlink, "copying local path");

    if !src.exists() {
        return Err(DownloadError::NotFound {
            url: src.display().to_string(),
        });
    }

    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| DownloadError::io(parent, e))?;
    }

    if symlink {
        #[cfg(unix)]
        {
            tokio::fs::symlink(src, dest)
                .await
                .map_err(|e| DownloadError::io(dest, e))?;
        }
        #[cfg(windows)]
        {
            if src.is_dir() {
                tokio::fs::symlink_dir(src, dest)
                    .await
                    .map_err(|e| DownloadError::io(dest, e))?;
            } else {
                tokio::fs::symlink_file(src, dest)
                    .await
                    .map_err(|e| DownloadError::io(dest, e))?;
            }
        }
    } else {
        copy_dir_recursive(src, dest).await?;
    }

    info!(src = ?src, dest = ?dest, "copy complete");

    Ok(())
}

/// Recursively copy a directory.
async fn copy_dir_recursive(src: &Path, dest: &Path) -> Result<()> {
    if src.is_file() {
        tokio::fs::copy(src, dest)
            .await
            .map_err(|e| DownloadError::io(src, e))?;
        return Ok(());
    }

    tokio::fs::create_dir_all(dest)
        .await
        .map_err(|e| DownloadError::io(dest, e))?;

    let mut entries = tokio::fs::read_dir(src)
        .await
        .map_err(|e| DownloadError::io(src, e))?;

    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| DownloadError::io(src, e))?
    {
        let entry_path = entry.path();
        let file_name = entry.file_name();
        let dest_path = dest.join(&file_name);

        let file_type = entry
            .file_type()
            .await
            .map_err(|e| DownloadError::io(&entry_path, e))?;

        if file_type.is_dir() {
            Box::pin(copy_dir_recursive(&entry_path, &dest_path)).await?;
        } else if file_type.is_file() {
            tokio::fs::copy(&entry_path, &dest_path)
                .await
                .map_err(|e| DownloadError::io(&entry_path, e))?;
        } else if file_type.is_symlink() {
            let target = tokio::fs::read_link(&entry_path)
                .await
                .map_err(|e| DownloadError::io(&entry_path, e))?;

            #[cfg(unix)]
            {
                tokio::fs::symlink(&target, &dest_path)
                    .await
                    .map_err(|e| DownloadError::io(&dest_path, e))?;
            }
            #[cfg(windows)]
            {
                if target.is_dir() {
                    tokio::fs::symlink_dir(&target, &dest_path)
                        .await
                        .map_err(|e| DownloadError::io(&dest_path, e))?;
                } else {
                    tokio::fs::symlink_file(&target, &dest_path)
                        .await
                        .map_err(|e| DownloadError::io(&dest_path, e))?;
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_handler_default() {
        let handler = GitHandler::new();
        assert_eq!(handler.depth, Some(1));
        assert!(!handler.recursive);
    }

    #[test]
    fn git_handler_builder() {
        let handler = GitHandler::new().full_clone().recursive();
        assert!(handler.depth.is_none());
        assert!(handler.recursive);
    }

    #[tokio::test]
    async fn git_available_check() {
        // This may or may not pass depending on system
        let _ = GitHandler::is_available().await;
    }
}
