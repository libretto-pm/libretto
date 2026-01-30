//! VCS Manager for orchestrating all version control operations.
//!
//! The `VcsManager` provides a unified interface for:
//! - Automatic VCS type detection
//! - Credential management
//! - Reference caching
//! - Parallel operations

use crate::cache::ReferenceCache;
use crate::credentials::CredentialManager;
use crate::error::{Result, VcsError};
use crate::fossil::FossilRepository;
use crate::git::GitRepository;
use crate::hg::HgRepository;
use crate::parallel::{BatchCloneBuilder, CloneRequest, ParallelCloneResult, ParallelCloner};
use crate::perforce::PerforceRepository;
use crate::svn::SvnRepository;
use crate::types::{CloneOptions, CloneResult, RepoStatus, VcsRef, VcsType};
use crate::url::VcsUrl;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::debug;

/// VCS Manager for unified version control operations.
#[derive(Debug)]
pub struct VcsManager {
    /// Credential manager.
    credentials: Arc<CredentialManager>,
    /// Reference cache for Git.
    reference_cache: Option<Arc<ReferenceCache>>,
    /// Cache directory.
    cache_dir: Option<PathBuf>,
    /// Default clone options.
    default_options: CloneOptions,
    /// Maximum parallel operations.
    max_parallel: usize,
}

impl Default for VcsManager {
    fn default() -> Self {
        Self::new()
    }
}

impl VcsManager {
    /// Create a new VCS manager with default settings.
    #[must_use]
    pub fn new() -> Self {
        Self {
            credentials: Arc::new(CredentialManager::new()),
            reference_cache: None,
            cache_dir: None,
            default_options: CloneOptions::default(),
            max_parallel: num_cpus::get() * 2,
        }
    }

    /// Create with a cache directory.
    ///
    /// # Errors
    /// Returns error if cache cannot be initialized.
    pub fn with_cache(cache_dir: PathBuf) -> Result<Self> {
        let reference_cache = ReferenceCache::new(cache_dir.join("vcs-references"))?;

        Ok(Self {
            credentials: Arc::new(CredentialManager::new()),
            reference_cache: Some(Arc::new(reference_cache)),
            cache_dir: Some(cache_dir),
            default_options: CloneOptions::default(),
            max_parallel: num_cpus::get() * 2,
        })
    }

    /// Set credential manager.
    #[must_use]
    pub fn with_credentials(mut self, credentials: Arc<CredentialManager>) -> Self {
        self.credentials = credentials;
        self
    }

    /// Set default clone options.
    #[must_use]
    pub fn with_default_options(mut self, options: CloneOptions) -> Self {
        self.default_options = options;
        self
    }

    /// Set maximum parallel operations.
    #[must_use]
    pub const fn with_max_parallel(mut self, max: usize) -> Self {
        self.max_parallel = max;
        self
    }

    /// Load credentials from auth.json file.
    ///
    /// # Errors
    /// Returns error if auth.json cannot be loaded.
    pub fn load_auth_json(&self, path: &Path) -> Result<()> {
        self.credentials.load_auth_json(path)
    }

    /// Clone a repository.
    ///
    /// Automatically detects VCS type from URL.
    ///
    /// # Errors
    /// Returns error if clone fails.
    pub fn clone(&self, url: &str, dest: &Path, reference: Option<&VcsRef>) -> Result<CloneResult> {
        self.clone_with_options(url, dest, reference, &self.default_options)
    }

    /// Clone with specific options.
    ///
    /// # Errors
    /// Returns error if clone fails.
    pub fn clone_with_options(
        &self,
        url: &str,
        dest: &Path,
        reference: Option<&VcsRef>,
        options: &CloneOptions,
    ) -> Result<CloneResult> {
        let vcs_type = self.detect_vcs_type(url);

        debug!(
            url,
            dest = ?dest,
            vcs_type = %vcs_type,
            "cloning repository"
        );

        match vcs_type {
            VcsType::Git => self.clone_git(url, dest, reference, options),
            VcsType::Svn => self.clone_svn(url, dest, reference),
            VcsType::Hg => self.clone_hg(url, dest, reference),
            VcsType::Fossil => self.clone_fossil(url, dest),
            VcsType::Perforce => self.clone_perforce(url, dest, reference),
        }
    }

