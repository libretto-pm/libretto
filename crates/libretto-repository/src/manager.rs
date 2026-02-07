//! Repository manager for coordinating multiple repositories.
//!
//! The manager handles:
//! - Multiple repository sources with priority ordering
//! - Unified package search across repositories
//! - Package resolution with conflict handling
//! - Caching coordination
//! - Statistics aggregation

use crate::cache::RepositoryCache;
use crate::error::{RepositoryError, Result};
use crate::packagist::{PackagistClient, PackagistConfig, SecurityAdvisory};
use crate::providers::{
    BitbucketClient, BitbucketConfig, GitHubClient, GitHubConfig, GitLabClient, GitLabConfig,
    ProviderType, VcsProvider, detect_provider,
};
use crate::types::{
    PackageSearchResult, PrioritizedRepository, RepositoryConfig, RepositoryPriority,
    RepositoryType, Stability,
};
use dashmap::DashMap;
use libretto_cache::TieredCache;
use libretto_core::{Package, PackageId, VersionConstraint};
use parking_lot::RwLock;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use tracing::{debug, info, warn};
use url::Url;

/// Repository manager statistics.
#[derive(Debug, Default)]
pub struct ManagerStats {
    /// Total package lookups.
    pub lookups: AtomicU64,
    /// Successful lookups.
    pub successful_lookups: AtomicU64,
    /// Failed lookups.
    pub failed_lookups: AtomicU64,
    /// Total search requests.
    pub searches: AtomicU64,
    /// Security advisory checks.
    pub advisory_checks: AtomicU64,
    /// Total time spent on lookups (ms).
    total_lookup_time_ms: AtomicU64,
}

impl ManagerStats {
    /// Create new stats tracker.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Get average lookup time in milliseconds.
    #[must_use]
    pub fn avg_lookup_time_ms(&self) -> f64 {
        let total = self.total_lookup_time_ms.load(Ordering::Relaxed);
        let count = self.lookups.load(Ordering::Relaxed);
        if count == 0 {
            0.0
        } else {
            total as f64 / count as f64
        }
    }

    /// Get success rate.
    #[must_use]
    pub fn success_rate(&self) -> f64 {
        let total = self.lookups.load(Ordering::Relaxed);
        let success = self.successful_lookups.load(Ordering::Relaxed);
        if total == 0 {
            100.0
        } else {
            (success as f64 / total as f64) * 100.0
        }
    }

    /// Get summary string.
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "Manager: {} lookups ({:.1}% success, {:.1}ms avg), {} searches, {} advisory checks",
            self.lookups.load(Ordering::Relaxed),
            self.success_rate(),
            self.avg_lookup_time_ms(),
            self.searches.load(Ordering::Relaxed),
            self.advisory_checks.load(Ordering::Relaxed)
        )
    }
}

/// High-performance repository manager.
pub struct RepositoryManager {
    /// Configured repositories.
    repositories: RwLock<Vec<PrioritizedRepository>>,
    /// Packagist clients by URL.
    packagist_clients: DashMap<String, Arc<PackagistClient>>,
    /// GitHub client.
    github: RwLock<Option<Arc<GitHubClient>>>,
    /// GitLab client.
    gitlab: RwLock<Option<Arc<GitLabClient>>>,
    /// Bitbucket client.
    bitbucket: RwLock<Option<Arc<BitbucketClient>>>,
    /// Shared cache.
    cache: Arc<RepositoryCache>,
    /// Tiered cache for persistent storage.
    tiered_cache: Option<Arc<TieredCache>>,
    /// Statistics.
    stats: Arc<ManagerStats>,
    /// Default Packagist URL.
    default_packagist_url: RwLock<Option<Url>>,
    /// Minimum stability for package selection.
    minimum_stability: RwLock<Stability>,
}

impl std::fmt::Debug for RepositoryManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RepositoryManager")
            .field("repositories", &self.repositories.read().len())
            .field("packagist_clients", &self.packagist_clients.len())
            .finish()
    }
}

impl Default for RepositoryManager {
    fn default() -> Self {
        Self::new()
    }
}

impl RepositoryManager {
    /// Create a new repository manager.
    #[must_use]
    pub fn new() -> Self {
        Self {
            repositories: RwLock::new(Vec::new()),
            packagist_clients: DashMap::new(),
            github: RwLock::new(None),
            gitlab: RwLock::new(None),
            bitbucket: RwLock::new(None),
            cache: Arc::new(RepositoryCache::new()),
            tiered_cache: None,
            stats: Arc::new(ManagerStats::new()),
            default_packagist_url: RwLock::new(None),
            minimum_stability: RwLock::new(Stability::Stable),
        }
    }

