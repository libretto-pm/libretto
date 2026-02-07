//! Fast Packagist fetcher with metadata caching.
//!
//! Uses reqwest with HTTP/2, connection pooling, and aggressive timeouts.
//! Caches package metadata locally for fast resolution on subsequent runs.

use libretto_repository::providers::{
    BitbucketClient, GitHubClient, GitLabClient, ProviderType, VcsProvider, detect_provider,
    parse_vcs_url,
};
use libretto_resolver::turbo::{FetchedPackage, FetchedVersion, TurboFetcher};
use reqwest::Client;
use sonic_rs::{JsonContainerTrait, JsonValueTrait, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tracing::{debug, trace};
use url::Url;

/// Cache TTL for skipping even conditional requests (5 minutes).
/// Within this window, we don't even send a conditional request.
/// After this, we send If-Modified-Since and usually get a fast 304.
const METADATA_SKIP_TTL: Duration = Duration::from_secs(300);

/// Statistics collected during package fetching operations.
#[derive(Debug, Clone, Default)]
pub struct FetcherStats {
    /// Total HTTP requests made.
    pub requests: u64,
    /// Total bytes downloaded from network.
    pub bytes_downloaded: u64,
    /// Requests served from local cache.
    pub cache_hits: u64,
}

impl FetcherStats {
    /// Calculate cache hit rate as a percentage.
    pub fn cache_hit_rate(&self) -> f64 {
        let total = self.requests + self.cache_hits;
        if total == 0 {
            0.0
        } else {
            (self.cache_hits as f64 / total as f64) * 100.0
        }
    }
}

/// Fast Packagist fetcher with HTTP/2, connection pooling, and metadata caching.
pub struct Fetcher {
    client: Client,
    base_url: String,
    cache_dir: PathBuf,
    vcs_repositories: Vec<Url>,
    vcs_owner_index: HashMap<String, Vec<Url>>,
    root_constraints: HashMap<String, String>,
    vcs_cache: dashmap::DashMap<String, Option<FetchedPackage>>,
    requests: AtomicU64,
    bytes: AtomicU64,
    cache_hits: AtomicU64,
}

impl Fetcher {
    #[allow(dead_code)]
    pub fn new() -> Result<Self, reqwest::Error> {
        Self::new_with_vcs_context(Vec::new(), HashMap::new())
    }

    pub fn new_with_composer_repositories(composer: &Value) -> Result<Self, reqwest::Error> {
        let repositories = extract_vcs_repository_urls(composer);
        let root_constraints = extract_root_constraints(composer);
        Self::new_with_vcs_context(repositories, root_constraints)
    }

    #[allow(dead_code)]
    pub fn new_with_vcs_repositories(
        vcs_repositories: Vec<String>,
    ) -> Result<Self, reqwest::Error> {
        Self::new_with_vcs_context(vcs_repositories, HashMap::new())
    }

    pub fn new_with_vcs_context(
        vcs_repositories: Vec<String>,
        root_constraints: HashMap<String, String>,
    ) -> Result<Self, reqwest::Error> {
        let client = Client::builder()
            // Connection pooling
            .pool_max_idle_per_host(100)
            .pool_idle_timeout(Duration::from_secs(90))
            // HTTP/2 settings
            .http2_adaptive_window(true)
            .http2_initial_stream_window_size(2 * 1024 * 1024)
            .http2_initial_connection_window_size(4 * 1024 * 1024)
            // Timeouts
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(10))
            // Compression
            .gzip(true)
            .brotli(true)
            .deflate(true)
            .zstd(true)
            // TCP optimizations
            .tcp_keepalive(Duration::from_secs(30))
            .tcp_nodelay(true)
            .user_agent(format!("Libretto/{}", env!("CARGO_PKG_VERSION")))
            .build()?;

        // Set up metadata cache directory
        let cache_dir = directories::BaseDirs::new().map_or_else(
            || PathBuf::from(".libretto/metadata"),
            |d| d.home_dir().join(".libretto").join("metadata"),
        );
        let _ = std::fs::create_dir_all(&cache_dir);

        let vcs_repositories: Vec<Url> = vcs_repositories
            .into_iter()
            .filter_map(|url| Url::parse(&url).ok())
            .collect();
        let mut vcs_owner_index: HashMap<String, Vec<Url>> = HashMap::new();
        for url in &vcs_repositories {
            if let Some((owner, _)) = parse_vcs_url(url) {
                vcs_owner_index
                    .entry(owner.to_ascii_lowercase())
                    .or_default()
                    .push(url.clone());
            }
        }

        Ok(Self {
            client,
            base_url: "https://repo.packagist.org/p2".to_string(),
            cache_dir,
            vcs_repositories,
            vcs_owner_index,
            root_constraints,
            vcs_cache: dashmap::DashMap::new(),
            requests: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
            cache_hits: AtomicU64::new(0),
        })
    }

    /// Get the total number of HTTP requests made.
    ///
    /// This can be used for statistics reporting after fetching operations.
    pub fn request_count(&self) -> u64 {
        self.requests.load(Ordering::Relaxed)
    }

    /// Get the total bytes downloaded from the network.
    ///
    /// This can be used for statistics reporting after fetching operations.
    pub fn bytes_downloaded(&self) -> u64 {
        self.bytes.load(Ordering::Relaxed)
    }

    /// Get the number of cache hits (requests served from local cache).
    ///
    /// This can be used for statistics reporting after fetching operations.
    pub fn cache_hits(&self) -> u64 {
        self.cache_hits.load(Ordering::Relaxed)
    }

    /// Get fetch statistics as a tuple: (requests, `bytes_downloaded`, `cache_hits`).
    ///
    /// Convenience method for getting all stats at once for reporting.
    pub fn stats(&self) -> FetcherStats {
        FetcherStats {
            requests: self.request_count(),
            bytes_downloaded: self.bytes_downloaded(),
            cache_hits: self.cache_hits(),
        }
    }

    /// Get cache file path for a package
    fn cache_path(&self, name: &str) -> PathBuf {
        // Replace / with ~ for filesystem safety
        let safe_name = name.replace('/', "~");
        self.cache_dir.join(format!("{safe_name}.json"))
    }

    /// Get `ETag` cache path
    fn etag_path(&self, name: &str) -> PathBuf {
        let safe_name = name.replace('/', "~");
        self.cache_dir.join(format!("{safe_name}.etag"))
    }

    /// Read cached data if it exists (regardless of age).
    fn read_cache(&self, name: &str) -> Option<Vec<u8>> {
        std::fs::read(self.cache_path(name)).ok()
    }

    /// Check if cache is fresh enough to skip even a conditional request.
    fn is_cache_fresh(&self, name: &str) -> bool {
        let path = self.cache_path(name);
        if let Ok(meta) = std::fs::metadata(&path)
            && let Ok(modified) = meta.modified()
            && let Ok(age) = modified.elapsed()
        {
            age < METADATA_SKIP_TTL
        } else {
            false
        }
    }

    /// Read stored `ETag` for a package.
    fn read_etag(&self, name: &str) -> Option<String> {
        std::fs::read_to_string(self.etag_path(name)).ok()
    }

    /// Write response data + `ETag` to cache, touch the modification time.
    fn write_cache(&self, name: &str, data: &[u8], etag: Option<&str>) {
        let _ = std::fs::write(self.cache_path(name), data);
        if let Some(etag) = etag {
            let _ = std::fs::write(self.etag_path(name), etag);
        }
    }

    /// Touch cache file to reset its modification time (on 304 Not Modified).
    fn touch_cache(&self, name: &str) {
        let path = self.cache_path(name);
        // Update mtime by re-opening and setting length to current length
        if let Ok(file) = std::fs::OpenOptions::new().write(true).open(&path)
            && let Ok(meta) = file.metadata()
        {
            let _ = file.set_len(meta.len());
        }
    }

    async fn fetch_from_vcs(&self, name: &str) -> Option<FetchedPackage> {
        if self.vcs_repositories.is_empty() {
            return None;
        }

        if let Some(cached) = self.vcs_cache.get(name) {
            return cached.clone();
        }

        let is_root_package = self.root_constraints.contains_key(name);
        let prefer_branch_only = self
            .root_constraints
            .get(name)
            .is_some_and(|constraint| is_dev_constraint(constraint));

        // First, narrow candidate repositories by vendor/owner match.
        // This avoids blasting VCS APIs for unrelated transitive package names.
        let vendor = name.split('/').next().map(|v| v.to_ascii_lowercase());
        let mut candidate_urls: Vec<Url> = vendor
            .as_deref()
            .and_then(|vendor| self.vcs_owner_index.get(vendor))
            .cloned()
            .unwrap_or_default();

        // For direct root requirements, still fall back to all configured VCS repos
        // to support fork overrides where package vendor differs from repo owner.
        if is_root_package {
            for url in &self.vcs_repositories {
                if !candidate_urls.iter().any(|u| u == url) {
                    candidate_urls.push(url.clone());
                }
            }
        }

        if candidate_urls.is_empty() {
            self.vcs_cache.insert(name.to_string(), None);
            return None;
        }

        for url in &candidate_urls {
            let package = match detect_provider(url) {
                Some(ProviderType::GitHub) => {
                    if let Ok(client) = GitHubClient::new() {
                        self.fetch_from_vcs_provider(&client, url, name, prefer_branch_only)
                            .await
                    } else {
                        None
                    }
                }
                Some(ProviderType::GitLab) => {
                    if let Ok(client) = GitLabClient::new() {
                        self.fetch_from_vcs_provider(&client, url, name, prefer_branch_only)
                            .await
                    } else {
                        None
                    }
                }
                Some(ProviderType::Bitbucket) => {
                    if let Ok(client) = BitbucketClient::new() {
                        self.fetch_from_vcs_provider(&client, url, name, prefer_branch_only)
                            .await
                    } else {
                        None
                    }
                }
                None => None,
            };

            if let Some(package) = package {
                self.vcs_cache
                    .insert(name.to_string(), Some(package.clone()));
                return Some(package);
            }
        }

        self.vcs_cache.insert(name.to_string(), None);
        None
    }

    async fn fetch_from_vcs_provider<P: VcsProvider>(
        &self,
        provider: &P,
        url: &Url,
        expected_name: &str,
        prefer_branch_only: bool,
    ) -> Option<FetchedPackage> {
        // Fast path first: default/main/master only.
        // This is enough for common dev-branch constraints and keeps API usage bounded.
        let mut quick_refs: Vec<(String, bool)> = Vec::new();
        let default_branch: Option<String> =
            if let Ok(default_branch_name) = provider.get_default_branch(url).await {
                quick_refs.push((default_branch_name.clone(), true));
                Some(default_branch_name)
            } else {
                None
            };
        quick_refs.push(("main".to_string(), true));
        quick_refs.push(("master".to_string(), true));

        let mut checked_refs: HashSet<String> = HashSet::new();
        let mut versions = Vec::new();

        for (reference, is_branch) in quick_refs {
            if !checked_refs.insert(reference.clone()) {
                continue;
            }

            let composer_json = match self
                .fetch_vcs_composer_json_with_fallback(provider, url, &reference)
                .await
            {
                Some(content) => content,
                None => continue,
            };

            let composer: Value = match sonic_rs::from_str(&composer_json) {
                Ok(value) => value,
                Err(_) => continue,
            };

            let Some(package_name) = composer.get("name").and_then(Value::as_str) else {
                continue;
            };
            if !package_name.eq_ignore_ascii_case(expected_name) {
                continue;
            }

            let mut require = parse_string_map(&composer, "require");
            require.retain(|(dep, _)| !is_platform_package(dep));

            let version = if is_branch {
                format!("dev-{reference}")
            } else {
                reference.clone()
            };

            versions.push(FetchedVersion {
                version,
                require,
                require_dev: parse_string_map(&composer, "require-dev"),
                replace: parse_string_map(&composer, "replace"),
                provide: parse_string_map(&composer, "provide"),
                suggest: parse_string_map(&composer, "suggest"),
                dist_url: vcs_dist_url(provider.name(), url, &reference),
                dist_type: Some("zip".to_string()),
                dist_shasum: None,
                source_url: Some(url.to_string()),
                source_type: Some("git".to_string()),
                source_reference: Some(reference.clone()),
                package_type: composer
                    .get("type")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                description: composer
                    .get("description")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                homepage: composer
                    .get("homepage")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                license: parse_string_or_array(&composer, "license"),
                authors: composer.get("authors").cloned(),
                keywords: parse_string_or_array(&composer, "keywords"),
                time: composer
                    .get("time")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                autoload: composer.get("autoload").cloned(),
                autoload_dev: composer.get("autoload-dev").cloned(),
                extra: composer.get("extra").cloned(),
                support: composer.get("support").cloned(),
                funding: composer.get("funding").cloned(),
                notification_url: composer
                    .get("notification-url")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                bin: parse_string_or_array(&composer, "bin"),
            });

            // Composer compatibility: when a repository's default branch was
            // renamed away from `master`, legacy constraints may still use
            // `dev-master`. Add an alias for the default branch.
            if is_branch
                && default_branch.as_deref() == Some(reference.as_str())
                && reference != "master"
                && !versions.iter().any(|v| v.version == "dev-master")
                && let Some(mut alias) = versions.last().cloned()
            {
                alias.version = "dev-master".to_string();
                versions.push(alias);
            }
        }

        if versions.is_empty() {
            return None;
        }

        // For explicit dev branch constraints, the quick references are sufficient.
        if prefer_branch_only {
            return Some(FetchedPackage {
                name: expected_name.to_string(),
                versions,
            });
        }

        // Slow path: enrich with additional branches/tags, capped to avoid API exhaustion.
        let mut extra_refs: Vec<(String, bool)> = Vec::new();

        if let Ok(branches) = provider.list_branches(url).await {
            for branch in branches.into_iter().take(16) {
                extra_refs.push((branch, true));
            }
        }

        if let Ok(tags) = provider.list_tags(url).await {
            for tag in tags.into_iter().take(24) {
                extra_refs.push((tag, false));
            }
        }

        for (reference, is_branch) in extra_refs {
            if !checked_refs.insert(reference.clone()) {
                continue;
            }

            let composer_json = match self
                .fetch_vcs_composer_json_with_fallback(provider, url, &reference)
                .await
            {
                Some(content) => content,
                None => continue,
            };

            let composer: Value = match sonic_rs::from_str(&composer_json) {
                Ok(value) => value,
                Err(_) => continue,
            };

            let Some(package_name) = composer.get("name").and_then(Value::as_str) else {
                continue;
            };
            if !package_name.eq_ignore_ascii_case(expected_name) {
                continue;
            }

            let mut require = parse_string_map(&composer, "require");
            require.retain(|(dep, _)| !is_platform_package(dep));

            let version = if is_branch {
                format!("dev-{reference}")
            } else {
                reference.clone()
            };

            versions.push(FetchedVersion {
                version,
                require,
                require_dev: parse_string_map(&composer, "require-dev"),
                replace: parse_string_map(&composer, "replace"),
                provide: parse_string_map(&composer, "provide"),
                suggest: parse_string_map(&composer, "suggest"),
                dist_url: vcs_dist_url(provider.name(), url, &reference),
                dist_type: Some("zip".to_string()),
                dist_shasum: None,
                source_url: Some(url.to_string()),
                source_type: Some("git".to_string()),
                source_reference: Some(reference.clone()),
                package_type: composer
                    .get("type")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                description: composer
                    .get("description")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                homepage: composer
                    .get("homepage")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                license: parse_string_or_array(&composer, "license"),
                authors: composer.get("authors").cloned(),
                keywords: parse_string_or_array(&composer, "keywords"),
                time: composer
                    .get("time")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                autoload: composer.get("autoload").cloned(),
                autoload_dev: composer.get("autoload-dev").cloned(),
                extra: composer.get("extra").cloned(),
                support: composer.get("support").cloned(),
                funding: composer.get("funding").cloned(),
                notification_url: composer
                    .get("notification-url")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                bin: parse_string_or_array(&composer, "bin"),
            });

            // Composer compatibility: when a repository's default branch was
            // renamed away from `master`, legacy constraints may still use
            // `dev-master`. Add an alias for the default branch.
            if is_branch
                && default_branch.as_deref() == Some(reference.as_str())
                && reference != "master"
                && !versions.iter().any(|v| v.version == "dev-master")
                && let Some(mut alias) = versions.last().cloned()
            {
                alias.version = "dev-master".to_string();
                versions.push(alias);
            }
        }

        if versions.is_empty() {
            None
        } else {
            Some(FetchedPackage {
                name: expected_name.to_string(),
                versions,
            })
        }
    }

    async fn fetch_vcs_composer_json_with_fallback<P: VcsProvider>(
        &self,
        provider: &P,
        url: &Url,
        reference: &str,
    ) -> Option<String> {
        if let Ok(content) = provider.fetch_composer_json(url, reference).await {
            return Some(content);
        }

        // Public GitHub repositories can still expose raw files even when the API
        // is unavailable (e.g. unauthenticated rate limits).
        self.fetch_github_raw_composer_json(url, reference).await
    }

    async fn fetch_github_raw_composer_json(&self, url: &Url, reference: &str) -> Option<String> {
        let host = url.host_str()?;
        if !host.eq_ignore_ascii_case("github.com") {
            return None;
        }

        let (owner, repo) = parse_vcs_url(url)?;
        let raw_url =
            format!("https://raw.githubusercontent.com/{owner}/{repo}/{reference}/composer.json");

        self.requests.fetch_add(1, Ordering::Relaxed);
        let response = self.client.get(&raw_url).send().await.ok()?;
        if !response.status().is_success() {
            return None;
        }

        let bytes = response.bytes().await.ok()?;
        self.bytes.fetch_add(bytes.len() as u64, Ordering::Relaxed);
        String::from_utf8(bytes.to_vec()).ok()
    }

    async fn fetch_impl(&self, name: &str) -> Option<FetchedPackage> {
        // If cache is very fresh (< 5 min), skip network entirely
        if self.is_cache_fresh(name)
            && let Some(cached) = self.read_cache(name)
        {
            self.cache_hits.fetch_add(1, Ordering::Relaxed);
            trace!(package = %name, "cache fresh, skipping network");
            if let Some(package) = self.parse_response(name, &cached) {
                return Some(package);
            }
        }

        // We have a cache file but it's older than SKIP_TTL — do a conditional request
        let has_cache = self.cache_path(name).exists();
        let url = format!("{}/{}.json", self.base_url, name);
        self.requests.fetch_add(1, Ordering::Relaxed);

        let mut request = self.client.get(&url);

        // Add conditional headers
        if has_cache {
            // Prefer ETag (more reliable), fall back to If-Modified-Since
            if let Some(etag) = self.read_etag(name) {
                request = request.header("If-None-Match", etag);
            }
            if let Ok(meta) = std::fs::metadata(self.cache_path(name))
                && let Ok(modified) = meta.modified()
            {
                // Convert SystemTime to HTTP date
                let datetime: chrono::DateTime<chrono::Utc> = modified.into();
                request = request.header(
                    "If-Modified-Since",
                    datetime.format("%a, %d %b %Y %H:%M:%S GMT").to_string(),
                );
            }
        }

        let response = match request.send().await {
            Ok(r) => r,
            Err(e) => {
                debug!(package = %name, error = %e, "fetch failed");
                // Fall back to stale cache on network error
                if let Some(cached) = self.read_cache(name) {
                    trace!(package = %name, "using stale cache after network error");
                    if let Some(package) = self.parse_response(name, &cached) {
                        return Some(package);
                    }
                }
                return self.fetch_from_vcs(name).await;
            }
        };

        // 304 Not Modified — cache is still valid
        if response.status() == reqwest::StatusCode::NOT_MODIFIED {
            self.cache_hits.fetch_add(1, Ordering::Relaxed);
            self.touch_cache(name);
            trace!(package = %name, "304 not modified");
            if let Some(cached) = self.read_cache(name)
                && let Some(package) = self.parse_response(name, &cached)
            {
                return Some(package);
            }
            return self.fetch_from_vcs(name).await;
        }

        if !response.status().is_success() {
            debug!(package = %name, status = %response.status(), "non-success status");
            return self.fetch_from_vcs(name).await;
        }

        // Save ETag from response
        let etag = response
            .headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .map(String::from);

        let bytes = match response.bytes().await {
            Ok(b) => b,
            Err(e) => {
                debug!(package = %name, error = %e, "failed to read body");
                return self.fetch_from_vcs(name).await;
            }
        };

        self.bytes.fetch_add(bytes.len() as u64, Ordering::Relaxed);

        // Cache the response
        self.write_cache(name, &bytes, etag.as_deref());

        let bytes = bytes.to_vec();
        if let Some(package) = self.parse_response(name, &bytes) {
            return Some(package);
        }

        self.fetch_from_vcs(name).await
    }

    fn parse_response(&self, name: &str, bytes: &[u8]) -> Option<FetchedPackage> {
        let json: Value = match sonic_rs::from_slice(bytes) {
            Ok(j) => j,
            Err(e) => {
                debug!(package = %name, error = %e, "JSON parse failed");
                return None;
            }
        };

        let minified = json.get("minified").and_then(Value::as_str) == Some("composer/2.0");
        let package_name = name.to_string();
        let versions = json
            .get("packages")
            .and_then(Value::as_object)
            .and_then(|packages| packages.get(&package_name))
            .and_then(Value::as_array)?;

        let expanded_versions = expand_versions_for_deserialize(versions, minified);

        let fetched_versions: Vec<FetchedVersion> = expanded_versions
            .iter()
            .filter_map(|version_value| {
                let version_json = match sonic_rs::to_string(version_value) {
                    Ok(json) => json,
                    Err(e) => {
                        debug!(package = %name, error = %e, "failed to serialize package version");
                        return None;
                    }
                };

                let v: PackagistVersion = match sonic_rs::from_str(&version_json) {
                    Ok(version) => version,
                    Err(e) => {
                        debug!(package = %name, error = %e, "failed to deserialize package version");
                        return None;
                    }
                };

                Some(FetchedVersion {
                    version: v.version.clone(),
                    require: v
                        .require
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                    require_dev: v
                        .require_dev
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                    replace: v
                        .replace
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                    provide: v
                        .provide
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                    suggest: v
                        .suggest
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                    dist_url: v.dist.as_ref().map(|d| d.url.clone()),
                    dist_type: v.dist.as_ref().map(|d| d.dist_type.clone()),
                    dist_shasum: v.dist.as_ref().and_then(|d| d.shasum.clone()),
                    source_url: v.source.as_ref().map(|s| s.url.clone()),
                    source_type: v.source.as_ref().map(|s| s.source_type.clone()),
                    source_reference: v.source.as_ref().map(|s| s.reference.clone()),
                    // Full metadata
                    package_type: v.package_type.clone(),
                    description: v.description.clone(),
                    homepage: v.homepage.clone(),
                    license: v.license.clone(),
                    authors: v.authors.as_ref().and_then(|a| sonic_rs::to_value(a).ok()),
                    keywords: v.keywords.clone(),
                    time: v.time.clone(),
                    autoload: v.autoload.as_ref().and_then(|a| sonic_rs::to_value(a).ok()),
                    autoload_dev: v
                        .autoload_dev
                        .as_ref()
                        .and_then(|a| sonic_rs::to_value(a).ok()),
                    extra: v.extra.clone(),
                    support: v.support.as_ref().and_then(|s| sonic_rs::to_value(s).ok()),
                    funding: v.funding.as_ref().and_then(|f| sonic_rs::to_value(f).ok()),
                    notification_url: v.notification_url.clone(),
                    bin: v.bin.clone(),
                })
            })
            .collect();

        if fetched_versions.is_empty() {
            None
        } else {
            Some(FetchedPackage {
                name: name.to_string(),
                versions: fetched_versions,
            })
        }
    }
}