    /// Clone a Git repository.
    fn clone_git(
        &self,
        url: &str,
        dest: &Path,
        reference: Option<&VcsRef>,
        options: &CloneOptions,
    ) -> Result<CloneResult> {
        let vcs_url = VcsUrl::parse(url)?;

        // Apply reference cache if available
        let mut options = options.clone();
        if let Some(ref cache) = self.reference_cache
            && let Some(ref_path) = cache.get_reference(&vcs_url)
        {
            options.reference = Some(ref_path);
            debug!(url, "using reference repository");
        }

        let repo = GitRepository::clone_with_credentials(
            &vcs_url,
            dest,
            reference,
            &options,
            Arc::clone(&self.credentials),
        )?;

        let commit = repo.head_commit()?;

        Ok(CloneResult {
            path: dest.to_path_buf(),
            commit,
            vcs_type: VcsType::Git,
            reference: reference.cloned().unwrap_or_default(),
        })
    }

    /// Clone an SVN repository.
    fn clone_svn(&self, url: &str, dest: &Path, reference: Option<&VcsRef>) -> Result<CloneResult> {
        let revision = reference.map(super::types::VcsRef::as_str);
        let repo = SvnRepository::checkout(url, dest, revision)?;
        let commit = repo.revision()?;

        Ok(CloneResult {
            path: dest.to_path_buf(),
            commit,
            vcs_type: VcsType::Svn,
            reference: reference.cloned().unwrap_or_default(),
        })
    }

    /// Clone a Mercurial repository.
    fn clone_hg(&self, url: &str, dest: &Path, reference: Option<&VcsRef>) -> Result<CloneResult> {
        let repo = HgRepository::clone(url, dest, reference)?;
        let commit = repo.identify()?;

        Ok(CloneResult {
            path: dest.to_path_buf(),
            commit,
            vcs_type: VcsType::Hg,
            reference: reference.cloned().unwrap_or_default(),
        })
    }

    /// Clone a Fossil repository.
    fn clone_fossil(&self, url: &str, dest: &Path) -> Result<CloneResult> {
        let repo = FossilRepository::clone(url, dest)?;
        let status = repo.status()?;

        Ok(CloneResult {
            path: dest.to_path_buf(),
            commit: status.head,
            vcs_type: VcsType::Fossil,
            reference: VcsRef::Default,
        })
    }

    /// Clone a Perforce depot.
    fn clone_perforce(
        &self,
        url: &str,
        dest: &Path,
        reference: Option<&VcsRef>,
    ) -> Result<CloneResult> {
        let revision = reference.map(super::types::VcsRef::as_str);
        let repo = PerforceRepository::clone(url, dest, revision)?;
        let commit = repo.changelist()?;

        Ok(CloneResult {
            path: dest.to_path_buf(),
            commit,
            vcs_type: VcsType::Perforce,
            reference: reference.cloned().unwrap_or_default(),
        })
    }

    /// Clone multiple repositories in parallel.
    ///
    /// # Errors
    /// Individual errors are returned in the result.
    #[must_use]
    pub fn clone_many(&self, requests: Vec<CloneRequest>) -> ParallelCloneResult {
        let mut cloner = ParallelCloner::new()
            .with_credentials(Arc::clone(&self.credentials))
            .with_max_parallel(self.max_parallel);

        if let Some(ref cache) = self.reference_cache {
            cloner = cloner.with_reference_cache(Arc::clone(cache));
        }

        cloner.clone_all(requests)
    }

    /// Create a batch clone builder.
    #[must_use]
    pub fn batch_clone(&self) -> BatchCloneBuilder {
        let mut builder = BatchCloneBuilder::new()
            .credentials(Arc::clone(&self.credentials))
            .max_parallel(self.max_parallel);

        if let Some(ref cache) = self.reference_cache {
            builder = builder.reference_cache(Arc::clone(cache));
        }

        builder
    }

    /// Open an existing repository.
    ///
    /// Automatically detects VCS type.
    ///
    /// # Errors
    /// Returns error if repository cannot be opened.
    pub fn open(&self, path: &Path) -> Result<Box<dyn Repository>> {
        let vcs_type = VcsType::detect(path).ok_or_else(|| VcsError::NotRepository {
            path: path.to_path_buf(),
        })?;

        match vcs_type {
            VcsType::Git => {
                let repo =
                    GitRepository::open_with_credentials(path, Arc::clone(&self.credentials))?;
                Ok(Box::new(repo))
            }
            VcsType::Svn => {
                let repo = SvnRepository::open(path)?;
                Ok(Box::new(repo))
            }
            VcsType::Hg => {
                let repo = HgRepository::open(path)?;
                Ok(Box::new(repo))
            }
            VcsType::Fossil => {
                let repo = FossilRepository::open(path)?;
                Ok(Box::new(repo))
            }
            VcsType::Perforce => {
                let repo = PerforceRepository::open(path)?;
                Ok(Box::new(repo))
            }
        }
    }

