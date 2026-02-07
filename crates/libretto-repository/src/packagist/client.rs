//! Packagist API v2 client implementation.
//!
//! Supports:
//! - Metadata-url pattern for lazy package loading
//! - Provider-includes for incremental metadata
//! - `ETags` and If-Modified-Since for caching
//! - Private Packagist instances

use crate::cache::{
    DEFAULT_ADVISORY_TTL, DEFAULT_METADATA_TTL, DEFAULT_SEARCH_TTL, RepositoryCache,
};
use crate::client::{HttpClient, HttpClientConfig};
use crate::error::{RepositoryError, Result};
use crate::packagist::types::{
    ChangesResponse, PackageMetadataResponse, PackageVersionJson, PackagesJson,
    PopularPackagesResponse, SearchResponse, SearchResult, SecurityAdvisoriesResponse,
    SecurityAdvisory, StatisticsResponse, expand_minified_versions,
};
use dashmap::DashMap;
use libretto_core::{Package, PackageId};
use parking_lot::RwLock;
use serde::Serialize;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};
use url::Url;

/// Default Packagist repository URL.
pub const PACKAGIST_URL: &str = "https://repo.packagist.org/";

/// Default Packagist API URL.
pub const PACKAGIST_API_URL: &str = "https://packagist.org/";

/// Packagist client configuration.
#[derive(Debug, Clone)]
pub struct PackagistConfig {
    /// Repository URL (e.g., repo.packagist.org).
    pub repo_url: Url,
    /// API URL (e.g., packagist.org).
    pub api_url: Url,
    /// Bearer token for authentication.
    pub token: Option<String>,
    /// HTTP client configuration.
    pub http_config: HttpClientConfig,
    /// Whether to use lazy loading for metadata.
    pub lazy_load: bool,
    /// Maximum parallel requests for batch fetching.
    pub max_parallel_requests: usize,
    /// Cache metadata TTL override.
    pub metadata_ttl: Option<Duration>,
}

impl Default for PackagistConfig {
    fn default() -> Self {
        Self {
            repo_url: Url::parse(PACKAGIST_URL).expect("valid URL"),
            api_url: Url::parse(PACKAGIST_API_URL).expect("valid URL"),
            token: None,
            http_config: HttpClientConfig::default(),
            lazy_load: true,
            max_parallel_requests: 10,
            metadata_ttl: None,
        }
    }
}

impl PackagistConfig {
    /// Create configuration for a private Packagist instance.
    #[must_use]
    pub fn private(repo_url: Url, api_url: Url, token: String) -> Self {
        Self {
            repo_url,
            api_url,
            token: Some(token),
            ..Default::default()
        }
    }
}

/// Packagist client statistics.
#[derive(Debug, Default)]
pub struct PackagistStats {
    /// Metadata fetches.
    pub metadata_fetches: AtomicU64,
    /// Search requests.
    pub search_requests: AtomicU64,
    /// Advisory checks.
    pub advisory_checks: AtomicU64,
    /// Cache hits.
    pub cache_hits: AtomicU64,
    /// Cache misses.
    pub cache_misses: AtomicU64,
    /// Total packages fetched.
    pub packages_fetched: AtomicU64,
    /// Total versions fetched.
    pub versions_fetched: AtomicU64,
}

impl PackagistStats {
    /// Create new stats tracker.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Get summary string.
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "Packagist: {} metadata fetches, {} searches, {} packages, {} versions, {:.1}% cache hit",
            self.metadata_fetches.load(Ordering::Relaxed),
            self.search_requests.load(Ordering::Relaxed),
            self.packages_fetched.load(Ordering::Relaxed),
            self.versions_fetched.load(Ordering::Relaxed),
            self.cache_hit_rate()
        )
    }

    fn cache_hit_rate(&self) -> f64 {
        let hits = self.cache_hits.load(Ordering::Relaxed);
        let misses = self.cache_misses.load(Ordering::Relaxed);
        let total = hits + misses;
        if total == 0 {
            0.0
        } else {
            (hits as f64 / total as f64) * 100.0
        }
    }
}