fn expand_versions_for_deserialize(versions: &[Value], minified: bool) -> Vec<Value> {
    if !minified {
        return versions.to_vec();
    }

    let mut expanded = Vec::with_capacity(versions.len());
    let mut previous: BTreeMap<String, Value> = BTreeMap::new();

    for version in versions {
        let Some(fields) = version.as_object() else {
            continue;
        };

        let mut merged = previous.clone();
        for (key, value) in fields {
            if value.as_str() == Some("__unset") {
                merged.remove(key);
            } else {
                merged.insert(key.to_string(), value.clone());
            }
        }

        previous = merged.clone();
        if let Ok(expanded_value) = sonic_rs::to_value(&merged) {
            expanded.push(expanded_value);
        }
    }

    expanded
}

impl TurboFetcher for Fetcher {
    fn fetch(
        &self,
        name: String,
    ) -> Pin<Box<dyn std::future::Future<Output = Option<FetchedPackage>> + Send + '_>> {
        Box::pin(async move { self.fetch_impl(&name).await })
    }
}

fn parse_string_map(json: &Value, key: &str) -> Vec<(String, String)> {
    json.get(key)
        .and_then(Value::as_object)
        .map(|object| {
            object
                .iter()
                .filter_map(|(k, v)| v.as_str().map(|value| (k.to_string(), value.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

fn parse_string_or_array(json: &Value, key: &str) -> Option<Vec<String>> {
    let value = json.get(key)?;

    if let Some(s) = value.as_str() {
        return Some(vec![s.to_string()]);
    }

    value.as_array().map(|arr| {
        arr.iter()
            .filter_map(Value::as_str)
            .map(ToString::to_string)
            .collect::<Vec<String>>()
    })
}

fn vcs_dist_url(provider_name: &str, url: &Url, reference: &str) -> Option<String> {
    match provider_name {
        "GitHub" => {
            let (owner, repo) = parse_vcs_url(url)?;
            Some(format!(
                "https://api.github.com/repos/{owner}/{repo}/zipball/{reference}"
            ))
        }
        _ => None,
    }
}

fn extract_vcs_repository_urls(composer: &Value) -> Vec<String> {
    let mut urls = Vec::new();

    let repositories = match composer.get("repositories") {
        Some(repositories) => repositories,
        None => return urls,
    };

    let mut push_if_vcs = |repo: &Value| {
        let repo_type = repo.get("type").and_then(Value::as_str);
        let repo_url = repo.get("url").and_then(Value::as_str);
        if repo_type == Some("vcs")
            && let Some(url) = repo_url
        {
            urls.push(url.to_string());
        }
    };

    if let Some(arr) = repositories.as_array() {
        for repo in arr {
            push_if_vcs(repo);
        }
    } else if let Some(obj) = repositories.as_object() {
        for (_, repo) in obj {
            if repo.is_object() {
                push_if_vcs(repo);
            }
        }
    }

    urls.sort_unstable();
    urls.dedup();
    urls
}

fn extract_root_constraints(composer: &Value) -> HashMap<String, String> {
    let mut constraints = HashMap::new();

    for key in ["require", "require-dev"] {
        let Some(reqs) = composer.get(key).and_then(Value::as_object) else {
            continue;
        };
        for (name, constraint) in reqs {
            let Some(constraint) = constraint.as_str() else {
                continue;
            };
            if is_platform_package(name) {
                continue;
            }
            constraints.insert(name.to_string(), constraint.to_string());
        }
    }

    constraints
}

fn is_platform_package(name: &str) -> bool {
    libretto_core::is_platform_package_name(name)
}

fn is_dev_constraint(constraint: &str) -> bool {
    let value = constraint.trim().to_ascii_lowercase();
    value.contains("dev-") || value.contains("@dev") || value.ends_with("-dev")
}

// --- Packagist JSON types ---

#[derive(Debug, serde::Deserialize)]
struct PackagistVersion {
    version: String,
    #[serde(default, deserialize_with = "deserialize_deps")]
    require: HashMap<String, String>,
    #[serde(default, rename = "require-dev", deserialize_with = "deserialize_deps")]
    require_dev: HashMap<String, String>,
    #[serde(default, deserialize_with = "deserialize_deps")]
    replace: HashMap<String, String>,
    #[serde(default, deserialize_with = "deserialize_deps")]
    provide: HashMap<String, String>,
    #[serde(default, deserialize_with = "deserialize_deps")]
    suggest: HashMap<String, String>,
    #[serde(default)]
    dist: Option<PackagistDist>,
    #[serde(default)]
    source: Option<PackagistSource>,
    // Additional metadata - these fields are part of the Packagist API response
    // and are kept for completeness/debugging even if not directly used
    #[serde(default)]
    #[allow(dead_code)] // Part of API response, used for Debug trait
    name: Option<String>,
    #[serde(default, rename = "type")]
    package_type: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    homepage: Option<String>,
    #[serde(default, deserialize_with = "deserialize_string_vec")]
    license: Option<Vec<String>>,
    #[serde(default, deserialize_with = "deserialize_authors")]
    authors: Option<Vec<PackagistAuthor>>,
    #[serde(default, deserialize_with = "deserialize_string_vec")]
    keywords: Option<Vec<String>>,
    #[serde(default)]
    time: Option<String>,
    #[serde(default, deserialize_with = "deserialize_autoload")]
    autoload: Option<PackagistAutoload>,
    #[serde(
        default,
        rename = "autoload-dev",
        deserialize_with = "deserialize_autoload"
    )]
    autoload_dev: Option<PackagistAutoload>,
    #[serde(default)]
    extra: Option<sonic_rs::Value>,
    #[serde(default)]
    support: Option<PackagistSupport>,
    #[serde(default, deserialize_with = "deserialize_funding")]
    funding: Option<Vec<PackagistFunding>>,
    #[serde(default, rename = "notification-url")]
    notification_url: Option<String>,
    #[serde(default, deserialize_with = "deserialize_string_vec")]
    bin: Option<Vec<String>>,
}

fn deserialize_deps<'de, D>(deserializer: D) -> Result<HashMap<String, String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{self, Visitor};

    struct DepsVisitor;

    impl<'de> Visitor<'de> for DepsVisitor {
        type Value = HashMap<String, String>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a map or \"__unset\" string")
        }

        fn visit_str<E>(self, _v: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(HashMap::new())
        }

        fn visit_none<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(HashMap::new())
        }

        fn visit_unit<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(HashMap::new())
        }

        fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
        where
            M: de::MapAccess<'de>,
        {
            let mut deps = HashMap::new();
            while let Some((key, value)) = map.next_entry::<String, String>()? {
                deps.insert(key, value);
            }
            Ok(deps)
        }
    }

    deserializer.deserialize_any(DepsVisitor)
}