    /// Create manager with tiered cache.
    #[must_use]
    pub fn with_tiered_cache(mut self, cache: Arc<TieredCache>) -> Self {
        self.tiered_cache = Some(cache.clone());
        self.cache = Arc::new(RepositoryCache::with_tiered_cache(cache));
        self
    }

    /// Set minimum stability for package selection.
    pub fn set_minimum_stability(&self, stability: Stability) {
        *self.minimum_stability.write() = stability;
    }

    /// Get minimum stability.
    #[must_use]
    pub fn minimum_stability(&self) -> Stability {
        *self.minimum_stability.read()
    }

    /// Add a repository.
    pub fn add_repository(&self, config: RepositoryConfig, name: impl Into<String>) {
        let repo = PrioritizedRepository::new(config, name);
        self.repositories.write().push(repo);
        self.sort_repositories();
    }

    /// Add a repository with priority.
    pub fn add_repository_with_priority(
        &self,
        config: RepositoryConfig,
        name: impl Into<String>,
        priority: RepositoryPriority,
    ) {
        let repo = PrioritizedRepository::new(config, name).with_priority(priority);
        self.repositories.write().push(repo);
        self.sort_repositories();
    }

    /// Add the default Packagist repository.
    ///
    /// # Errors
    /// Returns error if Packagist client cannot be created.
    pub fn add_packagist(&self) -> Result<()> {
        let url = Url::parse("https://repo.packagist.org/").map_err(|e| {
            RepositoryError::InvalidConfig {
                message: e.to_string(),
            }
        })?;

        let config = RepositoryConfig {
            url: Some(url.clone()),
            repo_type: RepositoryType::Composer,
            auth: None,
            options: Default::default(),
        };

        self.add_repository(config, "packagist.org");
        *self.default_packagist_url.write() = Some(url);

        Ok(())
    }

    /// Get the Packagist client for a URL.
    fn get_packagist_client(&self, url: &Url) -> Result<Arc<PackagistClient>> {
        let key = url.to_string();

        if let Some(client) = self.packagist_clients.get(&key) {
            return Ok(Arc::clone(&client));
        }

        // For repo.packagist.org, use packagist.org for API
        let api_url = if url.host_str() == Some("repo.packagist.org") {
            Url::parse("https://packagist.org/").expect("valid URL")
        } else {
            url.clone()
        };

        let config = PackagistConfig {
            repo_url: url.clone(),
            api_url,
            ..Default::default()
        };

        let client =
            PackagistClient::with_config(config).map(|c| c.with_cache(Arc::clone(&self.cache)))?;
        let client = Arc::new(client);

        self.packagist_clients.insert(key, Arc::clone(&client));
        Ok(client)
    }

    /// Get or create a VCS provider for the given URL.
    fn get_vcs_provider(&self, url: &Url) -> Option<Arc<dyn VcsProvider>> {
        match detect_provider(url)? {
            ProviderType::GitHub => {
                let client = self.github.read();
                if client.is_none() {
                    drop(client);
                    if let Ok(new_client) = GitHubClient::new() {
                        *self.github.write() =
                            Some(Arc::new(new_client.with_cache(Arc::clone(&self.cache))));
                    }
                }
                self.github
                    .read()
                    .as_ref()
                    .map(|c| Arc::clone(c) as Arc<dyn VcsProvider>)
            }
            ProviderType::GitLab => {
                let client = self.gitlab.read();
                if client.is_none() {
                    drop(client);
                    if let Ok(new_client) = GitLabClient::new() {
                        *self.gitlab.write() =
                            Some(Arc::new(new_client.with_cache(Arc::clone(&self.cache))));
                    }
                }
                self.gitlab
                    .read()
                    .as_ref()
                    .map(|c| Arc::clone(c) as Arc<dyn VcsProvider>)
            }
            ProviderType::Bitbucket => {
                let client = self.bitbucket.read();
                if client.is_none() {
                    drop(client);
                    if let Ok(new_client) = BitbucketClient::new() {
                        *self.bitbucket.write() =
                            Some(Arc::new(new_client.with_cache(Arc::clone(&self.cache))));
                    }
                }
                self.bitbucket
                    .read()
                    .as_ref()
                    .map(|c| Arc::clone(c) as Arc<dyn VcsProvider>)
            }
        }
    }

    /// Configure GitHub authentication.
    pub fn set_github_token(&self, token: String) {
        if let Ok(client) = GitHubClient::with_config(GitHubConfig::with_token(token)) {
            *self.github.write() = Some(Arc::new(client.with_cache(Arc::clone(&self.cache))));
        }
    }

    /// Configure GitLab authentication.
    pub fn set_gitlab_token(&self, token: String) {
        if let Ok(client) = GitLabClient::with_config(GitLabConfig::with_token(token)) {
            *self.gitlab.write() = Some(Arc::new(client.with_cache(Arc::clone(&self.cache))));
        }
    }

