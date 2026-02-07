//! GitLab API client for fetching repository metadata.

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

/// GitLab API base URL.
pub const GITLAB_API_URL: &str = "https://gitlab.com/api/v4/";

/// GitLab configuration.
#[derive(Debug, Clone)]
pub struct GitLabConfig {
    /// API base URL (for self-hosted GitLab).
    pub api_url: Url,
    /// Personal access token or OAuth token.
    pub token: Option<String>,
    /// HTTP client configuration.
    pub http_config: HttpClientConfig,
}

impl Default for GitLabConfig {
    fn default() -> Self {
        Self {
            api_url: Url::parse(GITLAB_API_URL).expect("valid URL"),
            token: None,
            http_config: HttpClientConfig::default(),
        }
    }
}

impl GitLabConfig {
    /// Create configuration for self-hosted GitLab.
    #[must_use]
    pub fn self_hosted(base_url: Url, token: String) -> Self {
        let api_url = base_url.join("api/v4/").expect("valid URL");
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
            ..Default::default()
        }
    }
}

/// GitLab API client.
pub struct GitLabClient {
    config: GitLabConfig,
    http: Arc<HttpClient>,
    cache: Arc<RepositoryCache>,
}

impl std::fmt::Debug for GitLabClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitLabClient")
            .field("api_url", &self.config.api_url)
            .finish()
    }
}

impl GitLabClient {
    /// Create a new GitLab client.
    ///
    /// # Errors
    /// Returns error if HTTP client cannot be created.
    pub fn new() -> Result<Self> {
        Self::with_config(GitLabConfig::default())
    }

