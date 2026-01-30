//! Core VCS types and abstractions.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;

/// Supported VCS types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VcsType {
    /// Git version control.
    Git,
    /// Subversion.
    Svn,
    /// Mercurial.
    Hg,
    /// Fossil.
    Fossil,
    /// Perforce.
    Perforce,
}

impl VcsType {
    /// Get the command name for this VCS.
    #[must_use]
    pub const fn command(&self) -> &'static str {
        match self {
            Self::Git => "git",
            Self::Svn => "svn",
            Self::Hg => "hg",
            Self::Fossil => "fossil",
            Self::Perforce => "p4",
        }
    }

    /// Detect VCS type from a path.
    #[must_use]
    pub fn detect(path: &std::path::Path) -> Option<Self> {
        if path.join(".git").exists()
            || (path.join("HEAD").exists() && path.join("objects").exists())
        {
            Some(Self::Git)
        } else if path.join(".svn").exists() {
            Some(Self::Svn)
        } else if path.join(".hg").exists() {
            Some(Self::Hg)
        } else if path.join(".fslckout").exists() || path.join("_FOSSIL_").exists() {
            Some(Self::Fossil)
        } else if path.join(".p4config").exists() {
            Some(Self::Perforce)
        } else {
            None
        }
    }
}

impl fmt::Display for VcsType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Git => write!(f, "git"),
            Self::Svn => write!(f, "svn"),
            Self::Hg => write!(f, "hg"),
            Self::Fossil => write!(f, "fossil"),
            Self::Perforce => write!(f, "perforce"),
        }
    }
}

impl std::str::FromStr for VcsType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "git" => Ok(Self::Git),
            "svn" | "subversion" => Ok(Self::Svn),
            "hg" | "mercurial" => Ok(Self::Hg),
            "fossil" => Ok(Self::Fossil),
            "p4" | "perforce" => Ok(Self::Perforce),
            _ => Err(format!("unknown vcs type: {s}")),
        }
    }
}

/// VCS reference (branch, tag, commit, revision).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
#[derive(Default)]
pub enum VcsRef {
    /// Branch name.
    Branch(String),
    /// Tag name.
    Tag(String),
    /// Git commit SHA or SVN/Hg revision.
    Commit(String),
    /// Default branch (HEAD, trunk, default).
    #[default]
    Default,
}

impl VcsRef {
    /// Parse a reference string intelligently.
    #[must_use]
    pub fn parse(reference: &str) -> Self {
        let reference = reference.trim();

        if reference.is_empty()
            || reference == "HEAD"
            || reference == "trunk"
            || reference == "default"
        {
            return Self::Default;
        }

        // 40-character hex is a Git SHA
        if reference.len() == 40 && reference.chars().all(|c| c.is_ascii_hexdigit()) {
            return Self::Commit(reference.to_string());
        }

        // Short SHA (7-39 hex chars)
        if reference.len() >= 7
            && reference.len() < 40
            && reference.chars().all(|c| c.is_ascii_hexdigit())
        {
            return Self::Commit(reference.to_string());
        }

        // Starts with v and followed by a digit - likely a tag
        if reference.starts_with('v') && reference[1..].starts_with(|c: char| c.is_ascii_digit()) {
            return Self::Tag(reference.to_string());
        }

        // Contains only digits and dots - likely a version tag
        if reference.chars().all(|c| c.is_ascii_digit() || c == '.') && reference.contains('.') {
            return Self::Tag(reference.to_string());
        }

        // refs/tags/ prefix
        if let Some(tag) = reference.strip_prefix("refs/tags/") {
            return Self::Tag(tag.to_string());
        }

        // refs/heads/ prefix
        if let Some(branch) = reference.strip_prefix("refs/heads/") {
            return Self::Branch(branch.to_string());
        }

        // Default to branch
        Self::Branch(reference.to_string())
    }

