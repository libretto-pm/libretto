//! GitHub API client for fetching repository metadata.

use crate::cache::{DEFAULT_METADATA_TTL, RepositoryCache};
use crate::client::{AuthType, HttpClient, HttpClientConfig};
use crate::error::{RepositoryError, Result};
use crate::providers::{VcsProvider, parse_vcs_url};
use serde::Deserialize;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tracing::debug;
use url::Url;

/// GitHub API base URL.
pub const GITHUB_API_URL: &str = "https://api.github.com/";

/// GitHub configuration.
#[derive(Debug, Clone)]
pub struct GitHubConfig {
    /// API base URL (for GitHub Enterprise).
    pub api_url: Url,
    /// OAuth token for authenticated requests.
    pub token: Option<String>,
    /// HTTP client configuration.
    pub http_config: HttpClientConfig,
}

impl Default for GitHubConfig {
    fn default() -> Self {
        Self {
            api_url: Url::parse(GITHUB_API_URL).expect("valid URL"),
            token: None,
            http_config: HttpClientConfig {
                // GitHub rate limit is 60/hour unauthenticated, 5000/hour authenticated
                rate_limit_per_host: 10, // Conservative default
                ..Default::default()
            },
        }
    }
}

impl GitHubConfig {
    /// Create configuration for GitHub Enterprise.
    #[must_use]
    pub fn enterprise(api_url: Url, token: String) -> Self {
        Self {
            api_url,
            token: Some(token),
            ..Default::default()
        }
    }

    /// Create configuration with token.
    #[must_use]
    pub fn with_token(token: String) -> Self {
        Self {
            token: Some(token),
            http_config: HttpClientConfig {
                // Higher rate limit with token
                rate_limit_per_host: 50,
                ..Default::default()
            },
            ..Default::default()
        }
    }
}

/// GitHub API client.
pub struct GitHubClient {
    config: GitHubConfig,
    http: Arc<HttpClient>,
    cache: Arc<RepositoryCache>,
}

impl std::fmt::Debug for GitHubClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitHubClient")
            .field("api_url", &self.config.api_url)
            .finish()
    }
}

impl GitHubClient {
    /// Create a new GitHub client.
    ///
    /// # Errors
    /// Returns error if HTTP client cannot be created.
    pub fn new() -> Result<Self> {
        Self::with_config(GitHubConfig::default())
    }

    /// Create a new GitHub client with custom configuration.
    ///
    /// # Errors
    /// Returns error if HTTP client cannot be created.
    pub fn with_config(config: GitHubConfig) -> Result<Self> {
        let mut http_config = config.http_config.clone();
        http_config.user_agent = format!(
            "Libretto/{} (mailto=libretto@example.com)",
            env!("CARGO_PKG_VERSION")
        );

        let http =
            HttpClient::with_config(http_config).map_err(|e| RepositoryError::InvalidConfig {
                message: format!("HTTP client creation failed: {e}"),
            })?;

        if let Some(ref token) = config.token
            && let Some(host) = config.api_url.host_str()
        {
            http.set_auth(host, AuthType::Bearer(token.clone()));
        }

        Ok(Self {
            config,
            http: Arc::new(http),
            cache: Arc::new(RepositoryCache::new()),
        })
    }

    /// Create client with shared cache.
    #[must_use]
    pub fn with_cache(mut self, cache: Arc<RepositoryCache>) -> Self {
        self.cache = cache;
        self
    }

    /// Build API URL for a repository endpoint.
    fn api_url(&self, owner: &str, repo: &str, endpoint: &str) -> Result<Url> {
        let path = format!("repos/{owner}/{repo}/{endpoint}");
        self.config
            .api_url
            .join(&path)
            .map_err(|e| RepositoryError::InvalidUrl {
                url: self.config.api_url.to_string(),
                message: e.to_string(),
            })
    }