    /// Detect VCS type from URL.
    #[must_use]
    pub fn detect_vcs_type(&self, url: &str) -> VcsType {
        let url_lower = url.to_lowercase();

        // Check for explicit scheme
        if url_lower.starts_with("svn://") || url_lower.starts_with("svn+ssh://") {
            return VcsType::Svn;
        }
        if url_lower.contains("/svn/") || url_lower.ends_with("/svn") {
            return VcsType::Svn;
        }

        // Check for Mercurial
        if url_lower.starts_with("hg://") || url_lower.contains("mercurial") {
            return VcsType::Hg;
        }

        // Check for Fossil
        if url_lower.contains("fossil") || url_lower.ends_with(".fossil") {
            return VcsType::Fossil;
        }

        // Check for Perforce
        if url_lower.starts_with("p4://")
            || url_lower.starts_with("//")
            || url_lower.contains("perforce")
        {
            return VcsType::Perforce;
        }

        // Default to Git
        VcsType::Git
    }

    /// Update a repository.
    ///
    /// # Errors
    /// Returns error if update fails.
    pub fn update(&self, path: &Path) -> Result<()> {
        let repo = self.open(path)?;
        repo.update()
    }

    /// Get status of a repository.
    ///
    /// # Errors
    /// Returns error if status cannot be determined.
    pub fn status(&self, path: &Path) -> Result<RepoStatus> {
        let repo = self.open(path)?;
        repo.status()
    }

    /// Check if a path is a VCS repository.
    #[must_use]
    pub fn is_repository(&self, path: &Path) -> bool {
        VcsType::detect(path).is_some()
    }

    /// Get the VCS type of a repository.
    #[must_use]
    pub fn repository_type(&self, path: &Path) -> Option<VcsType> {
        VcsType::detect(path)
    }

    /// Check which VCS tools are available.
    #[must_use]
    pub fn available_tools(&self) -> Vec<VcsType> {
        let mut available = Vec::new();

        // Git is always checked first (most common)
        if std::process::Command::new("git")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            available.push(VcsType::Git);
        }

        if SvnRepository::is_available() {
            available.push(VcsType::Svn);
        }

        if HgRepository::is_available() {
            available.push(VcsType::Hg);
        }

        if FossilRepository::is_available() {
            available.push(VcsType::Fossil);
        }

        if PerforceRepository::is_available() {
            available.push(VcsType::Perforce);
        }

        available
    }

    /// Warm up reference cache for a list of URLs.
    ///
    /// Pre-caches bare repositories for faster subsequent clones.
    #[must_use]
    pub fn warm_cache(&self, urls: &[&str]) -> Vec<Result<PathBuf>> {
        let Some(ref cache) = self.reference_cache else {
            return urls
                .iter()
                .map(|_| Err(VcsError::git("no cache configured")))
                .collect();
        };

        urls.iter()
            .filter_map(|url| VcsUrl::parse(url).ok())
            .map(|url| cache.get_or_create(&url))
            .collect()
    }

    /// Clear the reference cache.
    pub fn clear_cache(&self) -> Result<()> {
        if let Some(ref cache) = self.reference_cache {
            cache.clear()?;
        }
        Ok(())
    }

    /// Get reference cache statistics.
    #[must_use]
    pub fn cache_stats(&self) -> Option<CacheStats> {
        self.reference_cache.as_ref().map(|cache| CacheStats {
            count: cache.count(),
            size_bytes: cache.current_size(),
        })
    }

    /// Get credential manager.
    #[must_use]
    pub const fn credentials(&self) -> &Arc<CredentialManager> {
        &self.credentials
    }

    /// Get the cache directory.
    #[must_use]
    pub fn cache_dir(&self) -> Option<&Path> {
        self.cache_dir.as_deref()
    }
}

/// Cache statistics.
#[derive(Debug, Clone)]
pub struct CacheStats {
    /// Number of cached repositories.
    pub count: usize,
    /// Total size in bytes.
    pub size_bytes: u64,
}

impl CacheStats {
    /// Format size as human-readable string.
    #[must_use]
    pub fn size_human(&self) -> String {
        const KB: u64 = 1024;
        const MB: u64 = KB * 1024;
        const GB: u64 = MB * 1024;

        if self.size_bytes >= GB {
            format!("{:.2} GB", self.size_bytes as f64 / GB as f64)
        } else if self.size_bytes >= MB {
            format!("{:.2} MB", self.size_bytes as f64 / MB as f64)
        } else if self.size_bytes >= KB {
            format!("{:.2} KB", self.size_bytes as f64 / KB as f64)
        } else {
            format!("{} bytes", self.size_bytes)
        }
    }
}

/// Trait for common repository operations.
pub trait Repository: std::fmt::Debug + Send + Sync {
    /// Get repository path.
    fn path(&self) -> &Path;

    /// Get VCS type.
    fn vcs_type(&self) -> VcsType;