    /// Configure Bitbucket authentication.
    pub fn set_bitbucket_credentials(&self, username: String, app_password: String) {
        if let Ok(client) =
            BitbucketClient::with_config(BitbucketConfig::with_app_password(username, app_password))
        {
            *self.bitbucket.write() = Some(Arc::new(client.with_cache(Arc::clone(&self.cache))));
        }
    }

    /// Sort repositories by priority.
    fn sort_repositories(&self) {
        self.repositories
            .write()
            .sort_by(|a, b| b.priority.cmp(&a.priority));
    }

    /// Get package from repositories.
    ///
    /// Searches repositories in priority order and returns the first match.
    ///
    /// # Errors
    /// Returns error if package is not found in any repository.
    pub async fn get_package(&self, package_id: &PackageId) -> Result<Vec<Package>> {
        let start = Instant::now();
        self.stats.lookups.fetch_add(1, Ordering::Relaxed);

        let repositories = self.repositories.read().clone();
        let mut last_error = None;

        for repo in repositories.iter().filter(|r| r.enabled) {
            match repo.config.repo_type {
                RepositoryType::Composer => {
                    if let Some(ref url) = repo.config.url {
                        match self.get_packagist_client(url) {
                            Ok(client) => match client.get_package(package_id).await {
                                Ok(packages) if !packages.is_empty() => {
                                    self.stats
                                        .successful_lookups
                                        .fetch_add(1, Ordering::Relaxed);
                                    self.stats.total_lookup_time_ms.fetch_add(
                                        start.elapsed().as_millis() as u64,
                                        Ordering::Relaxed,
                                    );
                                    return Ok(packages);
                                }
                                Ok(_) => {
                                    debug!(package = %package_id, repo = %repo.name, "no versions found");
                                }
                                Err(e) => {
                                    debug!(package = %package_id, repo = %repo.name, error = %e, "lookup failed");
                                    last_error = Some(e);
                                }
                            },
                            Err(e) => {
                                warn!(repo = %repo.name, error = %e, "failed to create client");
                                last_error = Some(e);
                            }
                        }
                    }
                }
                RepositoryType::Vcs => {
                    if let Some(ref url) = repo.config.url
                        && let Some(_provider) = self.get_vcs_provider(url)
                    {
                        // VCS repositories need special handling - we'd fetch composer.json
                        // For now, skip VCS repos in basic package lookup
                        debug!(package = %package_id, repo = %repo.name, "VCS lookup not implemented");
                    }
                }
                RepositoryType::Path => {
                    // Path repositories need local filesystem access
                    debug!(package = %package_id, repo = %repo.name, "path lookup not implemented");
                }
                RepositoryType::Package => {
                    // Check inline package definition
                    if let Some(ref inline) = repo.config.options.package
                        && inline.name == package_id.full_name()
                    {
                        debug!(package = %package_id, repo = %repo.name, "inline package found");
                        // Would convert inline package to Package type
                    }
                }
                RepositoryType::Artifact => {
                    // Artifact repositories need directory scanning
                    debug!(package = %package_id, repo = %repo.name, "artifact lookup not implemented");
                }
            }
        }

        self.stats.failed_lookups.fetch_add(1, Ordering::Relaxed);
        self.stats
            .total_lookup_time_ms
            .fetch_add(start.elapsed().as_millis() as u64, Ordering::Relaxed);

        Err(
            last_error.unwrap_or_else(|| RepositoryError::PackageNotFound {
                name: package_id.full_name(),
                repositories: repositories.iter().map(|r| r.name.clone()).collect(),
            }),
        )
    }

    /// Find best matching version for a package.
    ///
    /// # Errors
    /// Returns error if no matching version is found.
    pub async fn find_version(
        &self,
        package_id: &PackageId,
        constraint: &VersionConstraint,
    ) -> Result<Package> {
        let packages = self.get_package(package_id).await?;
        let min_stability = *self.minimum_stability.read();

        packages
            .into_iter()
            .filter(|p| {
                // Check version constraint
                if !constraint.matches(&p.version) {
                    return false;
                }

                // Check stability
                let stability = Stability::from_version(&p.version.to_string());
                stability >= min_stability
            })
            .max_by(|a, b| a.version.cmp(&b.version))
            .ok_or_else(|| RepositoryError::VersionNotFound {
                name: package_id.full_name(),
                constraint: constraint.to_string(),
            })
    }