#[derive(Debug, serde::Deserialize)]
struct PackagistDist {
    #[serde(rename = "type")]
    dist_type: String,
    url: String,
    shasum: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct PackagistSource {
    #[serde(rename = "type")]
    source_type: String,
    url: String,
    reference: String,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
struct PackagistAuthor {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    homepage: Option<String>,
    #[serde(default)]
    role: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
struct PackagistAutoload {
    #[serde(
        default,
        rename = "psr-4",
        deserialize_with = "deserialize_autoload_map"
    )]
    psr4: Option<HashMap<String, sonic_rs::Value>>,
    #[serde(
        default,
        rename = "psr-0",
        deserialize_with = "deserialize_autoload_map"
    )]
    psr0: Option<HashMap<String, sonic_rs::Value>>,
    #[serde(default, deserialize_with = "deserialize_string_vec")]
    classmap: Option<Vec<String>>,
    #[serde(default, deserialize_with = "deserialize_string_vec")]
    files: Option<Vec<String>>,
    #[serde(
        default,
        rename = "exclude-from-classmap",
        deserialize_with = "deserialize_string_vec"
    )]
    exclude_from_classmap: Option<Vec<String>>,
}

fn deserialize_autoload<'de, D>(deserializer: D) -> Result<Option<PackagistAutoload>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{self, Visitor};

    struct AutoloadVisitor;

    impl<'de> Visitor<'de> for AutoloadVisitor {
        type Value = Option<PackagistAutoload>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a struct, null, or \"__unset\" string")
        }

        fn visit_str<E>(self, _v: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            // "__unset" or any string -> None
            Ok(None)
        }

        fn visit_none<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }

        fn visit_unit<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }

        fn visit_map<M>(self, map: M) -> Result<Self::Value, M::Error>
        where
            M: de::MapAccess<'de>,
        {
            // Deserialize the struct using serde's Deserialize implementation
            let autoload =
                serde::Deserialize::deserialize(de::value::MapAccessDeserializer::new(map))?;
            Ok(Some(autoload))
        }
    }

    deserializer.deserialize_any(AutoloadVisitor)
}