/// High-performance Packagist API client.
pub struct PackagistClient {
    /// Configuration.
    config: PackagistConfig,
    /// HTTP client.
    http: Arc<HttpClient>,
    /// Cache.
    cache: Arc<RepositoryCache>,
    /// Metadata URL pattern from packages.json.
    metadata_url: RwLock<Option<String>>,
    /// Notification URL.
    notify_url: RwLock<Option<String>>,
    /// In-flight requests for deduplication.
    in_flight: DashMap<String, Arc<tokio::sync::Semaphore>>,
    /// Statistics.
    stats: Arc<PackagistStats>,
    /// Last packages.json fetch time.
    last_root_fetch: RwLock<Option<Instant>>,
}

impl std::fmt::Debug for PackagistClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PackagistClient")
            .field("repo_url", &self.config.repo_url)
            .field("api_url", &self.config.api_url)
            .finish()
    }
}

impl PackagistClient {
    /// Create a new Packagist client with default configuration.
    ///
    /// # Errors
    /// Returns error if HTTP client cannot be created.
    pub fn new() -> Result<Self> {
        Self::with_config(PackagistConfig::default())
    }

    /// Alias for `new()` - creates a Packagist client for packagist.org.
    ///
    /// # Errors
    /// Returns error if HTTP client cannot be created.
    pub fn packagist() -> Result<Self> {
        Self::new()
    }

    /// Create a new Packagist client with custom configuration.
    ///
    /// # Errors
    /// Returns error if HTTP client cannot be created.
    pub fn with_config(config: PackagistConfig) -> Result<Self> {
        let http = HttpClient::with_config(config.http_config.clone()).map_err(|e| {
            RepositoryError::InvalidConfig {
                message: format!("HTTP client creation failed: {e}"),
            }
        })?;

        // Set authentication if provided
        if let Some(ref token) = config.token {
            if let Some(host) = config.repo_url.host_str() {
                http.set_auth(host, crate::client::AuthType::Bearer(token.clone()));
            }
            if let Some(host) = config.api_url.host_str() {
                http.set_auth(host, crate::client::AuthType::Bearer(token.clone()));
            }
        }

        Ok(Self {
            config,
            http: Arc::new(http),
            cache: Arc::new(RepositoryCache::new()),
            metadata_url: RwLock::new(None),
            notify_url: RwLock::new(None),
            in_flight: DashMap::new(),
            stats: Arc::new(PackagistStats::new()),
            last_root_fetch: RwLock::new(None),
        })
    }

    /// Create client with shared cache.
    #[must_use]
    pub fn with_cache(mut self, cache: Arc<RepositoryCache>) -> Self {
        self.cache = cache;
        self
    }

    /// Get the repository URL.
    #[must_use]
    pub const fn repo_url(&self) -> &Url {
        &self.config.repo_url
    }

    /// Get the API URL.
    #[must_use]
    pub const fn api_url(&self) -> &Url {
        &self.config.api_url
    }

    /// Get client statistics.
    #[must_use]
    pub fn stats(&self) -> &PackagistStats {
        &self.stats
    }

    /// Initialize the client by fetching packages.json.
    ///
    /// # Errors
    /// Returns error if packages.json cannot be fetched.
    pub async fn init(&self) -> Result<()> {
        self.fetch_root().await?;
        Ok(())
    }

