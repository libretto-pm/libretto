//! Fast Packagist fetcher with metadata caching.
//!
//! Uses reqwest with HTTP/2, connection pooling, and aggressive timeouts.
//! Caches package metadata locally for fast resolution on subsequent runs.

use libretto_resolver::turbo::{FetchedPackage, FetchedVersion, TurboFetcher};
use reqwest::Client;
use std::collections::HashMap;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tracing::{debug, trace};

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
    requests: AtomicU64,
    bytes: AtomicU64,
    cache_hits: AtomicU64,
}

impl Fetcher {
    pub fn new() -> Result<Self, reqwest::Error> {
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

        Ok(Self {
            client,
            base_url: "https://repo.packagist.org/p2".to_string(),
            cache_dir,
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

    /// Get ETag cache path
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

    /// Read stored ETag for a package.
    fn read_etag(&self, name: &str) -> Option<String> {
        std::fs::read_to_string(self.etag_path(name)).ok()
    }

    /// Write response data + ETag to cache, touch the modification time.
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

    async fn fetch_impl(&self, name: &str) -> Option<FetchedPackage> {
        // If cache is very fresh (< 5 min), skip network entirely
        if self.is_cache_fresh(name) {
            if let Some(cached) = self.read_cache(name) {
                self.cache_hits.fetch_add(1, Ordering::Relaxed);
                trace!(package = %name, "cache fresh, skipping network");
                return self.parse_response(name, &cached);
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
                    return self.parse_response(name, &cached);
                }
                return None;
            }
        };

        // 304 Not Modified — cache is still valid
        if response.status() == reqwest::StatusCode::NOT_MODIFIED {
            self.cache_hits.fetch_add(1, Ordering::Relaxed);
            self.touch_cache(name);
            trace!(package = %name, "304 not modified");
            if let Some(cached) = self.read_cache(name) {
                return self.parse_response(name, &cached);
            }
            return None;
        }

        if !response.status().is_success() {
            debug!(package = %name, status = %response.status(), "non-success status");
            return None;
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
                return None;
            }
        };

        self.bytes.fetch_add(bytes.len() as u64, Ordering::Relaxed);

        // Cache the response
        self.write_cache(name, &bytes, etag.as_deref());

        let bytes = bytes.to_vec();
        self.parse_response(name, &bytes)
    }

    fn parse_response(&self, name: &str, bytes: &[u8]) -> Option<FetchedPackage> {
        let json: PackagistResponse = match sonic_rs::from_slice(&bytes) {
            Ok(j) => j,
            Err(e) => {
                debug!(package = %name, error = %e, "JSON parse failed");
                return None;
            }
        };

        let versions = json.packages.get(name)?;

        let fetched_versions: Vec<FetchedVersion> = versions
            .iter()
            .filter_map(|v| {
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

impl TurboFetcher for Fetcher {
    fn fetch(
        &self,
        name: String,
    ) -> Pin<Box<dyn std::future::Future<Output = Option<FetchedPackage>> + Send + '_>> {
        Box::pin(async move { self.fetch_impl(&name).await })
    }
}

// --- Packagist JSON types ---

#[derive(Debug, serde::Deserialize)]
struct PackagistResponse {
    packages: HashMap<String, Vec<PackagistVersion>>,
}

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