    /// Get file contents from a repository.
    pub async fn get_file_contents(
        &self,
        owner: &str,
        repo: &str,
        path: &str,
        reference: &str,
    ) -> Result<String> {
        let url = self.api_url(owner, repo, &format!("contents/{path}"))?;
        let cache_key = format!("github:{owner}/{repo}/{path}@{reference}");

        // Check cache
        if let Some(data) = self.cache.get_metadata(&cache_key)
            && let Ok(content) = String::from_utf8(data.to_vec())
        {
            debug!(owner, repo, path, reference, "cache hit");
            return Ok(content);
        }

        let mut url = url;
        url.query_pairs_mut().append_pair("ref", reference);

        let response = self.http.get(&url).await?;

        let content_response: GitHubContentResponse = sonic_rs::from_slice(&response.body)
            .map_err(|e| RepositoryError::ParseError {
                source: url.to_string(),
                message: e.to_string(),
            })?;

        let content =
            match content_response.encoding.as_deref() {
                Some("base64") => {
                    let decoded = base64_decode(&content_response.content.replace('\n', ""))
                        .map_err(|e| RepositoryError::ParseError {
                            source: url.to_string(),
                            message: format!("base64 decode failed: {e}"),
                        })?;
                    String::from_utf8(decoded).map_err(|e| RepositoryError::ParseError {
                        source: url.to_string(),
                        message: format!("UTF-8 decode failed: {e}"),
                    })?
                }
                _ => content_response.content,
            };

        // Cache result
        let _ = self
            .cache
            .put_metadata(&cache_key, content.as_bytes(), DEFAULT_METADATA_TTL, None);

        Ok(content)
    }

    /// List repository tags.
    pub async fn list_tags(&self, owner: &str, repo: &str) -> Result<Vec<GitHubTag>> {
        let url = self.api_url(owner, repo, "tags")?;
        let cache_key = format!("github:{owner}/{repo}/tags");

        if let Some(data) = self.cache.get_metadata(&cache_key)
            && let Ok(tags) = sonic_rs::from_slice::<Vec<GitHubTag>>(&data)
        {
            return Ok(tags);
        }

        let response = self.http.get(&url).await?;

        let tags: Vec<GitHubTag> =
            sonic_rs::from_slice(&response.body).map_err(|e| RepositoryError::ParseError {
                source: url.to_string(),
                message: e.to_string(),
            })?;

        if let Ok(data) = sonic_rs::to_vec(&tags) {
            let _ = self
                .cache
                .put_metadata(&cache_key, &data, DEFAULT_METADATA_TTL, None);
        }

        Ok(tags)
    }

    /// List repository branches.
    pub async fn list_branches(&self, owner: &str, repo: &str) -> Result<Vec<GitHubBranch>> {
        let url = self.api_url(owner, repo, "branches")?;
        let cache_key = format!("github:{owner}/{repo}/branches");

        if let Some(data) = self.cache.get_metadata(&cache_key)
            && let Ok(branches) = sonic_rs::from_slice::<Vec<GitHubBranch>>(&data)
        {
            return Ok(branches);
        }

        let response = self.http.get(&url).await?;

        let branches: Vec<GitHubBranch> =
            sonic_rs::from_slice(&response.body).map_err(|e| RepositoryError::ParseError {
                source: url.to_string(),
                message: e.to_string(),
            })?;

        if let Ok(data) = sonic_rs::to_vec(&branches) {
            let _ = self
                .cache
                .put_metadata(&cache_key, &data, DEFAULT_METADATA_TTL, None);
        }

        Ok(branches)
    }

    /// Get repository information.
    pub async fn get_repository(&self, owner: &str, repo: &str) -> Result<GitHubRepository> {
        let path = format!("repos/{owner}/{repo}");
        let url = self
            .config
            .api_url
            .join(&path)
            .map_err(|e| RepositoryError::InvalidUrl {
                url: self.config.api_url.to_string(),
                message: e.to_string(),
            })?;

        let cache_key = format!("github:{owner}/{repo}/info");

        if let Some(data) = self.cache.get_metadata(&cache_key)
            && let Ok(repo_info) = sonic_rs::from_slice::<GitHubRepository>(&data)
        {
            return Ok(repo_info);
        }

        let response = self.http.get(&url).await?;

        let repo_info: GitHubRepository =
            sonic_rs::from_slice(&response.body).map_err(|e| RepositoryError::ParseError {
                source: url.to_string(),
                message: e.to_string(),
            })?;

        if let Ok(data) = sonic_rs::to_vec(&repo_info) {
            let _ = self
                .cache
                .put_metadata(&cache_key, &data, DEFAULT_METADATA_TTL, None);
        }

        Ok(repo_info)
    }