    /// Fetch the root packages.json to get metadata URL pattern.
    async fn fetch_root(&self) -> Result<()> {
        // Check if we've fetched recently (within 5 minutes)
        if let Some(last_fetch) = *self.last_root_fetch.read()
            && last_fetch.elapsed() < Duration::from_secs(300)
        {
            return Ok(());
        }

        let url = self.config.repo_url.join("packages.json").map_err(|e| {
            RepositoryError::InvalidUrl {
                url: self.config.repo_url.to_string(),
                message: e.to_string(),
            }
        })?;

        let cache_key = url.to_string();

        // Try cache first
        if let Some(data) = self.cache.get_metadata(&cache_key)
            && let Ok(packages_json) = sonic_rs::from_slice::<PackagesJson>(&data)
        {
            self.update_root_config(&packages_json);
            self.stats.cache_hits.fetch_add(1, Ordering::Relaxed);
            return Ok(());
        }

        // Fetch from network
        let response = self.http.get_with_cache(&url, Some(&cache_key)).await?;

        if response.was_cached {
            self.cache.record_conditional_hit();
            return Ok(());
        }

        let packages_json: PackagesJson =
            sonic_rs::from_slice(&response.body).map_err(|e| RepositoryError::ParseError {
                source: url.to_string(),
                message: e.to_string(),
            })?;

        self.update_root_config(&packages_json);

        // Cache the response
        let ttl = response.max_age.unwrap_or(Duration::from_secs(300));
        let http_meta = self.http.get_cache_metadata(&cache_key);
        self.cache
            .put_metadata(&cache_key, &response.body, ttl, http_meta.as_ref())
            .map_err(|e| RepositoryError::Cache {
                message: e.to_string(),
            })?;

        self.stats.cache_misses.fetch_add(1, Ordering::Relaxed);
        *self.last_root_fetch.write() = Some(Instant::now());

        Ok(())
    }

    /// Update configuration from packages.json.
    fn update_root_config(&self, packages_json: &PackagesJson) {
        if let Some(ref metadata_url) = packages_json.metadata_url {
            *self.metadata_url.write() = Some(metadata_url.clone());
        }
        if let Some(ref notify_batch) = packages_json.notify_batch {
            *self.notify_url.write() = Some(notify_batch.clone());
        }
    }

    /// Get package metadata.
    ///
    /// # Errors
    /// Returns error if package cannot be fetched.
    pub async fn get_package(&self, package_id: &PackageId) -> Result<Vec<Package>> {
        // Ensure we have the metadata URL
        self.fetch_root().await?;

        let package_name = package_id.full_name();
        let cache_key = format!("pkg:{package_name}");

        // Check cache
        if let Some(data) = self.cache.get_metadata(&cache_key)
            && let Ok(versions) = sonic_rs::from_slice::<Vec<PackageVersionJson>>(&data)
        {
            self.stats.cache_hits.fetch_add(1, Ordering::Relaxed);
            let packages: Vec<Package> = versions
                .iter()
                .filter_map(|v| v.to_package(package_id))
                .collect();
            return Ok(packages);
        }

        // Fetch tagged releases
        let versions = self.fetch_package_metadata(package_id, false).await?;

        // Optionally fetch dev releases
        let dev_versions = if self.config.lazy_load {
            vec![] // Only fetch dev versions when explicitly requested
        } else {
            self.fetch_package_metadata(package_id, true)
                .await
                .unwrap_or_default()
        };

        // Combine and expand minified versions
        let all_versions: Vec<PackageVersionJson> =
            versions.into_iter().chain(dev_versions).collect();

        let expanded = expand_minified_versions(&all_versions);

        // Convert to Package types
        let packages: Vec<Package> = expanded
            .iter()
            .filter_map(|v| v.to_package(package_id))
            .collect();

        // Cache the result
        if let Ok(data) = sonic_rs::to_vec(&expanded) {
            let ttl = self.config.metadata_ttl.unwrap_or(DEFAULT_METADATA_TTL);
            let _ = self.cache.put_metadata(&cache_key, &data, ttl, None);
        }

        self.stats.packages_fetched.fetch_add(1, Ordering::Relaxed);
        self.stats
            .versions_fetched
            .fetch_add(packages.len() as u64, Ordering::Relaxed);

        info!(
            package = %package_id,
            versions = packages.len(),
            "fetched package metadata"
        );

        Ok(packages)
    }