fn deserialize_autoload_map<'de, D>(
    deserializer: D,
) -> Result<Option<HashMap<String, sonic_rs::Value>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{self, Visitor};

    struct AutoloadMapVisitor;

    impl<'de> Visitor<'de> for AutoloadMapVisitor {
        type Value = Option<HashMap<String, sonic_rs::Value>>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a map, null, or \"__unset\" string")
        }

        fn visit_str<E>(self, _v: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }

        fn visit_none<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }

        fn visit_unit<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }

        fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
        where
            M: de::MapAccess<'de>,
        {
            let mut result = HashMap::new();
            while let Some((key, value)) = map.next_entry::<String, sonic_rs::Value>()? {
                result.insert(key, value);
            }
            Ok(Some(result))
        }
    }

    deserializer.deserialize_any(AutoloadMapVisitor)
}

fn deserialize_string_vec<'de, D>(deserializer: D) -> Result<Option<Vec<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{self, Visitor};

    struct StringVecVisitor;

    impl<'de> Visitor<'de> for StringVecVisitor {
        type Value = Option<Vec<String>>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a sequence, null, or \"__unset\" string")
        }

        fn visit_str<E>(self, _v: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }

        fn visit_none<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }

        fn visit_unit<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }

        fn visit_seq<S>(self, mut seq: S) -> Result<Self::Value, S::Error>
        where
            S: de::SeqAccess<'de>,
        {
            let mut result = Vec::new();
            // Handle both strings and nested arrays (e.g., classmap can be [["path"]] or ["path"])
            loop {
                // Try to get a string first
                match seq.next_element::<String>() {
                    Ok(Some(s)) => {
                        result.push(s);
                    }
                    Ok(None) => break,
                    Err(_) => {
                        // Not a string - try as nested array of strings
                        if let Ok(Some(arr)) = seq.next_element::<Vec<String>>() {
                            result.extend(arr);
                        } else {
                            // Skip unknown element
                            let _ = seq.next_element::<serde::de::IgnoredAny>();
                        }
                    }
                }
            }
            Ok(Some(result))
        }
    }

    deserializer.deserialize_any(StringVecVisitor)
}