    /// Get the reference as a string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Branch(s) | Self::Tag(s) | Self::Commit(s) => s,
            Self::Default => "HEAD",
        }
    }

    /// Check if this is a commit reference.
    #[must_use]
    pub const fn is_commit(&self) -> bool {
        matches!(self, Self::Commit(_))
    }

    /// Check if this is a branch reference.
    #[must_use]
    pub const fn is_branch(&self) -> bool {
        matches!(self, Self::Branch(_))
    }

    /// Check if this is a tag reference.
    #[must_use]
    pub const fn is_tag(&self) -> bool {
        matches!(self, Self::Tag(_))
    }
}

impl fmt::Display for VcsRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Branch(s) => write!(f, "branch:{s}"),
            Self::Tag(s) => write!(f, "tag:{s}"),
            Self::Commit(s) => write!(f, "commit:{s}"),
            Self::Default => write!(f, "default"),
        }
    }
}

/// Clone options for VCS operations.
#[derive(Debug, Clone)]
pub struct CloneOptions {
    /// Clone depth (None for full clone).
    pub depth: Option<u32>,
    /// Enable recursive submodule initialization.
    pub recursive: bool,
    /// Sparse checkout paths (None for full checkout).
    pub sparse_paths: Option<Vec<String>>,
    /// Reference repository for object sharing.
    pub reference: Option<PathBuf>,
    /// Use single-branch clone.
    pub single_branch: bool,
    /// Enable Git LFS.
    pub lfs: bool,
    /// Enable worktree mode.
    pub worktree: bool,
    /// Timeout in seconds.
    pub timeout_secs: Option<u64>,
}

impl Default for CloneOptions {
    fn default() -> Self {
        Self {
            depth: Some(1), // Shallow clone by default for performance
            recursive: false,
            sparse_paths: None,
            reference: None,
            single_branch: true,
            lfs: false,
            worktree: false,
            timeout_secs: Some(300), // 5 minute default timeout
        }
    }
}

impl CloneOptions {
    /// Create options for a full clone.
    #[must_use]
    pub fn full() -> Self {
        Self {
            depth: None,
            single_branch: false,
            ..Self::default()
        }
    }

    /// Create options for a shallow clone.
    #[must_use]
    pub fn shallow(depth: u32) -> Self {
        Self {
            depth: Some(depth),
            single_branch: true,
            ..Self::default()
        }
    }

    /// Enable recursive submodule cloning.
    #[must_use]
    pub const fn with_submodules(mut self) -> Self {
        self.recursive = true;
        self
    }

    /// Set sparse checkout paths.
    #[must_use]
    pub fn with_sparse_paths(mut self, paths: Vec<String>) -> Self {
        self.sparse_paths = Some(paths);
        self
    }

    /// Set reference repository.
    #[must_use]
    pub fn with_reference(mut self, reference: PathBuf) -> Self {
        self.reference = Some(reference);
        self
    }

    /// Enable Git LFS.
    #[must_use]
    pub const fn with_lfs(mut self) -> Self {
        self.lfs = true;
        self
    }

    /// Set timeout.
    #[must_use]
    pub const fn with_timeout(mut self, secs: u64) -> Self {
        self.timeout_secs = Some(secs);
        self
    }

    /// Disable timeout.
    #[must_use]
    pub const fn without_timeout(mut self) -> Self {
        self.timeout_secs = None;
        self
    }
}

/// Result of a clone operation.
#[derive(Debug, Clone)]
pub struct CloneResult {
    /// Path to the cloned repository.
    pub path: PathBuf,
    /// Commit SHA of the checked out revision.
    pub commit: String,
    /// VCS type.
    pub vcs_type: VcsType,
    /// Reference that was checked out.
    pub reference: VcsRef,
}