    /// Fetch package metadata from Packagist.
    async fn fetch_package_metadata(
        &self,
        package_id: &PackageId,
        dev: bool,
    ) -> Result<Vec<PackageVersionJson>> {
        let metadata_url = self.metadata_url.read().clone();
        let pattern = metadata_url.as_deref().unwrap_or("p2/%package%.json");

        let package_name = package_id.full_name();
        let suffix = if dev { "~dev" } else { "" };
        let path = pattern
            .replace("%package%", &package_name)
            .replace(".json", &format!("{suffix}.json"));

        let url = self
            .config
            .repo_url
            .join(&path)
            .map_err(|e| RepositoryError::InvalidUrl {
                url: format!("{}{path}", self.config.repo_url),
                message: e.to_string(),
            })?;

        let cache_key = url.to_string();

        // Deduplicate in-flight requests
        let semaphore = self
            .in_flight
            .entry(cache_key.clone())
            .or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(1)))
            .clone();

        let _permit = semaphore
            .acquire()
            .await
            .map_err(|_| RepositoryError::Network {
                url: url.to_string(),
                message: "Request cancelled".into(),
                status: None,
            })?;

        let response = self.http.get_with_cache(&url, Some(&cache_key)).await?;

        self.stats.metadata_fetches.fetch_add(1, Ordering::Relaxed);

        if response.was_cached {
            self.cache.record_conditional_hit();
            // Return cached data
            if let Some(data) = self.cache.get_metadata(&cache_key)
                && let Ok(pkg_response) = sonic_rs::from_slice::<PackageMetadataResponse>(&data)
            {
                return Ok(pkg_response
                    .packages
                    .get(&package_name)
                    .cloned()
                    .unwrap_or_default());
            }
        }

        let pkg_response: PackageMetadataResponse =
            sonic_rs::from_slice(&response.body).map_err(|e| RepositoryError::ParseError {
                source: url.to_string(),
                message: e.to_string(),
            })?;

        // Cache the response
        let ttl = response
            .max_age
            .unwrap_or_else(|| self.config.metadata_ttl.unwrap_or(DEFAULT_METADATA_TTL));
        let http_meta = self.http.get_cache_metadata(&cache_key);
        let _ = self
            .cache
            .put_metadata(&cache_key, &response.body, ttl, http_meta.as_ref());

        // Remove from in-flight
        self.in_flight.remove(&cache_key);

        Ok(pkg_response
            .packages
            .get(&package_name)
            .cloned()
            .unwrap_or_default())
    }

    /// Fetch multiple packages in parallel.
    ///
    /// # Errors
    /// Returns error if any package fetch fails.
    pub async fn get_packages_parallel(
        &self,
        package_ids: &[PackageId],
    ) -> Result<Vec<(PackageId, Vec<Package>)>> {
        // Ensure we have the metadata URL
        self.fetch_root().await?;

        let start = Instant::now();

        // Fetch all packages in parallel using tokio
        let futures: Vec<_> = package_ids
            .iter()
            .map(|id| {
                let id = id.clone();
                async move {
                    let result = self.get_package(&id).await;
                    (id, result)
                }
            })
            .collect();

        let results: Vec<_> = futures::future::join_all(futures).await;

        let successful: Vec<_> = results
            .into_iter()
            .filter_map(|(id, result)| match result {
                Ok(packages) => Some((id, packages)),
                Err(e) => {
                    warn!(package = %id, error = %e, "failed to fetch package");
                    None
                }
            })
            .collect();

        debug!(
            packages = successful.len(),
            elapsed_ms = start.elapsed().as_millis(),
            "fetched packages in parallel"
        );

        Ok(successful)
    }

    /// Search for packages.
    ///
    /// # Arguments
    /// * `query` - Search query string.
    /// * `per_page` - Results per page (max 100).
    ///
    /// # Errors
    /// Returns error if search fails.
    pub async fn search(&self, query: &str, per_page: Option<u32>) -> Result<Vec<SearchResult>> {
        let per_page = per_page.unwrap_or(15).min(100);
        let mut url =
            self.config
                .api_url
                .join("search.json")
                .map_err(|e| RepositoryError::InvalidUrl {
                    url: self.config.api_url.to_string(),
                    message: e.to_string(),
                })?;

        url.query_pairs_mut()
            .append_pair("q", query)
            .append_pair("per_page", &per_page.to_string());

        let cache_key = format!("search:{query}:{per_page}");

        // Check cache
        if let Some(data) = self.cache.get_metadata(&cache_key)
            && let Ok(results) = sonic_rs::from_slice::<Vec<SearchResult>>(&data)
        {
            self.stats.cache_hits.fetch_add(1, Ordering::Relaxed);
            return Ok(results);
        }

        let response = self.http.get(&url).await?;
        self.stats.search_requests.fetch_add(1, Ordering::Relaxed);

        let search_response: SearchResponse =
            sonic_rs::from_slice(&response.body).map_err(|e| RepositoryError::ParseError {
                source: url.to_string(),
                message: e.to_string(),
            })?;

        // Cache results
        if let Ok(data) = sonic_rs::to_vec(&search_response.results) {
            let _ = self
                .cache
                .put_metadata(&cache_key, &data, DEFAULT_SEARCH_TTL, None);
        }

        info!(
            query = %query,
            results = search_response.results.len(),
            total = search_response.total,
            "search completed"
        );

        Ok(search_response.results)
    }

    /// Search with pagination.
    ///
    /// # Errors
    /// Returns error if search fails.
    pub async fn search_paginated(
        &self,
        query: &str,
        page: u32,
        per_page: u32,
    ) -> Result<(Vec<SearchResult>, u64, Option<String>)> {
        let per_page = per_page.min(100);
        let mut url =
            self.config
                .api_url
                .join("search.json")
                .map_err(|e| RepositoryError::InvalidUrl {
                    url: self.config.api_url.to_string(),
                    message: e.to_string(),
                })?;

        url.query_pairs_mut()
            .append_pair("q", query)
            .append_pair("per_page", &per_page.to_string())
            .append_pair("page", &page.to_string());

        let response = self.http.get(&url).await?;
        self.stats.search_requests.fetch_add(1, Ordering::Relaxed);

        let search_response: SearchResponse =
            sonic_rs::from_slice(&response.body).map_err(|e| RepositoryError::ParseError {
                source: url.to_string(),
                message: e.to_string(),
            })?;

        Ok((
            search_response.results,
            search_response.total,
            search_response.next,
        ))
    }

    /// Search by tag.
    ///
    /// # Errors
    /// Returns error if search fails.
    pub async fn search_by_tag(&self, tag: &str) -> Result<Vec<SearchResult>> {
        let mut url =
            self.config
                .api_url
                .join("search.json")
                .map_err(|e| RepositoryError::InvalidUrl {
                    url: self.config.api_url.to_string(),
                    message: e.to_string(),
                })?;

        url.query_pairs_mut().append_pair("tags", tag);

        let response = self.http.get(&url).await?;
        self.stats.search_requests.fetch_add(1, Ordering::Relaxed);

        let search_response: SearchResponse =
            sonic_rs::from_slice(&response.body).map_err(|e| RepositoryError::ParseError {
                source: url.to_string(),
                message: e.to_string(),
            })?;

        Ok(search_response.results)
    }

    /// Search by type.
    ///
    /// # Errors
    /// Returns error if search fails.
    pub async fn search_by_type(
        &self,
        query: &str,
        package_type: &str,
    ) -> Result<Vec<SearchResult>> {
        let mut url =
            self.config
                .api_url
                .join("search.json")
                .map_err(|e| RepositoryError::InvalidUrl {
                    url: self.config.api_url.to_string(),
                    message: e.to_string(),
                })?;

        url.query_pairs_mut()
            .append_pair("q", query)
            .append_pair("type", package_type);

        let response = self.http.get(&url).await?;
        self.stats.search_requests.fetch_add(1, Ordering::Relaxed);

        let search_response: SearchResponse =
            sonic_rs::from_slice(&response.body).map_err(|e| RepositoryError::ParseError {
                source: url.to_string(),
                message: e.to_string(),
            })?;

        Ok(search_response.results)
    }

    /// Get popular packages.
    ///
    /// # Errors
    /// Returns error if fetch fails.
    pub async fn get_popular(
        &self,
        per_page: Option<u32>,
    ) -> Result<Vec<crate::packagist::types::PopularPackage>> {
        let per_page = per_page.unwrap_or(100).min(100);
        let mut url = self
            .config
            .api_url
            .join("explore/popular.json")
            .map_err(|e| RepositoryError::InvalidUrl {
                url: self.config.api_url.to_string(),
                message: e.to_string(),
            })?;

        url.query_pairs_mut()
            .append_pair("per_page", &per_page.to_string());

        let response = self.http.get(&url).await?;

        let popular_response: PopularPackagesResponse = sonic_rs::from_slice(&response.body)
            .map_err(|e| RepositoryError::ParseError {
                source: url.to_string(),
                message: e.to_string(),
            })?;

        Ok(popular_response.packages)
    }

    /// Get security advisories for packages.
    ///
    /// # Arguments
    /// * `packages` - List of package names to check.
    ///
    /// # Errors
    /// Returns error if advisory fetch fails.
    pub async fn get_security_advisories(
        &self,
        packages: &[String],
    ) -> Result<Vec<SecurityAdvisory>> {
        if packages.is_empty() {
            return Ok(vec![]);
        }

        let cache_key = format!("advisories:{}", packages.join(","));

        // Check cache
        if let Some(data) = self.cache.get_metadata(&cache_key)
            && let Ok(advisories) = sonic_rs::from_slice::<Vec<SecurityAdvisory>>(&data)
        {
            self.stats.cache_hits.fetch_add(1, Ordering::Relaxed);
            return Ok(advisories);
        }

        let mut url = self
            .config
            .api_url
            .join("api/security-advisories/")
            .map_err(|e| RepositoryError::InvalidUrl {
                url: self.config.api_url.to_string(),
                message: e.to_string(),
            })?;

        // Add packages as query parameters
        {
            let mut query = url.query_pairs_mut();
            for package in packages {
                query.append_pair("packages[]", package);
            }
        }

        let response = self.http.get(&url).await?;
        self.stats.advisory_checks.fetch_add(1, Ordering::Relaxed);

        let advisories_response: SecurityAdvisoriesResponse = sonic_rs::from_slice(&response.body)
            .map_err(|e| RepositoryError::ParseError {
                source: url.to_string(),
                message: e.to_string(),
            })?;

        // Flatten advisories from all packages
        let all_advisories: Vec<SecurityAdvisory> = advisories_response
            .advisories
            .into_values()
            .flatten()
            .collect();

        // Cache results
        if let Ok(data) = sonic_rs::to_vec(&all_advisories) {
            let _ = self
                .cache
                .put_metadata(&cache_key, &data, DEFAULT_ADVISORY_TTL, None);
        }

        info!(
            packages = packages.len(),
            advisories = all_advisories.len(),
            "checked security advisories"
        );

        Ok(all_advisories)
    }

    /// Get updates since a timestamp.
    ///
    /// # Arguments
    /// * `since` - Timestamp to get changes since (10000 * `unix_timestamp`).
    ///
    /// # Errors
    /// Returns error if changes cannot be fetched.
    pub async fn get_changes(&self, since: u64) -> Result<ChangesResponse> {
        let mut url = self
            .config
            .api_url
            .join("metadata/changes.json")
            .map_err(|e| RepositoryError::InvalidUrl {
                url: self.config.api_url.to_string(),
                message: e.to_string(),
            })?;

        url.query_pairs_mut()
            .append_pair("since", &since.to_string());

        let response = self.http.get(&url).await?;

        let changes: ChangesResponse =
            sonic_rs::from_slice(&response.body).map_err(|e| RepositoryError::ParseError {
                source: url.to_string(),
                message: e.to_string(),
            })?;

        if let Some(ref error) = changes.error {
            warn!(error = %error, "changes API returned error");
        }

        Ok(changes)
    }

    /// Get Packagist statistics.
    ///
    /// # Errors
    /// Returns error if stats cannot be fetched.
    pub async fn get_statistics(&self) -> Result<StatisticsResponse> {
        let url = self.config.api_url.join("statistics.json").map_err(|e| {
            RepositoryError::InvalidUrl {
                url: self.config.api_url.to_string(),
                message: e.to_string(),
            }
        })?;

        let response = self.http.get(&url).await?;

        let stats: StatisticsResponse =
            sonic_rs::from_slice(&response.body).map_err(|e| RepositoryError::ParseError {
                source: url.to_string(),
                message: e.to_string(),
            })?;

        Ok(stats)
    }

    /// Send download notification (fire-and-forget).
    ///
    /// This is called when a package is downloaded to report statistics
    /// to Packagist.
    pub async fn notify_download(&self, package_name: &str, version: &str) {
        let notify_url = self.notify_url.read().clone();

        if let Some(url_str) = notify_url {
            let url_str = if url_str.starts_with("http") {
                url_str
            } else {
                format!("{}{}", self.config.api_url, url_str.trim_start_matches('/'))
            };

            if let Ok(url) = Url::parse(&url_str) {
                let body = sonic_rs::to_string(&NotifyPayload {
                    downloads: vec![NotifyDownload {
                        name: package_name.to_string(),
                        version: version.to_string(),
                    }],
                })
                .unwrap_or_default();

                self.http.post_fire_and_forget(&url, &body).await;
                debug!(package = %package_name, version = %version, "sent download notification");
            }
        }
    }

    /// Clear all cached data.
    pub fn clear_cache(&self) {
        self.cache.clear();
        *self.metadata_url.write() = None;
        *self.notify_url.write() = None;
        *self.last_root_fetch.write() = None;
    }

    /// Get the cache instance.
    #[must_use]
    pub fn cache(&self) -> &RepositoryCache {
        &self.cache
    }

    /// Get the HTTP client.
    #[must_use]
    pub fn http_client(&self) -> &HttpClient {
        &self.http
    }
}