    /// Search for packages across all repositories.
    ///
    /// # Errors
    /// Returns error if search fails.
    pub async fn search(&self, query: &str) -> Result<Vec<PackageSearchResult>> {
        self.stats.searches.fetch_add(1, Ordering::Relaxed);

        let repositories = self.repositories.read().clone();
        let mut results = Vec::new();
        let mut seen = std::collections::HashSet::new();

        for repo in repositories.iter().filter(|r| r.enabled) {
            if repo.config.repo_type != RepositoryType::Composer {
                continue;
            }

            if let Some(ref url) = repo.config.url {
                match self.get_packagist_client(url) {
                    Ok(client) => match client.search(query, Some(50)).await {
                        Ok(search_results) => {
                            for result in search_results {
                                if seen.insert(result.name.clone()) {
                                    results.push(PackageSearchResult {
                                        name: result.name,
                                        description: result.description,
                                        downloads: result.downloads,
                                        favers: result.favers,
                                        repository: result.repository,
                                        abandoned: result.abandoned.is_abandoned(),
                                        replacement: result
                                            .abandoned
                                            .replacement()
                                            .map(String::from),
                                    });
                                }
                            }
                        }
                        Err(e) => {
                            warn!(repo = %repo.name, error = %e, "search failed");
                        }
                    },
                    Err(e) => {
                        warn!(repo = %repo.name, error = %e, "failed to create client");
                    }
                }
            }
        }

        info!(query = %query, results = results.len(), "search completed");
        Ok(results)
    }

    /// Get security advisories for packages.
    ///
    /// # Errors
    /// Returns error if advisory check fails.
    pub async fn get_security_advisories(
        &self,
        packages: &[String],
    ) -> Result<Vec<SecurityAdvisory>> {
        self.stats.advisory_checks.fetch_add(1, Ordering::Relaxed);

        // Use default Packagist for advisories
        let url = self.default_packagist_url.read().clone();
        let url =
            url.unwrap_or_else(|| Url::parse("https://repo.packagist.org/").expect("valid URL"));

        let client = self.get_packagist_client(&url)?;
        client.get_security_advisories(packages).await
    }

    /// Send download notification for a package.
    pub async fn notify_download(&self, package_name: &str, version: &str) {
        let default_url = self.default_packagist_url.read().clone();
        if let Some(url) = default_url
            && let Ok(client) = self.get_packagist_client(&url)
        {
            client.notify_download(package_name, version).await;
        }
    }

    /// Get manager statistics.
    #[must_use]
    pub fn stats(&self) -> &ManagerStats {
        &self.stats
    }

    /// Get cache statistics.
    #[must_use]
    pub fn cache_stats(&self) -> &crate::cache::RepositoryCacheStats {
        self.cache.stats()
    }

    /// Clear all caches.
    pub fn clear_cache(&self) {
        self.cache.clear();
        for client in &self.packagist_clients {
            client.clear_cache();
        }
    }

    /// Get list of configured repositories.
    #[must_use]
    pub fn repositories(&self) -> Vec<PrioritizedRepository> {
        self.repositories.read().clone()
    }

    /// Get number of configured repositories.
    #[must_use]
    pub fn repository_count(&self) -> usize {
        self.repositories.read().len()
    }

    /// Initialize all Packagist clients (fetch packages.json).
    ///
    /// # Errors
    /// Returns error if initialization fails.
    pub async fn init_packagist(&self) -> Result<()> {
        for client in &self.packagist_clients {
            client.value().init().await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manager_new() {
        let manager = RepositoryManager::new();
        assert_eq!(manager.repository_count(), 0);
    }

    #[test]
    fn test_add_repository() {
        let manager = RepositoryManager::new();

        let config = RepositoryConfig {
            url: Some(Url::parse("https://example.com").unwrap()),
            repo_type: RepositoryType::Composer,
            auth: None,
            options: Default::default(),
        };

        manager.add_repository(config, "test");
        assert_eq!(manager.repository_count(), 1);
    }

    #[test]
    fn test_repository_priority() {
        let manager = RepositoryManager::new();

        let config1 = RepositoryConfig {
            url: Some(Url::parse("https://first.com").unwrap()),
            repo_type: RepositoryType::Composer,
            auth: None,
            options: Default::default(),
        };

        let config2 = RepositoryConfig {
            url: Some(Url::parse("https://second.com").unwrap()),
            repo_type: RepositoryType::Composer,
            auth: None,
            options: Default::default(),
        };

        manager.add_repository_with_priority(config1, "first", RepositoryPriority::Low);
        manager.add_repository_with_priority(config2, "second", RepositoryPriority::High);

        let repos = manager.repositories();
        assert_eq!(repos[0].name, "second"); // High priority first
        assert_eq!(repos[1].name, "first"); // Low priority second
    }

    #[test]
    fn test_stats() {
        let stats = ManagerStats::new();
        stats.lookups.store(100, Ordering::Relaxed);
        stats.successful_lookups.store(90, Ordering::Relaxed);
        stats.total_lookup_time_ms.store(5000, Ordering::Relaxed);

        assert!((stats.avg_lookup_time_ms() - 50.0).abs() < f64::EPSILON);
        assert!((stats.success_rate() - 90.0).abs() < f64::EPSILON);
    }
}