fn deserialize_authors<'de, D>(deserializer: D) -> Result<Option<Vec<PackagistAuthor>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{self, Visitor};

    struct AuthorsVisitor;

    impl<'de> Visitor<'de> for AuthorsVisitor {
        type Value = Option<Vec<PackagistAuthor>>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a sequence, null, or \"__unset\" string")
        }

        fn visit_str<E>(self, _v: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }

        fn visit_none<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }

        fn visit_unit<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }

        fn visit_seq<S>(self, mut seq: S) -> Result<Self::Value, S::Error>
        where
            S: de::SeqAccess<'de>,
        {
            let mut result = Vec::new();
            while let Some(value) = seq.next_element::<PackagistAuthor>()? {
                result.push(value);
            }
            Ok(Some(result))
        }
    }

    deserializer.deserialize_any(AuthorsVisitor)
}

fn deserialize_funding<'de, D>(deserializer: D) -> Result<Option<Vec<PackagistFunding>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{self, Visitor};

    struct FundingVisitor;

    impl<'de> Visitor<'de> for FundingVisitor {
        type Value = Option<Vec<PackagistFunding>>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a sequence, null, or \"__unset\" string")
        }

        fn visit_str<E>(self, _v: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }

        fn visit_none<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }

        fn visit_unit<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }

        fn visit_seq<S>(self, mut seq: S) -> Result<Self::Value, S::Error>
        where
            S: de::SeqAccess<'de>,
        {
            let mut result = Vec::new();
            while let Some(value) = seq.next_element::<PackagistFunding>()? {
                result.push(value);
            }
            Ok(Some(result))
        }
    }

    deserializer.deserialize_any(FundingVisitor)
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
struct PackagistSupport {
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    issues: Option<String>,
    #[serde(default)]
    forum: Option<String>,
    #[serde(default)]
    wiki: Option<String>,
    #[serde(default)]
    irc: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    docs: Option<String>,
    #[serde(default)]
    rss: Option<String>,
    #[serde(default)]
    chat: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
struct PackagistFunding {
    #[serde(default, rename = "type")]
    funding_type: Option<String>,
    #[serde(default)]
    url: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_vcs_repositories_from_array_form() {
        let composer: Value = sonic_rs::from_str(
            r#"{
                "repositories": [
                    {"type": "composer", "url": "https://packagist.org"},
                    {"type": "vcs", "url": "https://github.com/example/repo-a"},
                    {"type": "vcs", "url": "https://github.com/example/repo-b"}
                ]
            }"#,
        )
        .expect("valid json");