    /// List releases.
    pub async fn list_releases(&self, owner: &str, repo: &str) -> Result<Vec<GitHubRelease>> {
        let url = self.api_url(owner, repo, "releases")?;
        let cache_key = format!("github:{owner}/{repo}/releases");

        if let Some(data) = self.cache.get_metadata(&cache_key)
            && let Ok(releases) = sonic_rs::from_slice::<Vec<GitHubRelease>>(&data)
        {
            return Ok(releases);
        }

        let response = self.http.get(&url).await?;

        let releases: Vec<GitHubRelease> =
            sonic_rs::from_slice(&response.body).map_err(|e| RepositoryError::ParseError {
                source: url.to_string(),
                message: e.to_string(),
            })?;

        if let Ok(data) = sonic_rs::to_vec(&releases) {
            let _ = self
                .cache
                .put_metadata(&cache_key, &data, DEFAULT_METADATA_TTL, None);
        }

        Ok(releases)
    }

    /// Get rate limit status.
    pub async fn get_rate_limit(&self) -> Result<GitHubRateLimit> {
        let url =
            self.config
                .api_url
                .join("rate_limit")
                .map_err(|e| RepositoryError::InvalidUrl {
                    url: self.config.api_url.to_string(),
                    message: e.to_string(),
                })?;

        let response = self.http.get(&url).await?;

        let rate_limit: GitHubRateLimitResponse =
            sonic_rs::from_slice(&response.body).map_err(|e| RepositoryError::ParseError {
                source: url.to_string(),
                message: e.to_string(),
            })?;

        Ok(rate_limit.rate)
    }
}

impl VcsProvider for GitHubClient {
    fn name(&self) -> &'static str {
        "GitHub"
    }

    fn can_handle(&self, url: &Url) -> bool {
        url.host_str()
            .is_some_and(|h| h.contains("github.com") || h.contains("github."))
    }

    fn fetch_composer_json<'a>(
        &'a self,
        url: &'a Url,
        reference: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>> {
        Box::pin(async move {
            let (owner, repo) = parse_vcs_url(url).ok_or_else(|| RepositoryError::InvalidUrl {
                url: url.to_string(),
                message: "Could not parse owner/repo from URL".into(),
            })?;

            self.get_file_contents(&owner, &repo, "composer.json", reference)
                .await
        })
    }

    fn list_tags<'a>(
        &'a self,
        url: &'a Url,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<String>>> + Send + 'a>> {
        Box::pin(async move {
            let (owner, repo) = parse_vcs_url(url).ok_or_else(|| RepositoryError::InvalidUrl {
                url: url.to_string(),
                message: "Could not parse owner/repo from URL".into(),
            })?;

            let tags = self.list_tags(&owner, &repo).await?;
            Ok(tags.into_iter().map(|t| t.name).collect())
        })
    }

    fn list_branches<'a>(
        &'a self,
        url: &'a Url,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<String>>> + Send + 'a>> {
        Box::pin(async move {
            let (owner, repo) = parse_vcs_url(url).ok_or_else(|| RepositoryError::InvalidUrl {
                url: url.to_string(),
                message: "Could not parse owner/repo from URL".into(),
            })?;

            let branches = self.list_branches(&owner, &repo).await?;
            Ok(branches.into_iter().map(|b| b.name).collect())
        })
    }

    fn get_default_branch<'a>(
        &'a self,
        url: &'a Url,
    ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>> {
        Box::pin(async move {
            let (owner, repo) = parse_vcs_url(url).ok_or_else(|| RepositoryError::InvalidUrl {
                url: url.to_string(),
                message: "Could not parse owner/repo from URL".into(),
            })?;

            let repo_info = self.get_repository(&owner, &repo).await?;
            Ok(repo_info.default_branch)
        })
    }
}

// GitHub API response types

/// GitHub content response.
#[derive(Debug, Deserialize)]
struct GitHubContentResponse {
    content: String,
    encoding: Option<String>,
}

/// GitHub tag.
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct GitHubTag {
    /// Tag name.
    pub name: String,
    /// Commit info.
    pub commit: GitHubCommitRef,
}

/// GitHub commit reference.
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct GitHubCommitRef {
    /// Commit SHA.
    pub sha: String,
    /// Commit URL.
    pub url: String,
}