/// Notification payload for download tracking.
#[derive(Debug, Serialize)]
struct NotifyPayload {
    downloads: Vec<NotifyDownload>,
}

/// Individual download notification.
#[derive(Debug, Serialize)]
struct NotifyDownload {
    name: String,
    version: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = PackagistConfig::default();
        assert_eq!(config.repo_url.as_str(), PACKAGIST_URL);
        assert_eq!(config.api_url.as_str(), PACKAGIST_API_URL);
        assert!(config.token.is_none());
        assert!(config.lazy_load);
    }

    #[test]
    fn test_config_private() {
        let repo_url = Url::parse("https://repo.private.packagist.com/acme/").unwrap();
        let api_url = Url::parse("https://private.packagist.com/acme/").unwrap();
        let config = PackagistConfig::private(repo_url.clone(), api_url.clone(), "token123".into());

        assert_eq!(config.repo_url, repo_url);
        assert_eq!(config.api_url, api_url);
        assert_eq!(config.token, Some("token123".into()));
    }

    #[test]
    fn test_stats() {
        let stats = PackagistStats::new();
        stats.cache_hits.store(75, Ordering::Relaxed);
        stats.cache_misses.store(25, Ordering::Relaxed);

        assert!((stats.cache_hit_rate() - 75.0).abs() < f64::EPSILON);
    }
}