    /// Get current commit/revision.
    fn current_commit(&self) -> Result<String>;

    /// Get repository status.
    fn status(&self) -> Result<RepoStatus>;

    /// Check if repository is dirty.
    fn is_dirty(&self) -> Result<bool>;

    /// Update repository.
    fn update(&self) -> Result<()>;
}

impl Repository for GitRepository {
    fn path(&self) -> &Path {
        Self::path(self)
    }

    fn vcs_type(&self) -> VcsType {
        VcsType::Git
    }

    fn current_commit(&self) -> Result<String> {
        self.head_commit()
    }

    fn status(&self) -> Result<RepoStatus> {
        Self::status(self)
    }

    fn is_dirty(&self) -> Result<bool> {
        Self::is_dirty(self)
    }

    fn update(&self) -> Result<()> {
        self.fetch("origin")
    }
}

impl Repository for SvnRepository {
    fn path(&self) -> &Path {
        Self::path(self)
    }

    fn vcs_type(&self) -> VcsType {
        VcsType::Svn
    }

    fn current_commit(&self) -> Result<String> {
        self.revision()
    }

    fn status(&self) -> Result<RepoStatus> {
        Self::status(self)
    }

    fn is_dirty(&self) -> Result<bool> {
        Self::is_dirty(self)
    }

    fn update(&self) -> Result<()> {
        Self::update(self, None)
    }
}

impl Repository for HgRepository {
    fn path(&self) -> &Path {
        Self::path(self)
    }

    fn vcs_type(&self) -> VcsType {
        VcsType::Hg
    }

    fn current_commit(&self) -> Result<String> {
        self.identify()
    }

    fn status(&self) -> Result<RepoStatus> {
        Self::status(self)
    }

    fn is_dirty(&self) -> Result<bool> {
        Self::is_dirty(self)
    }

    fn update(&self) -> Result<()> {
        self.pull(None)
    }
}

impl Repository for FossilRepository {
    fn path(&self) -> &Path {
        Self::path(self)
    }

    fn vcs_type(&self) -> VcsType {
        VcsType::Fossil
    }

    fn current_commit(&self) -> Result<String> {
        let status = Self::status(self)?;
        Ok(status.head)
    }

    fn status(&self) -> Result<RepoStatus> {
        Self::status(self)
    }

    fn is_dirty(&self) -> Result<bool> {
        Self::is_dirty(self)
    }

    fn update(&self) -> Result<()> {
        self.pull()?;
        Self::update(self)
    }
}

impl Repository for PerforceRepository {
    fn path(&self) -> &Path {
        Self::path(self)
    }

    fn vcs_type(&self) -> VcsType {
        VcsType::Perforce
    }

    fn current_commit(&self) -> Result<String> {
        self.changelist()
    }

    fn status(&self) -> Result<RepoStatus> {
        Self::status(self)
    }

    fn is_dirty(&self) -> Result<bool> {
        Self::is_dirty(self)
    }

    fn update(&self) -> Result<()> {
        Self::update(self, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vcs_manager_new() {
        let manager = VcsManager::new();
        assert!(manager.reference_cache.is_none());
    }

    #[test]
    fn detect_vcs_type_git() {
        let manager = VcsManager::new();
        assert_eq!(
            manager.detect_vcs_type("https://github.com/owner/repo.git"),
            VcsType::Git
        );
    }

    #[test]
    fn detect_vcs_type_svn() {
        let manager = VcsManager::new();
        assert_eq!(
            manager.detect_vcs_type("svn://svn.example.com/repo"),
            VcsType::Svn
        );
        assert_eq!(
            manager.detect_vcs_type("https://example.com/svn/repo"),
            VcsType::Svn
        );
    }

    #[test]
    fn detect_vcs_type_hg() {
        let manager = VcsManager::new();
        assert_eq!(
            manager.detect_vcs_type("hg://hg.example.com/repo"),
            VcsType::Hg
        );
    }

    #[test]
    fn cache_stats_format() {
        let stats = CacheStats {
            count: 5,
            size_bytes: 1024 * 1024 * 100, // 100 MB
        };
        assert!(stats.size_human().contains("MB"));
    }

    #[test]
    fn available_tools() {
        let manager = VcsManager::new();
        let tools = manager.available_tools();
        // Git should almost always be available on development machines
        // but we don't assert it to avoid CI failures
        assert!(tools.len() <= 5);
    }

    #[test]
    fn detect_vcs_type_perforce() {
        let manager = VcsManager::new();
        assert_eq!(
            manager.detect_vcs_type("//depot/project/..."),
            VcsType::Perforce
        );
        assert_eq!(
            manager.detect_vcs_type("p4://server/depot/..."),
            VcsType::Perforce
        );
    }
}