/// GitHub branch.
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct GitHubBranch {
    /// Branch name.
    pub name: String,
    /// Commit info.
    pub commit: GitHubCommitRef,
    /// Whether protected.
    #[serde(default)]
    pub protected: bool,
}

/// GitHub repository info.
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct GitHubRepository {
    /// Repository ID.
    pub id: u64,
    /// Repository name.
    pub name: String,
    /// Full name (owner/repo).
    pub full_name: String,
    /// Description.
    #[serde(default)]
    pub description: Option<String>,
    /// Whether private.
    #[serde(default)]
    pub private: bool,
    /// HTML URL.
    pub html_url: String,
    /// Clone URL.
    pub clone_url: String,
    /// Default branch.
    pub default_branch: String,
    /// Star count.
    #[serde(default)]
    pub stargazers_count: u64,
    /// Fork count.
    #[serde(default)]
    pub forks_count: u64,
    /// Open issues count.
    #[serde(default)]
    pub open_issues_count: u64,
    /// Whether archived.
    #[serde(default)]
    pub archived: bool,
    /// License info.
    #[serde(default)]
    pub license: Option<GitHubLicense>,
}

/// GitHub license info.
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct GitHubLicense {
    /// License key.
    pub key: String,
    /// License name.
    pub name: String,
    /// SPDX ID.
    #[serde(default)]
    pub spdx_id: Option<String>,
}

/// GitHub release.
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct GitHubRelease {
    /// Release ID.
    pub id: u64,
    /// Tag name.
    pub tag_name: String,
    /// Release name.
    #[serde(default)]
    pub name: Option<String>,
    /// Whether prerelease.
    #[serde(default)]
    pub prerelease: bool,
    /// Whether draft.
    #[serde(default)]
    pub draft: bool,
    /// HTML URL.
    pub html_url: String,
    /// Tarball URL.
    #[serde(default)]
    pub tarball_url: Option<String>,
    /// Zipball URL.
    #[serde(default)]
    pub zipball_url: Option<String>,
    /// Created at.
    pub created_at: String,
    /// Published at.
    #[serde(default)]
    pub published_at: Option<String>,
}

/// GitHub rate limit response.
#[derive(Debug, Deserialize)]
struct GitHubRateLimitResponse {
    rate: GitHubRateLimit,
}

/// GitHub rate limit info.
#[derive(Debug, Clone, Deserialize)]
pub struct GitHubRateLimit {
    /// Request limit.
    pub limit: u32,
    /// Remaining requests.
    pub remaining: u32,
    /// Reset timestamp.
    pub reset: u64,
    /// Used requests.
    #[serde(default)]
    pub used: u32,
}

/// Simple base64 decoder.
fn base64_decode(input: &str) -> std::result::Result<Vec<u8>, &'static str> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    fn char_to_value(c: u8) -> Option<u8> {
        ALPHABET.iter().position(|&x| x == c).map(|p| p as u8)
    }

    let input = input.as_bytes();
    let mut output = Vec::with_capacity(input.len() * 3 / 4);
    let mut buffer = 0u32;
    let mut bits = 0;

    for &byte in input {
        if byte == b'=' {
            continue;
        }
        if byte.is_ascii_whitespace() {
            continue;
        }

        let value = char_to_value(byte).ok_or("invalid base64 character")?;
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;

        if bits >= 8 {
            bits -= 8;
            output.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_base64_decode() {
        assert_eq!(base64_decode("SGVsbG8=").unwrap(), b"Hello");
        assert_eq!(base64_decode("V29ybGQ=").unwrap(), b"World");
        assert_eq!(base64_decode("dGVzdA==").unwrap(), b"test");
    }

    #[test]
    fn test_config_default() {
        let config = GitHubConfig::default();
        assert_eq!(config.api_url.as_str(), GITHUB_API_URL);
        assert!(config.token.is_none());
    }

    #[test]
    fn test_config_with_token() {
        let config = GitHubConfig::with_token("token123".into());
        assert_eq!(config.token, Some("token123".into()));
        // Higher rate limit with token
        assert!(
            config.http_config.rate_limit_per_host
                > GitHubConfig::default().http_config.rate_limit_per_host
        );
    }
}
