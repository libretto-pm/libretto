//! Git repository utilities for testing VCS operations.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tempfile::{TempDir, tempdir};
use tokio::process::Command;

/// A temporary Git repository for testing.
#[derive(Debug)]
pub struct TempGitRepo {
    _dir: TempDir,
    path: PathBuf,
}

impl TempGitRepo {
    /// Create a new empty git repository.
    pub async fn new() -> Result<Self> {
        let dir = tempdir().context("Failed to create temp directory")?;
        let path = dir.path().to_path_buf();

        // Initialize git repository
        let output = Command::new("git")
            .args(["init"])
            .current_dir(&path)
            .output()
            .await
            .context("Failed to run git init")?;

        if !output.status.success() {
            anyhow::bail!(
                "git init failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        // Configure git user for commits
        Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(&path)
            .output()
            .await?;

        Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(&path)
            .output()
            .await?;

        Ok(Self { _dir: dir, path })
    }

    /// Create a repository with initial content.
    pub async fn with_content(files: &[(&str, &str)]) -> Result<Self> {
        let repo = Self::new().await?;

        for (path, content) in files {
            repo.write_file(path, content).await?;
        }

        repo.commit_all("Initial commit").await?;

        Ok(repo)
    }

    /// Create a repository simulating a PHP package.
    pub async fn php_package(name: &str, version: &str) -> Result<Self> {
        let composer_json = serde_json::json!({
            "name": name,
            "version": version,
            "type": "library",
            "autoload": {
                "psr-4": {
                    "Vendor\\Package\\": "src/"
                }
            }
        });

        let files = vec![
            (
                "composer.json",
                serde_json::to_string_pretty(&composer_json)?,
            ),
            (
                "src/Example.php",
                "<?php\nnamespace Vendor\\Package;\nclass Example {}".to_string(),
            ),
        ];

        let file_refs: Vec<(&str, &str)> = files.iter().map(|(p, c)| (*p, c.as_str())).collect();

        Self::with_content(&file_refs).await
    }

    /// Get the repository path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Write a file to the repository.
    pub async fn write_file(&self, relative_path: &str, content: &str) -> Result<()> {
        let file_path = self.path.join(relative_path);

        if let Some(parent) = file_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        tokio::fs::write(&file_path, content).await?;
        Ok(())
    }

    /// Read a file from the repository.
    pub async fn read_file(&self, relative_path: &str) -> Result<String> {
        let file_path = self.path.join(relative_path);
        Ok(tokio::fs::read_to_string(&file_path).await?)
    }

    /// Add all files and commit.
    pub async fn commit_all(&self, message: &str) -> Result<String> {
        // Add all files
        Command::new("git")
            .args(["add", "-A"])
            .current_dir(&self.path)
            .output()
            .await?;

        // Commit
        let output = Command::new("git")
            .args(["commit", "-m", message, "--allow-empty"])
            .current_dir(&self.path)
            .output()
            .await?;

        if !output.status.success() {
            anyhow::bail!(
                "git commit failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        // Get commit hash
        let output = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&self.path)
            .output()
            .await?;

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Create a tag.
    pub async fn create_tag(&self, tag: &str) -> Result<()> {
        let output = Command::new("git")
            .args(["tag", tag])
            .current_dir(&self.path)
            .output()
            .await?;

        if !output.status.success() {
            anyhow::bail!(
                "git tag failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        Ok(())
    }

    /// Create an annotated tag.
    pub async fn create_annotated_tag(&self, tag: &str, message: &str) -> Result<()> {
        let output = Command::new("git")
            .args(["tag", "-a", tag, "-m", message])
            .current_dir(&self.path)
            .output()
            .await?;

        if !output.status.success() {
            anyhow::bail!(
                "git tag failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        Ok(())
    }

    /// Create a branch.
    pub async fn create_branch(&self, branch: &str) -> Result<()> {
        let output = Command::new("git")
            .args(["branch", branch])
            .current_dir(&self.path)
            .output()
            .await?;

        if !output.status.success() {
            anyhow::bail!(
                "git branch failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        Ok(())
    }

    /// Checkout a branch or tag.
    pub async fn checkout(&self, ref_name: &str) -> Result<()> {
        let output = Command::new("git")
            .args(["checkout", ref_name])
            .current_dir(&self.path)
            .output()
            .await?;

        if !output.status.success() {
            anyhow::bail!(
                "git checkout failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        Ok(())
    }

    /// Get current HEAD commit hash.
    pub async fn head_commit(&self) -> Result<String> {
        let output = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&self.path)
            .output()
            .await?;

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Get list of all tags.
    pub async fn list_tags(&self) -> Result<Vec<String>> {
        let output = Command::new("git")
            .args(["tag", "-l"])
            .current_dir(&self.path)
            .output()
            .await?;

        let tags = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(String::from)
            .collect();

        Ok(tags)
    }

    /// Get list of all branches.
    pub async fn list_branches(&self) -> Result<Vec<String>> {
        let output = Command::new("git")
            .args(["branch", "--list", "--format=%(refname:short)"])
            .current_dir(&self.path)
            .output()
            .await?;

        let branches = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(String::from)
            .collect();

        Ok(branches)
    }

    /// Clone this repository to a new location.
    pub async fn clone_to(&self, target: &Path) -> Result<()> {
        let output = Command::new("git")
            .args([
                "clone",
                self.path.to_str().unwrap(),
                target.to_str().unwrap(),
            ])
            .output()
            .await?;

        if !output.status.success() {
            anyhow::bail!(
                "git clone failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        Ok(())
    }

    /// Add a submodule.
    pub async fn add_submodule(&self, url: &str, path: &str) -> Result<()> {
        let output = Command::new("git")
            .args(["submodule", "add", url, path])
            .current_dir(&self.path)
            .output()
            .await?;

        if !output.status.success() {
            anyhow::bail!(
                "git submodule add failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        Ok(())
    }

    /// Keep the repository (prevent cleanup on drop).
    #[must_use]
    pub fn persist(self) -> PathBuf {
        let path = self.path.clone();
        std::mem::forget(self);
        path
    }
}

/// Create a bare git repository for simulating a remote.
pub async fn create_bare_repo() -> Result<TempGitRepo> {
    let dir = tempdir()?;
    let path = dir.path().to_path_buf();

    Command::new("git")
        .args(["init", "--bare"])
        .current_dir(&path)
        .output()
        .await?;

    Ok(TempGitRepo { _dir: dir, path })
}

/// Check if git is available on the system.
pub async fn git_available() -> bool {
    Command::new("git")
        .args(["--version"])
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create_temp_repo() {
        if !git_available().await {
            return;
        }

        let repo = TempGitRepo::new().await.unwrap();
        assert!(repo.path().exists());
        assert!(repo.path().join(".git").exists());
    }

    #[tokio::test]
    async fn test_commit_and_tag() {
        if !git_available().await {
            return;
        }

        let repo = TempGitRepo::new().await.unwrap();

        repo.write_file("test.txt", "Hello, World!").await.unwrap();
        let commit = repo.commit_all("Initial commit").await.unwrap();
        assert!(!commit.is_empty());

        repo.create_tag("v1.0.0").await.unwrap();
        let tags = repo.list_tags().await.unwrap();
        assert!(tags.contains(&"v1.0.0".to_string()));
    }

    #[tokio::test]
    async fn test_php_package_repo() {
        if !git_available().await {
            return;
        }

        let repo = TempGitRepo::php_package("vendor/package", "1.0.0")
            .await
            .unwrap();

        assert!(repo.path().join("composer.json").exists());
        assert!(repo.path().join("src/Example.php").exists());

        let content = repo.read_file("composer.json").await.unwrap();
        assert!(content.contains("vendor/package"));
    }
}