/// Repository status information.
#[derive(Debug, Clone, Default)]
pub struct RepoStatus {
    /// Current branch or detached HEAD commit.
    pub head: String,
    /// Number of files with modifications.
    pub modified: usize,
    /// Number of staged files.
    pub staged: usize,
    /// Number of untracked files.
    pub untracked: usize,
    /// Number of commits ahead of remote.
    pub ahead: usize,
    /// Number of commits behind remote.
    pub behind: usize,
    /// Whether there are uncommitted changes.
    pub is_dirty: bool,
    /// Whether the repository has a remote configured.
    pub has_remote: bool,
    /// List of modified file paths.
    pub modified_files: Vec<PathBuf>,
    /// List of staged file paths.
    pub staged_files: Vec<PathBuf>,
    /// List of untracked file paths.
    pub untracked_files: Vec<PathBuf>,
}

impl RepoStatus {
    /// Check if repository is clean (no modifications).
    #[must_use]
    pub const fn is_clean(&self) -> bool {
        self.modified == 0 && self.staged == 0 && self.untracked == 0
    }

    /// Check if repository has unpushed commits.
    #[must_use]
    pub const fn has_unpushed(&self) -> bool {
        self.ahead > 0
    }

    /// Check if repository is behind remote.
    #[must_use]
    pub const fn is_behind(&self) -> bool {
        self.behind > 0
    }
}

/// Credentials for VCS authentication.
#[derive(Debug, Clone, Default)]
pub enum VcsCredentials {
    /// Username and password (or token).
    UserPass {
        /// Username.
        username: String,
        /// Password or access token.
        password: String,
    },
    /// SSH key authentication.
    SshKey {
        /// Path to private key file.
        private_key: PathBuf,
        /// Optional passphrase.
        passphrase: Option<String>,
    },
    /// SSH agent.
    SshAgent,
    /// Git credential helper.
    CredentialHelper {
        /// Helper command.
        helper: String,
    },
    /// No authentication.
    #[default]
    None,
}

impl VcsCredentials {
    /// Create username/password credentials.
    #[must_use]
    pub fn user_pass(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self::UserPass {
            username: username.into(),
            password: password.into(),
        }
    }

    /// Create SSH key credentials.
    #[must_use]
    pub const fn ssh_key(private_key: PathBuf, passphrase: Option<String>) -> Self {
        Self::SshKey {
            private_key,
            passphrase,
        }
    }

    /// Create OAuth token credentials.
    #[must_use]
    pub fn token(token: impl Into<String>) -> Self {
        Self::UserPass {
            username: "oauth2".to_string(),
            password: token.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vcs_ref_parse_commit() {
        let sha = "abc123def456abc123def456abc123def456abcd";
        assert!(matches!(VcsRef::parse(sha), VcsRef::Commit(_)));
    }

    #[test]
    fn vcs_ref_parse_short_commit() {
        assert!(matches!(VcsRef::parse("abc123d"), VcsRef::Commit(_)));
    }

    #[test]
    fn vcs_ref_parse_tag() {
        assert!(matches!(VcsRef::parse("v1.0.0"), VcsRef::Tag(_)));
        assert!(matches!(VcsRef::parse("1.2.3"), VcsRef::Tag(_)));
    }

    #[test]
    fn vcs_ref_parse_branch() {
        assert!(matches!(VcsRef::parse("main"), VcsRef::Branch(_)));
        assert!(matches!(VcsRef::parse("feature/test"), VcsRef::Branch(_)));
    }

    #[test]
    fn vcs_ref_parse_default() {
        assert!(matches!(VcsRef::parse("HEAD"), VcsRef::Default));
        assert!(matches!(VcsRef::parse(""), VcsRef::Default));
    }

    #[test]
    fn vcs_type_detect() {
        let temp = tempfile::tempdir().unwrap();
        assert!(VcsType::detect(temp.path()).is_none());

        std::fs::create_dir(temp.path().join(".git")).unwrap();
        assert_eq!(VcsType::detect(temp.path()), Some(VcsType::Git));
    }

    #[test]
    fn clone_options_default() {
        let opts = CloneOptions::default();
        assert_eq!(opts.depth, Some(1));
        assert!(opts.single_branch);
        assert!(!opts.recursive);
    }

    #[test]
    fn clone_options_full() {
        let opts = CloneOptions::full();
        assert!(opts.depth.is_none());
        assert!(!opts.single_branch);
    }
}