    /// Create a new GitLab client with custom configuration.
    ///
    /// # Errors
    /// Returns error if HTTP client cannot be created.
    pub fn with_config(config: GitLabConfig) -> Result<Self> {
        let http = HttpClient::with_config(config.http_config.clone()).map_err(|e| {
            RepositoryError::InvalidConfig {
                message: format!("HTTP client creation failed: {e}"),
            }
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

    /// URL-encode a project path for GitLab API.
    fn encode_project_path(owner: &str, repo: &str) -> String {
        format!("{owner}%2F{repo}")
    }

    /// Build API URL for a project endpoint.
    fn api_url(&self, owner: &str, repo: &str, endpoint: &str) -> Result<Url> {
        let encoded = Self::encode_project_path(owner, repo);
        let path = format!("projects/{encoded}/{endpoint}");
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
        let encoded_path = urlencoding::encode(path);
        let url = self.api_url(owner, repo, &format!("repository/files/{encoded_path}/raw"))?;
        let cache_key = format!("gitlab:{owner}/{repo}/{path}@{reference}");

        if let Some(data) = self.cache.get_metadata(&cache_key)
            && let Ok(content) = String::from_utf8(data.to_vec())
        {
            debug!(owner, repo, path, reference, "cache hit");
            return Ok(content);
        }

        let mut url = url;
        url.query_pairs_mut().append_pair("ref", reference);

        let response = self.http.get(&url).await?;

        let content =
            String::from_utf8(response.body.to_vec()).map_err(|e| RepositoryError::ParseError {
                source: url.to_string(),
                message: format!("UTF-8 decode failed: {e}"),
            })?;

        let _ = self
            .cache
            .put_metadata(&cache_key, content.as_bytes(), DEFAULT_METADATA_TTL, None);

        Ok(content)
    }

    /// List repository tags.
    pub async fn list_tags(&self, owner: &str, repo: &str) -> Result<Vec<GitLabTag>> {
        let url = self.api_url(owner, repo, "repository/tags")?;
        let cache_key = format!("gitlab:{owner}/{repo}/tags");

        if let Some(data) = self.cache.get_metadata(&cache_key)
            && let Ok(tags) = sonic_rs::from_slice::<Vec<GitLabTag>>(&data)
        {
            return Ok(tags);
        }

        let response = self.http.get(&url).await?;

        let tags: Vec<GitLabTag> =
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
    pub async fn list_branches(&self, owner: &str, repo: &str) -> Result<Vec<GitLabBranch>> {
        let url = self.api_url(owner, repo, "repository/branches")?;
        let cache_key = format!("gitlab:{owner}/{repo}/branches");

        if let Some(data) = self.cache.get_metadata(&cache_key)
            && let Ok(branches) = sonic_rs::from_slice::<Vec<GitLabBranch>>(&data)
        {
            return Ok(branches);
        }

        let response = self.http.get(&url).await?;

        let branches: Vec<GitLabBranch> =
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

    /// Get project information.
    pub async fn get_project(&self, owner: &str, repo: &str) -> Result<GitLabProject> {
        let encoded = Self::encode_project_path(owner, repo);
        let path = format!("projects/{encoded}");
        let url = self
            .config
            .api_url
            .join(&path)
            .map_err(|e| RepositoryError::InvalidUrl {
                url: self.config.api_url.to_string(),
                message: e.to_string(),
            })?;

        let cache_key = format!("gitlab:{owner}/{repo}/info");

        if let Some(data) = self.cache.get_metadata(&cache_key)
            && let Ok(project) = sonic_rs::from_slice::<GitLabProject>(&data)
        {
            return Ok(project);
        }

        let response = self.http.get(&url).await?;

        let project: GitLabProject =
            sonic_rs::from_slice(&response.body).map_err(|e| RepositoryError::ParseError {
                source: url.to_string(),
                message: e.to_string(),
            })?;

        if let Ok(data) = sonic_rs::to_vec(&project) {
            let _ = self
                .cache
                .put_metadata(&cache_key, &data, DEFAULT_METADATA_TTL, None);
        }

        Ok(project)
    }

    /// List releases.
    pub async fn list_releases(&self, owner: &str, repo: &str) -> Result<Vec<GitLabRelease>> {
        let url = self.api_url(owner, repo, "releases")?;
        let cache_key = format!("gitlab:{owner}/{repo}/releases");

        if let Some(data) = self.cache.get_metadata(&cache_key)
            && let Ok(releases) = sonic_rs::from_slice::<Vec<GitLabRelease>>(&data)
        {
            return Ok(releases);
        }

        let response = self.http.get(&url).await?;

        let releases: Vec<GitLabRelease> =
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
}

impl VcsProvider for GitLabClient {
    fn name(&self) -> &'static str {
        "GitLab"
    }

    fn can_handle(&self, url: &Url) -> bool {
        url.host_str()
            .is_some_and(|h| h.contains("gitlab.com") || h.contains("gitlab."))
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

            let project = self.get_project(&owner, &repo).await?;
            Ok(project.default_branch)
        })
    }
}

// GitLab API response types

/// GitLab tag.
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct GitLabTag {
    /// Tag name.
    pub name: String,
    /// Target commit SHA.
    pub target: String,
    /// Message (for annotated tags).
    #[serde(default)]
    pub message: Option<String>,
}

/// GitLab branch.
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct GitLabBranch {
    /// Branch name.
    pub name: String,
    /// Whether protected.
    #[serde(default)]
    pub protected: bool,
    /// Whether default.
    #[serde(default)]
    pub default: bool,
    /// Commit info.
    pub commit: GitLabCommit,
}

/// GitLab commit.
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct GitLabCommit {
    /// Commit SHA.
    pub id: String,
    /// Short SHA.
    #[serde(default)]
    pub short_id: Option<String>,
    /// Commit title.
    #[serde(default)]
    pub title: Option<String>,
    /// Author name.
    #[serde(default)]
    pub author_name: Option<String>,
}

/// GitLab project info.
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct GitLabProject {
    /// Project ID.
    pub id: u64,
    /// Project name.
    pub name: String,
    /// Full path with namespace.
    pub path_with_namespace: String,
    /// Description.
    #[serde(default)]
    pub description: Option<String>,
    /// Visibility (private, internal, public).
    pub visibility: String,
    /// Web URL.
    pub web_url: String,
    /// HTTP clone URL.
    pub http_url_to_repo: String,
    /// SSH clone URL.
    #[serde(default)]
    pub ssh_url_to_repo: Option<String>,
    /// Default branch.
    pub default_branch: String,
    /// Star count.
    #[serde(default)]
    pub star_count: u64,
    /// Fork count.
    #[serde(default)]
    pub forks_count: u64,
    /// Open issues count.
    #[serde(default)]
    pub open_issues_count: u64,
    /// Whether archived.
    #[serde(default)]
    pub archived: bool,
}

/// GitLab release.
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct GitLabRelease {
    /// Tag name.
    pub tag_name: String,
    /// Release name.
    #[serde(default)]
    pub name: Option<String>,
    /// Description.
    #[serde(default)]
    pub description: Option<String>,
    /// Created at.
    pub created_at: String,
    /// Released at.
    #[serde(default)]
    pub released_at: Option<String>,
    /// Assets.
    #[serde(default)]
    pub assets: Option<GitLabReleaseAssets>,
}

/// GitLab release assets.
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct GitLabReleaseAssets {
    /// Asset count.
    #[serde(default)]
    pub count: u32,
    /// Asset links.
    #[serde(default)]
    pub links: Vec<GitLabAssetLink>,
    /// Source archive URLs.
    #[serde(default)]
    pub sources: Vec<GitLabAssetSource>,
}

/// GitLab asset link.
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct GitLabAssetLink {
    /// Link name.
    pub name: String,
    /// URL.
    pub url: String,
}

/// GitLab asset source.
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct GitLabAssetSource {
    /// Format (zip, tar.gz, etc.).
    pub format: String,
    /// URL.
    pub url: String,
}

/// URL encoding helper.
mod urlencoding {
    pub fn encode(input: &str) -> String {
        let mut encoded = String::with_capacity(input.len() * 3);
        for byte in input.bytes() {
            match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    encoded.push(byte as char);
                }
                _ => {
                    encoded.push_str(&format!("%{byte:02X}"));
                }
            }
        }
        encoded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = GitLabConfig::default();
        assert_eq!(config.api_url.as_str(), GITLAB_API_URL);
        assert!(config.token.is_none());
    }

    #[test]
    fn test_encode_project_path() {
        assert_eq!(
            GitLabClient::encode_project_path("symfony", "console"),
            "symfony%2Fconsole"
        );
    }

    #[test]
    fn test_urlencoding() {
        assert_eq!(urlencoding::encode("hello/world"), "hello%2Fworld");
        assert_eq!(urlencoding::encode("composer.json"), "composer.json");
    }
}
