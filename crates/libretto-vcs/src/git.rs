//! High-performance Git operations using gitoxide (gix).
//!
//! This module provides Git operations with:
//! - Shallow clones by default for performance
//! - Full clones for source installations
//! - Support for all protocols (HTTPS, SSH, Git, File)
//! - Submodule handling
//! - Sparse checkout support
//! - LFS support via fallback to CLI
//! - Status checking and diff detection

use crate::credentials::CredentialManager;
use crate::error::{Result, VcsError};
use crate::types::{CloneOptions, CloneResult, RepoStatus, VcsCredentials, VcsRef, VcsType};
use crate::url::VcsUrl;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use tracing::{debug, info, trace, warn};

/// Git repository wrapper for high-level operations.
#[derive(Debug)]
pub struct GitRepository {
    /// Repository path.
    path: PathBuf,
    /// Credential manager.
    credentials: Arc<CredentialManager>,
}

impl GitRepository {
    /// Open an existing Git repository.
    ///
    /// # Errors
    /// Returns error if path is not a Git repository.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();

        if !Self::is_repository(&path) {
            return Err(VcsError::NotRepository { path });
        }

        Ok(Self {
            path,
            credentials: Arc::new(CredentialManager::new()),
        })
    }

    /// Open with custom credential manager.
    pub fn open_with_credentials(
        path: impl AsRef<Path>,
        credentials: Arc<CredentialManager>,
    ) -> Result<Self> {
        let path = path.as_ref().to_path_buf();

        if !Self::is_repository(&path) {
            return Err(VcsError::NotRepository { path });
        }

        Ok(Self { path, credentials })
    }

    /// Clone a repository.
    ///
    /// # Errors
    /// Returns error if clone fails.
    pub fn clone(
        url: &VcsUrl,
        dest: &Path,
        reference: Option<&VcsRef>,
        options: &CloneOptions,
    ) -> Result<Self> {
        Self::clone_with_credentials(
            url,
            dest,
            reference,
            options,
            Arc::new(CredentialManager::new()),
        )
    }

    /// Clone with custom credential manager.
    ///
    /// # Errors
    /// Returns error if clone fails.
    pub fn clone_with_credentials(
        url: &VcsUrl,
        dest: &Path,
        reference: Option<&VcsRef>,
        options: &CloneOptions,
        credentials: Arc<CredentialManager>,
    ) -> Result<Self> {
        debug!(
            url = %url,
            dest = ?dest,
            reference = ?reference,
            "cloning repository"
        );

        // Ensure destination directory exists
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| VcsError::io(parent, e))?;
        }

        // Use git CLI for cloning as gix blocking clone has limitations
        // with network operations in the current version
        let result = Self::clone_via_cli(url, dest, reference, options, &credentials)?;

        info!(
            url = %url,
            commit = %result.commit,
            "clone complete"
        );

        Ok(Self {
            path: dest.to_path_buf(),
            credentials,
        })
    }

    /// Clone using git CLI (more reliable for network operations).
    fn clone_via_cli(
        url: &VcsUrl,
        dest: &Path,
        reference: Option<&VcsRef>,
        options: &CloneOptions,
        credentials: &CredentialManager,
    ) -> Result<CloneResult> {
        let mut cmd = Command::new("git");
        cmd.arg("clone");

        // Depth settings
        if let Some(depth) = options.depth {
            cmd.arg("--depth").arg(depth.to_string());
        }

        // Single branch
        if options.single_branch {
            cmd.arg("--single-branch");
        }

        // Branch or tag
        if let Some(git_ref) = reference {
            match git_ref {
                VcsRef::Branch(branch) => {
                    cmd.arg("--branch").arg(branch);
                }
                VcsRef::Tag(tag) => {
                    cmd.arg("--branch").arg(tag);
                }
                VcsRef::Commit(_) | VcsRef::Default => {
                    // Will checkout after clone
                }
            }
        }

        // Recursive submodules
        if options.recursive {
            cmd.arg("--recurse-submodules");
            cmd.arg("--shallow-submodules");
        }

        // Reference repository
        if let Some(ref_repo) = &options.reference {
            cmd.arg("--reference").arg(ref_repo);
        }

        // Sparse checkout needs to be set up after clone
        // LFS needs to be handled separately

        // Set up credentials in environment if needed
        Self::setup_credentials_env(&mut cmd, url, credentials)?;

        // Add URL and destination
        let clone_url = Self::url_with_credentials(url, credentials);
        cmd.arg(&clone_url);
        cmd.arg(dest);

        // Git protocol version 2 for better performance
        cmd.env("GIT_PROTOCOL", "version=2");

        trace!(command = ?cmd, "executing git clone");

        let output = cmd.output().map_err(|e| VcsError::Command {
            command: "git clone".to_string(),
            message: e.to_string(),
            exit_code: None,
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Self::parse_git_error(&stderr, url.to_string()));
        }

        // Handle specific commit checkout
        if let Some(VcsRef::Commit(sha)) = reference {
            Self::checkout_commit_cli(dest, sha)?;
        }

        // Set up sparse checkout if requested
        if let Some(paths) = &options.sparse_paths {
            Self::setup_sparse_checkout_cli(dest, paths)?;
        }

        // Initialize LFS if enabled
        if options.lfs {
            Self::init_lfs_cli(dest)?;
        }

        // Get the HEAD commit
        let commit = Self::get_head_commit_cli(dest)?;

        Ok(CloneResult {
            path: dest.to_path_buf(),
            commit,
            vcs_type: VcsType::Git,
            reference: reference.cloned().unwrap_or_default(),
        })
    }

    /// Set up credentials in command environment.
    fn setup_credentials_env(
        cmd: &mut Command,
        url: &VcsUrl,
        credentials: &CredentialManager,
    ) -> Result<()> {
        let host = url.host.as_deref().unwrap_or("");
        let creds = credentials.get_credentials(host);

        match creds {
            VcsCredentials::UserPass { username, password } => {
                // Use GIT_ASKPASS to provide credentials
                let askpass_script = format!(
                    "#!/bin/sh\necho '{}'",
                    if std::env::args().any(|a| a.contains("username")) {
                        &username
                    } else {
                        &password
                    }
                );
                // For simplicity, we'll embed credentials in URL instead
                // The url_with_credentials function handles this
                let _ = askpass_script;
            }
            VcsCredentials::SshKey { private_key, .. } => {
                cmd.env(
                    "GIT_SSH_COMMAND",
                    format!("ssh -i {}", private_key.display()),
                );
            }
            VcsCredentials::SshAgent => {
                // SSH_AUTH_SOCK should already be set
            }
            VcsCredentials::CredentialHelper { helper } => {
                cmd.env("GIT_CREDENTIAL_HELPER", helper);
            }
            VcsCredentials::None => {}
        }

        Ok(())
    }

    /// Get URL with credentials embedded (for HTTPS).
    fn url_with_credentials(url: &VcsUrl, credentials: &CredentialManager) -> String {
        let host = url.host.as_deref().unwrap_or("");
        let creds = credentials.get_credentials(host);

        match creds {
            VcsCredentials::UserPass { username, password } => {
                if url.protocol == crate::url::GitProtocol::Https {
                    // Embed credentials in HTTPS URL
                    if let Ok(mut parsed) = url::Url::parse(&url.normalized) {
                        let _ = parsed.set_username(&username);
                        let _ = parsed.set_password(Some(&password));
                        return parsed.to_string();
                    }
                }
                url.normalized.clone()
            }
            _ => url.normalized.clone(),
        }
    }

    /// Checkout a specific commit.
    fn checkout_commit_cli(repo_path: &Path, sha: &str) -> Result<()> {
        // First try to fetch the specific commit
        let output = Command::new("git")
            .current_dir(repo_path)
            .args(["fetch", "--depth", "1", "origin", sha])
            .output()
            .map_err(|e| VcsError::Command {
                command: "git fetch".to_string(),
                message: e.to_string(),
                exit_code: None,
            })?;

        if !output.status.success() {
            // Fetch the full history if shallow fetch failed
            let output = Command::new("git")
                .current_dir(repo_path)
                .args(["fetch", "--unshallow"])
                .output()
                .map_err(|e| VcsError::Command {
                    command: "git fetch --unshallow".to_string(),
                    message: e.to_string(),
                    exit_code: None,
                })?;

            if !output.status.success() {
                // Continue anyway, the commit might be available
                trace!("unshallow fetch failed, continuing");
            }
        }

        // Checkout the commit
        let output = Command::new("git")
            .current_dir(repo_path)
            .args(["checkout", sha])
            .output()
            .map_err(|e| VcsError::Command {
                command: "git checkout".to_string(),
                message: e.to_string(),
                exit_code: None,
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(VcsError::CheckoutFailed {
                reference: sha.to_string(),
                reason: stderr.to_string(),
            });
        }

        Ok(())
    }

    /// Set up sparse checkout.
    fn setup_sparse_checkout_cli(repo_path: &Path, paths: &[String]) -> Result<()> {
        // Initialize sparse checkout
        let output = Command::new("git")
            .current_dir(repo_path)
            .args(["sparse-checkout", "init", "--cone"])
            .output()
            .map_err(|e| VcsError::Command {
                command: "git sparse-checkout init".to_string(),
                message: e.to_string(),
                exit_code: None,
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(VcsError::SparseCheckout {
                message: stderr.to_string(),
            });
        }

        // Set the paths
        let mut cmd = Command::new("git");
        cmd.current_dir(repo_path).args(["sparse-checkout", "set"]);
        for path in paths {
            cmd.arg(path);
        }

        let output = cmd.output().map_err(|e| VcsError::Command {
            command: "git sparse-checkout set".to_string(),
            message: e.to_string(),
            exit_code: None,
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(VcsError::SparseCheckout {
                message: stderr.to_string(),
            });
        }

        debug!(paths = ?paths, "sparse checkout configured");
        Ok(())
    }

    /// Initialize Git LFS.
    fn init_lfs_cli(repo_path: &Path) -> Result<()> {
        // Check if git-lfs is installed
        let output = Command::new("git").args(["lfs", "version"]).output();

        if output.is_err() || !output.expect("checked").status.success() {
            warn!("git-lfs not installed, skipping LFS initialization");
            return Ok(());
        }

        let output = Command::new("git")
            .current_dir(repo_path)
            .args(["lfs", "install", "--local"])
            .output()
            .map_err(|e| VcsError::Command {
                command: "git lfs install".to_string(),
                message: e.to_string(),
                exit_code: None,
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(VcsError::Lfs {
                message: stderr.to_string(),
            });
        }

        // Pull LFS files
        let output = Command::new("git")
            .current_dir(repo_path)
            .args(["lfs", "pull"])
            .output()
            .map_err(|e| VcsError::Command {
                command: "git lfs pull".to_string(),
                message: e.to_string(),
                exit_code: None,
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!(error = %stderr, "git lfs pull failed");
        }

        debug!("git lfs initialized");
        Ok(())
    }

    /// Get HEAD commit via CLI.
    fn get_head_commit_cli(repo_path: &Path) -> Result<String> {
        let output = Command::new("git")
            .current_dir(repo_path)
            .args(["rev-parse", "HEAD"])
            .output()
            .map_err(|e| VcsError::Command {
                command: "git rev-parse HEAD".to_string(),
                message: e.to_string(),
                exit_code: None,
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(VcsError::git(format!("failed to get HEAD: {stderr}")));
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Parse git error output to create appropriate `VcsError`.
    fn parse_git_error(stderr: &str, url: String) -> VcsError {
        let stderr_lower = stderr.to_lowercase();

        if stderr_lower.contains("repository not found")
            || stderr_lower.contains("does not exist")
            || stderr_lower.contains("not found")
        {
            return VcsError::RepositoryNotFound { url };
        }

        if stderr_lower.contains("authentication failed")
            || stderr_lower.contains("permission denied")
            || stderr_lower.contains("access denied")
            || stderr_lower.contains("invalid credentials")
        {
            return VcsError::AuthenticationFailed {
                url,
                reason: stderr.to_string(),
            };
        }

        if stderr_lower.contains("host key verification failed") {
            let host = url.split('/').nth(2).unwrap_or("unknown").to_string();
            return VcsError::HostKeyVerification { host };
        }

        if stderr_lower.contains("ssl") || stderr_lower.contains("certificate") {
            let host = url.split('/').nth(2).unwrap_or("unknown").to_string();
            return VcsError::Certificate {
                host,
                reason: stderr.to_string(),
            };
        }

        if stderr_lower.contains("timeout") || stderr_lower.contains("timed out") {
            return VcsError::Timeout { seconds: 0 };
        }

        let retryable = stderr_lower.contains("network")
            || stderr_lower.contains("connection")
            || stderr_lower.contains("temporary");

        VcsError::CloneFailed {
            url,
            reason: stderr.to_string(),
            retryable,
        }
    }

    /// Get repository path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Check if a path is a Git repository.
    #[must_use]
    pub fn is_repository(path: &Path) -> bool {
        path.join(".git").exists() || (path.join("HEAD").exists() && path.join("objects").exists())
    }

    /// Get the current HEAD commit.
    ///
    /// # Errors
    /// Returns error if HEAD cannot be resolved.
    pub fn head_commit(&self) -> Result<String> {
        Self::get_head_commit_cli(&self.path)
    }

    /// Get the current branch name.
    ///
    /// # Errors
    /// Returns error if branch cannot be determined.
    pub fn current_branch(&self) -> Result<Option<String>> {
        let output = Command::new("git")
            .current_dir(&self.path)
            .args(["symbolic-ref", "--short", "HEAD"])
            .output()
            .map_err(|e| VcsError::Command {
                command: "git symbolic-ref".to_string(),
                message: e.to_string(),
                exit_code: None,
            })?;

        if output.status.success() {
            Ok(Some(
                String::from_utf8_lossy(&output.stdout).trim().to_string(),
            ))
        } else {
            // Detached HEAD
            Ok(None)
        }
    }

    /// Checkout a reference.
    ///
    /// # Errors
    /// Returns error if checkout fails.
    pub fn checkout(&self, reference: &VcsRef) -> Result<()> {
        debug!(reference = %reference, "checking out");

        match reference {
            VcsRef::Commit(sha) => Self::checkout_commit_cli(&self.path, sha),
            VcsRef::Branch(name) | VcsRef::Tag(name) => {
                let output = Command::new("git")
                    .current_dir(&self.path)
                    .args(["checkout", name])
                    .output()
                    .map_err(|e| VcsError::Command {
                        command: "git checkout".to_string(),
                        message: e.to_string(),
                        exit_code: None,
                    })?;

                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    return Err(VcsError::CheckoutFailed {
                        reference: name.clone(),
                        reason: stderr.to_string(),
                    });
                }

                Ok(())
            }
            VcsRef::Default => {
                // Checkout default branch
                let output = Command::new("git")
                    .current_dir(&self.path)
                    .args(["checkout", "-"])
                    .output()
                    .map_err(|e| VcsError::Command {
                        command: "git checkout".to_string(),
                        message: e.to_string(),
                        exit_code: None,
                    })?;

                if !output.status.success() {
                    // Try main/master
                    for branch in &["main", "master"] {
                        let output = Command::new("git")
                            .current_dir(&self.path)
                            .args(["checkout", branch])
                            .output();
                        if output.is_ok() && output.expect("checked").status.success() {
                            return Ok(());
                        }
                    }
                }

                Ok(())
            }
        }
    }

    /// Get the URL of a remote.
    fn get_remote_url(&self, remote: &str) -> Result<Option<String>> {
        let output = Command::new("git")
            .current_dir(&self.path)
            .args(["remote", "get-url", remote])
            .output()
            .map_err(|e| VcsError::Command {
                command: "git remote get-url".to_string(),
                message: e.to_string(),
                exit_code: None,
            })?;

        if output.status.success() {
            Ok(Some(
                String::from_utf8_lossy(&output.stdout).trim().to_string(),
            ))
        } else {
            Ok(None)
        }
    }

    /// Set up SSH command with credentials if needed.
    fn setup_ssh_command(&self) -> Option<String> {
        // Get credentials for any configured remote
        if let Ok(Some(url)) = self.get_remote_url("origin")
            && let Ok(vcs_url) = VcsUrl::parse(&url)
        {
            let host = vcs_url.host.as_deref().unwrap_or("");
            let creds = self.credentials.get_credentials(host);

            if let VcsCredentials::SshKey { private_key, .. } = creds {
                return Some(format!("ssh -i {}", private_key.display()));
            }
        }
        None
    }

    /// Fetch from remote.
    ///
    /// # Errors
    /// Returns error if fetch fails.
    pub fn fetch(&self, remote: &str) -> Result<()> {
        debug!(remote, "fetching");

        let mut cmd = Command::new("git");
        cmd.current_dir(&self.path)
            .args(["fetch", remote])
            .env("GIT_PROTOCOL", "version=2");

        // Apply SSH credentials if available
        if let Some(ssh_cmd) = self.setup_ssh_command() {
            cmd.env("GIT_SSH_COMMAND", ssh_cmd);
        }

        let output = cmd.output().map_err(|e| VcsError::Command {
            command: "git fetch".to_string(),
            message: e.to_string(),
            exit_code: None,
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(VcsError::FetchFailed {
                remote: remote.to_string(),
                reason: stderr.to_string(),
                retryable: true,
            });
        }

        info!(remote, "fetch complete");
        Ok(())
    }

    /// Fetch with pruning.
    ///
    /// # Errors
    /// Returns error if fetch fails.
    pub fn fetch_prune(&self, remote: &str) -> Result<()> {
        let mut cmd = Command::new("git");
        cmd.current_dir(&self.path)
            .args(["fetch", "--prune", remote])
            .env("GIT_PROTOCOL", "version=2");

        // Apply SSH credentials if available
        if let Some(ssh_cmd) = self.setup_ssh_command() {
            cmd.env("GIT_SSH_COMMAND", ssh_cmd);
        }

        let output = cmd.output().map_err(|e| VcsError::Command {
            command: "git fetch --prune".to_string(),
            message: e.to_string(),
            exit_code: None,
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(VcsError::FetchFailed {
                remote: remote.to_string(),
                reason: stderr.to_string(),
                retryable: true,
            });
        }

        Ok(())
    }

    /// Pull from remote.
    ///
    /// # Errors
    /// Returns error if pull fails.
    pub fn pull(&self, remote: &str, branch: &str) -> Result<()> {
        debug!(remote, branch, "pulling");

        let mut cmd = Command::new("git");
        cmd.current_dir(&self.path)
            .args(["pull", remote, branch])
            .env("GIT_PROTOCOL", "version=2");

        // Apply SSH credentials if available
        if let Some(ssh_cmd) = self.setup_ssh_command() {
            cmd.env("GIT_SSH_COMMAND", ssh_cmd);
        }

        let output = cmd.output().map_err(|e| VcsError::Command {
            command: "git pull".to_string(),
            message: e.to_string(),
            exit_code: None,
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("conflict") {
                return Err(VcsError::MergeConflict {
                    path: self.path.clone(),
                });
            }
            return Err(VcsError::git(format!("pull failed: {stderr}")));
        }

        Ok(())
    }

    /// Get repository status.
    ///
    /// # Errors
    /// Returns error if status cannot be determined.
    pub fn status(&self) -> Result<RepoStatus> {
        let mut status = RepoStatus {
            head: self.head_commit().unwrap_or_else(|_| "unknown".to_string()),
            ..Default::default()
        };

        // Get porcelain status
        let output = Command::new("git")
            .current_dir(&self.path)
            .args(["status", "--porcelain=v1"])
            .output()
            .map_err(|e| VcsError::Command {
                command: "git status".to_string(),
                message: e.to_string(),
                exit_code: None,
            })?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                if line.len() < 3 {
                    continue;
                }
                let (status_chars, path) = line.split_at(3);
                let path = PathBuf::from(path.trim());

                match &status_chars[..2] {
                    "??" => {
                        status.untracked += 1;
                        status.untracked_files.push(path);
                    }
                    s if s.starts_with(' ') => {
                        status.modified += 1;
                        status.modified_files.push(path);
                    }
                    s if !s.trim().is_empty() => {
                        status.staged += 1;
                        status.staged_files.push(path);
                    }
                    _ => {}
                }
            }
        }

        status.is_dirty = status.modified > 0 || status.staged > 0;

        // Get ahead/behind count
        let output = Command::new("git")
            .current_dir(&self.path)
            .args(["rev-list", "--left-right", "--count", "@{upstream}...HEAD"])
            .output();

        if let Ok(output) = output
            && output.status.success()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let parts: Vec<&str> = stdout.split_whitespace().collect();
            if parts.len() == 2 {
                status.behind = parts[0].parse().unwrap_or(0);
                status.ahead = parts[1].parse().unwrap_or(0);
                status.has_remote = true;
            }
        }

        Ok(status)
    }

    /// Check if repository has local modifications.
    ///
    /// # Errors
    /// Returns error if check fails.
    pub fn is_dirty(&self) -> Result<bool> {
        let output = Command::new("git")
            .current_dir(&self.path)
            .args(["status", "--porcelain"])
            .output()
            .map_err(|e| VcsError::Command {
                command: "git status".to_string(),
                message: e.to_string(),
                exit_code: None,
            })?;

        Ok(!output.stdout.is_empty())
    }

    /// Initialize submodules.
    ///
    /// # Errors
    /// Returns error if submodule init fails.
    pub fn init_submodules(&self) -> Result<()> {
        debug!("initializing submodules");

        let output = Command::new("git")
            .current_dir(&self.path)
            .args(["submodule", "update", "--init", "--recursive"])
            .output()
            .map_err(|e| VcsError::Command {
                command: "git submodule update".to_string(),
                message: e.to_string(),
                exit_code: None,
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(VcsError::Submodule {
                message: stderr.to_string(),
                submodule_path: None,
            });
        }

        Ok(())
    }

    /// List submodules.
    ///
    /// # Errors
    /// Returns error if listing fails.
    pub fn list_submodules(&self) -> Result<Vec<SubmoduleInfo>> {
        let output = Command::new("git")
            .current_dir(&self.path)
            .args(["submodule", "status"])
            .output()
            .map_err(|e| VcsError::Command {
                command: "git submodule status".to_string(),
                message: e.to_string(),
                exit_code: None,
            })?;

        if !output.status.success() {
            return Ok(Vec::new());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut submodules = Vec::new();

        for line in stdout.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            // Format: " sha1 path (describe)"
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                let sha = parts[0].trim_start_matches(['-', '+', ' ']);
                let path = parts[1];
                submodules.push(SubmoduleInfo {
                    path: PathBuf::from(path),
                    commit: sha.to_string(),
                    initialized: !line.starts_with('-'),
                });
            }
        }

        Ok(submodules)
    }

    /// Create a worktree.
    ///
    /// # Errors
    /// Returns error if worktree creation fails.
    pub fn create_worktree(&self, path: &Path, reference: &VcsRef) -> Result<()> {
        let output = Command::new("git")
            .current_dir(&self.path)
            .args([
                "worktree",
                "add",
                &path.to_string_lossy(),
                reference.as_str(),
            ])
            .output()
            .map_err(|e| VcsError::Command {
                command: "git worktree add".to_string(),
                message: e.to_string(),
                exit_code: None,
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(VcsError::Worktree {
                message: stderr.to_string(),
            });
        }

        Ok(())
    }

    /// Remove a worktree.
    ///
    /// # Errors
    /// Returns error if worktree removal fails.
    pub fn remove_worktree(&self, path: &Path) -> Result<()> {
        let output = Command::new("git")
            .current_dir(&self.path)
            .args(["worktree", "remove", &path.to_string_lossy()])
            .output()
            .map_err(|e| VcsError::Command {
                command: "git worktree remove".to_string(),
                message: e.to_string(),
                exit_code: None,
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(VcsError::Worktree {
                message: stderr.to_string(),
            });
        }

        Ok(())
    }

    /// Verify a signed commit.
    ///
    /// # Errors
    /// Returns error if verification fails.
    pub fn verify_commit(&self, commit: &str) -> Result<bool> {
        let output = Command::new("git")
            .current_dir(&self.path)
            .args(["verify-commit", commit])
            .output()
            .map_err(|e| VcsError::Command {
                command: "git verify-commit".to_string(),
                message: e.to_string(),
                exit_code: None,
            })?;

        Ok(output.status.success())
    }

    /// Get diff between two references.
    ///
    /// # Errors
    /// Returns error if diff fails.
    pub fn diff(&self, from: &str, to: &str) -> Result<String> {
        let output = Command::new("git")
            .current_dir(&self.path)
            .args(["diff", from, to])
            .output()
            .map_err(|e| VcsError::Command {
                command: "git diff".to_string(),
                message: e.to_string(),
                exit_code: None,
            })?;

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// Check if repository needs update (is behind remote).
    ///
    /// # Errors
    /// Returns error if check fails.
    pub fn needs_update(&self, remote: &str, branch: &str) -> Result<bool> {
        // Fetch first to get latest refs
        self.fetch(remote)?;

        let output = Command::new("git")
            .current_dir(&self.path)
            .args(["rev-list", "--count", &format!("HEAD..{remote}/{branch}")])
            .output()
            .map_err(|e| VcsError::Command {
                command: "git rev-list".to_string(),
                message: e.to_string(),
                exit_code: None,
            })?;

        if output.status.success() {
            let count: usize = String::from_utf8_lossy(&output.stdout)
                .trim()
                .parse()
                .unwrap_or(0);
            Ok(count > 0)
        } else {
            Ok(false)
        }
    }
}

/// Submodule information.
#[derive(Debug, Clone)]
pub struct SubmoduleInfo {
    /// Submodule path relative to repository root.
    pub path: PathBuf,
    /// Current commit.
    pub commit: String,
    /// Whether submodule is initialized.
    pub initialized: bool,
}

/// Builder for Git clone operations with fluent API.
#[derive(Debug)]
pub struct GitCloneBuilder {
    url: VcsUrl,
    dest: PathBuf,
    reference: Option<VcsRef>,
    options: CloneOptions,
    credentials: Arc<CredentialManager>,
}

impl GitCloneBuilder {
    /// Create a new clone builder.
    #[must_use]
    pub fn new(url: VcsUrl, dest: PathBuf) -> Self {
        Self {
            url,
            dest,
            reference: None,
            options: CloneOptions::default(),
            credentials: Arc::new(CredentialManager::new()),
        }
    }

    /// Set the reference to checkout.
    #[must_use]
    pub fn reference(mut self, reference: VcsRef) -> Self {
        self.reference = Some(reference);
        self
    }

    /// Set clone depth.
    #[must_use]
    pub const fn depth(mut self, depth: u32) -> Self {
        self.options.depth = Some(depth);
        self
    }

    /// Enable full clone.
    #[must_use]
    pub const fn full_clone(mut self) -> Self {
        self.options.depth = None;
        self.options.single_branch = false;
        self
    }

    /// Enable recursive submodules.
    #[must_use]
    pub const fn recursive(mut self) -> Self {
        self.options.recursive = true;
        self
    }

    /// Set sparse checkout paths.
    #[must_use]
    pub fn sparse(mut self, paths: Vec<String>) -> Self {
        self.options.sparse_paths = Some(paths);
        self
    }

    /// Set reference repository.
    #[must_use]
    pub fn with_reference_repo(mut self, path: PathBuf) -> Self {
        self.options.reference = Some(path);
        self
    }

    /// Enable LFS.
    #[must_use]
    pub const fn with_lfs(mut self) -> Self {
        self.options.lfs = true;
        self
    }

    /// Set timeout.
    #[must_use]
    pub const fn timeout(mut self, secs: u64) -> Self {
        self.options.timeout_secs = Some(secs);
        self
    }

    /// Set credential manager.
    #[must_use]
    pub fn credentials(mut self, credentials: Arc<CredentialManager>) -> Self {
        self.credentials = credentials;
        self
    }

    /// Execute the clone.
    ///
    /// # Errors
    /// Returns error if clone fails.
    pub fn execute(self) -> Result<GitRepository> {
        GitRepository::clone_with_credentials(
            &self.url,
            &self.dest,
            self.reference.as_ref(),
            &self.options,
            self.credentials,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_not_repository() {
        let temp = tempfile::tempdir().unwrap();
        assert!(!GitRepository::is_repository(temp.path()));
    }

    #[test]
    fn clone_options_default() {
        let opts = CloneOptions::default();
        assert_eq!(opts.depth, Some(1));
        assert!(opts.single_branch);
    }

    #[test]
    fn parse_git_error_not_found() {
        let err = GitRepository::parse_git_error(
            "fatal: repository 'https://github.com/foo/bar' not found",
            "https://github.com/foo/bar".to_string(),
        );
        assert!(matches!(err, VcsError::RepositoryNotFound { .. }));
    }

    #[test]
    fn parse_git_error_auth() {
        let err = GitRepository::parse_git_error(
            "fatal: Authentication failed for 'https://github.com/foo/bar'",
            "https://github.com/foo/bar".to_string(),
        );
        assert!(matches!(err, VcsError::AuthenticationFailed { .. }));
    }
}