        let urls = extract_vcs_repository_urls(&composer);
        assert_eq!(urls.len(), 2);
        assert!(urls.contains(&"https://github.com/example/repo-a".to_string()));
        assert!(urls.contains(&"https://github.com/example/repo-b".to_string()));
    }

    #[test]
    fn extract_vcs_repositories_from_object_form() {
        let composer: Value = sonic_rs::from_str(
            r#"{
                "repositories": {
                    "packagist.org": false,
                    "custom": {"type": "vcs", "url": "https://github.com/example/repo-c"}
                }
            }"#,
        )
        .expect("valid json");

        let urls = extract_vcs_repository_urls(&composer);
        assert_eq!(urls, vec!["https://github.com/example/repo-c".to_string()]);
    }

    #[test]
    fn extract_root_constraints_from_require_sections() {
        let composer: Value = sonic_rs::from_str(
            r#"{
                "require": {
                    "php": "^8.2",
                    "acme/pkg-a": "dev-main",
                    "acme/pkg-b": "^1.0",
                    "php-open-source-saver/jwt-auth": "^2.0"
                },
                "require-dev": {
                    "ext-json": "*",
                    "acme/pkg-c": "^2.0@dev"
                }
            }"#,
        )
        .expect("valid json");

        let constraints = extract_root_constraints(&composer);
        assert_eq!(constraints.get("acme/pkg-a"), Some(&"dev-main".to_string()));
        assert_eq!(constraints.get("acme/pkg-b"), Some(&"^1.0".to_string()));
        assert_eq!(
            constraints.get("php-open-source-saver/jwt-auth"),
            Some(&"^2.0".to_string())
        );
        assert_eq!(constraints.get("acme/pkg-c"), Some(&"^2.0@dev".to_string()));
        assert!(!constraints.contains_key("php"));
        assert!(!constraints.contains_key("ext-json"));
    }

    #[test]
    fn detects_dev_constraints() {
        assert!(is_dev_constraint("dev-main"));
        assert!(is_dev_constraint("^1.2@dev"));
        assert!(is_dev_constraint("1.2.x-dev"));
        assert!(!is_dev_constraint("^1.2"));
    }

    #[test]
    fn expands_minified_versions_with_inheritance_and_unset() {
        let fetcher = Fetcher::new().expect("fetcher should initialize");
        let body = r#"{
            "minified": "composer/2.0",
            "packages": {
                "acme/pkg": [
                    {
                        "version": "2.0.0",
                        "require": {
                            "foo/bar": "^1.0",
                            "php": "^8.2"
                        },
                        "replace": {
                            "acme/virtual": "self.version"
                        },
                        "dist": {
                            "type": "zip",
                            "url": "https://example.test/acme/pkg/2.0.0.zip"
                        },
                        "source": {
                            "type": "git",
                            "url": "https://example.test/acme/pkg.git",
                            "reference": "ref-200"
                        }
                    },
                    {
                        "version": "1.9.0",
                        "dist": {
                            "type": "zip",
                            "url": "https://example.test/acme/pkg/1.9.0.zip"
                        },
                        "source": {
                            "type": "git",
                            "url": "https://example.test/acme/pkg.git",
                            "reference": "ref-190"
                        }
                    },
                    {
                        "version": "1.8.0",
                        "require": "__unset",
                        "dist": {
                            "type": "zip",
                            "url": "https://example.test/acme/pkg/1.8.0.zip"
                        },
                        "source": {
                            "type": "git",
                            "url": "https://example.test/acme/pkg.git",
                            "reference": "ref-180"
                        }
                    }
                ]
            }
        }"#;

        let package = fetcher
            .parse_response("acme/pkg", body.as_bytes())
            .expect("minified payload should parse");

        assert_eq!(package.versions.len(), 3);

        // `1.9.0` omits require in minified metadata and must inherit it from `2.0.0`.
        assert_eq!(package.versions[1].version, "1.9.0");
        assert_eq!(
            package.versions[1]
                .require
                .iter()
                .find(|(name, _)| name == "foo/bar")
                .map(|(_, constraint)| constraint.as_str()),
            Some("^1.0")
        );
        assert_eq!(
            package.versions[1]
                .replace
                .iter()
                .find(|(name, _)| name == "acme/virtual")
                .map(|(_, constraint)| constraint.as_str()),
            Some("self.version")
        );

        // `__unset` must clear inherited fields for older versions.
        assert_eq!(package.versions[2].version, "1.8.0");
        assert!(package.versions[2].require.is_empty());
    }
}
