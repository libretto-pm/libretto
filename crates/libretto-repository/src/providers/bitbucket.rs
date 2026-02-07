//! Bitbucket API client for fetching repository metadata.

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

/// Bitbucket API base URL.
pub const BITBUCKET_API_URL: &str = "https://api.bitbucket.org/2.0/";

/// Bitbucket configuration.
#[derive(Debug, Clone)]
pub struct BitbucketConfig {
    /// API base URL (for Bitbucket Server).
    pub api_url: Url,
    /// Username for authentication.
    pub username: Option<String>,
    /// App password or token.
    pub token: Option<String>,
    /// HTTP client configuration.
    pub http_config: HttpClientConfig,
}

impl Default for BitbucketConfig {
    fn default() -> Self {
        Self {
            api_url: Url::parse(BITBUCKET_API_URL).expect("valid URL"),
            username: None,
            token: None,
            http_config: HttpClientConfig::default(),
        }
    }
}

impl BitbucketConfig {
    /// Create configuration with app password.
    #[must_use]
    pub fn with_app_password(username: String, app_password: String) -> Self {
        Self {
            username: Some(username),
            token: Some(app_password),
            ..Default::default()
        }
    }

    /// Create configuration for Bitbucket Server.
    #[must_use]
    pub fn server(api_url: Url, username: String, token: String) -> Self {
        Self {
            api_url,
            username: Some(username),
            token: Some(token),
            ..Default::default()
        }
    }
}

/// Bitbucket API client.
pub struct BitbucketClient {
    config: BitbucketConfig,
    http: Arc<HttpClient>,
    cache: Arc<RepositoryCache>,
}

impl std::fmt::Debug for BitbucketClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BitbucketClient")
            .field("api_url", &self.config.api_url)
            .finish()
    }
}

impl BitbucketClient {
    /// Create a new Bitbucket client.
    ///
    /// # Errors
    /// Returns error if HTTP client cannot be created.
    pub fn new() -> Result<Self> {
        Self::with_config(BitbucketConfig::default())
    }

    /// Create a new Bitbucket client with custom configuration.
    ///
    /// # Errors
    /// Returns error if HTTP client cannot be created.
    pub fn with_config(config: BitbucketConfig) -> Result<Self> {
        let http = HttpClient::with_config(config.http_config.clone()).map_err(|e| {
            RepositoryError::InvalidConfig {
                message: format!("HTTP client creation failed: {e}"),
            }
        })?;

        // Bitbucket uses Basic auth with username and app password
        if let (Some(username), Some(token)) = (&config.username, &config.token)
            && let Some(host) = config.api_url.host_str()
        {
            http.set_auth(
                host,
                AuthType::Basic {
                    username: username.clone(),
                    password: token.clone(),
                },
            );
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
    fn api_url(&self, workspace: &str, repo: &str, endpoint: &str) -> Result<Url> {
        let path = format!("repositories/{workspace}/{repo}/{endpoint}");
        self.config
            .api_url
            .join(&path)
            .map_err(|e| RepositoryError::InvalidUrl {
                url: self.config.api_url.to_string(),
                message: e.to_string(),
            })
    }

    /// Fetch all pages of a paginated Bitbucket API endpoint.
    async fn fetch_all_pages<T>(&self, url: &Url) -> Result<Vec<T>>
    where
        T: serde::de::DeserializeOwned,
    {
        let mut all_items = Vec::new();
        let mut next_url = Some(url.clone());

        while let Some(current_url) = next_url {
            let response = self.http.get(&current_url).await?;

            let paginated: BitbucketPaginatedResponse<T> = sonic_rs::from_slice(&response.body)
                .map_err(|e| RepositoryError::ParseError {
                    source: current_url.to_string(),
                    message: e.to_string(),
                })?;

            all_items.extend(paginated.values);

            // Follow the next page if available
            next_url = paginated.next.and_then(|next| Url::parse(&next).ok());
        }

        Ok(all_items)
    }

    /// Get file contents from a repository.
    pub async fn get_file_contents(
        &self,
        workspace: &str,
        repo: &str,
        path: &str,
        reference: &str,
    ) -> Result<String> {
        let url = self.api_url(workspace, repo, &format!("src/{reference}/{path}"))?;
        let cache_key = format!("bitbucket:{workspace}/{repo}/{path}@{reference}");

        if let Some(data) = self.cache.get_metadata(&cache_key)
            && let Ok(content) = String::from_utf8(data.to_vec())
        {
            debug!(workspace, repo, path, reference, "cache hit");
            return Ok(content);
        }

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

    /// List repository tags with pagination support.
    pub async fn list_tags(&self, workspace: &str, repo: &str) -> Result<Vec<BitbucketRef>> {
        let url = self.api_url(workspace, repo, "refs/tags")?;
        let cache_key = format!("bitbucket:{workspace}/{repo}/tags");

        if let Some(data) = self.cache.get_metadata(&cache_key)
            && let Ok(tags) = sonic_rs::from_slice::<Vec<BitbucketRef>>(&data)
        {
            return Ok(tags);
        }

        let tags = self.fetch_all_pages::<BitbucketRef>(&url).await?;

        if let Ok(data) = sonic_rs::to_vec(&tags) {
            let _ = self
                .cache
                .put_metadata(&cache_key, &data, DEFAULT_METADATA_TTL, None);
        }

        Ok(tags)
    }

    /// List repository branches with pagination support.
    pub async fn list_branches(&self, workspace: &str, repo: &str) -> Result<Vec<BitbucketRef>> {
        let url = self.api_url(workspace, repo, "refs/branches")?;
        let cache_key = format!("bitbucket:{workspace}/{repo}/branches");

        if let Some(data) = self.cache.get_metadata(&cache_key)
            && let Ok(branches) = sonic_rs::from_slice::<Vec<BitbucketRef>>(&data)
        {
            return Ok(branches);
        }

        let branches = self.fetch_all_pages::<BitbucketRef>(&url).await?;

        if let Ok(data) = sonic_rs::to_vec(&branches) {
            let _ = self
                .cache
                .put_metadata(&cache_key, &data, DEFAULT_METADATA_TTL, None);
        }

        Ok(branches)
    }

    /// Get repository information.
    pub async fn get_repository(&self, workspace: &str, repo: &str) -> Result<BitbucketRepository> {
        let path = format!("repositories/{workspace}/{repo}");
        let url = self
            .config
            .api_url
            .join(&path)
            .map_err(|e| RepositoryError::InvalidUrl {
                url: self.config.api_url.to_string(),
                message: e.to_string(),
            })?;

        let cache_key = format!("bitbucket:{workspace}/{repo}/info");

        if let Some(data) = self.cache.get_metadata(&cache_key)
            && let Ok(repo_info) = sonic_rs::from_slice::<BitbucketRepository>(&data)
        {
            return Ok(repo_info);
        }

        let response = self.http.get(&url).await?;

        let repo_info: BitbucketRepository =
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
}

impl VcsProvider for BitbucketClient {
    fn name(&self) -> &'static str {
        "Bitbucket"
    }

    fn can_handle(&self, url: &Url) -> bool {
        url.host_str()
            .is_some_and(|h| h.contains("bitbucket.org") || h.contains("bitbucket."))
    }

    fn fetch_composer_json<'a>(
        &'a self,
        url: &'a Url,
        reference: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>> {
        Box::pin(async move {
            let (workspace, repo) =
                parse_vcs_url(url).ok_or_else(|| RepositoryError::InvalidUrl {
                    url: url.to_string(),
                    message: "Could not parse workspace/repo from URL".into(),
                })?;

            self.get_file_contents(&workspace, &repo, "composer.json", reference)
                .await
        })
    }

    fn list_tags<'a>(
        &'a self,
        url: &'a Url,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<String>>> + Send + 'a>> {
        Box::pin(async move {
            let (workspace, repo) =
                parse_vcs_url(url).ok_or_else(|| RepositoryError::InvalidUrl {
                    url: url.to_string(),
                    message: "Could not parse workspace/repo from URL".into(),
                })?;

            let tags = self.list_tags(&workspace, &repo).await?;
            Ok(tags.into_iter().map(|t| t.name).collect())
        })
    }

    fn list_branches<'a>(
        &'a self,
        url: &'a Url,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<String>>> + Send + 'a>> {
        Box::pin(async move {
            let (workspace, repo) =
                parse_vcs_url(url).ok_or_else(|| RepositoryError::InvalidUrl {
                    url: url.to_string(),
                    message: "Could not parse workspace/repo from URL".into(),
                })?;

            let branches = self.list_branches(&workspace, &repo).await?;
            Ok(branches.into_iter().map(|b| b.name).collect())
        })
    }

    fn get_default_branch<'a>(
        &'a self,
        url: &'a Url,
    ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>> {
        Box::pin(async move {
            let (workspace, repo) =
                parse_vcs_url(url).ok_or_else(|| RepositoryError::InvalidUrl {
                    url: url.to_string(),
                    message: "Could not parse workspace/repo from URL".into(),
                })?;

            let repo_info = self.get_repository(&workspace, &repo).await?;
            repo_info
                .mainbranch
                .map(|b| b.name)
                .ok_or_else(|| RepositoryError::VcsError {
                    url: url.to_string(),
                    message: "No default branch found".into(),
                })
        })
    }
}

// Bitbucket API response types

/// Bitbucket paginated response.
#[derive(Debug, Deserialize)]
struct BitbucketPaginatedResponse<T> {
    values: Vec<T>,
    #[serde(default)]
    next: Option<String>,
}

/// Bitbucket ref (tag or branch).
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct BitbucketRef {
    /// Ref name.
    pub name: String,
    /// Target commit.
    pub target: BitbucketTarget,
}

/// Bitbucket target (commit).
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct BitbucketTarget {
    /// Commit hash.
    pub hash: String,
    /// Commit date.
    #[serde(default)]
    pub date: Option<String>,
    /// Commit message.
    #[serde(default)]
    pub message: Option<String>,
}

/// Bitbucket repository info.
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct BitbucketRepository {
    /// Repository UUID.
    pub uuid: String,
    /// Repository slug.
    pub slug: String,
    /// Full name.
    pub full_name: String,
    /// Repository name.
    pub name: String,
    /// Description.
    #[serde(default)]
    pub description: Option<String>,
    /// Whether private.
    #[serde(default)]
    pub is_private: bool,
    /// SCM type (git, hg).
    pub scm: String,
    /// Main branch.
    #[serde(default)]
    pub mainbranch: Option<BitbucketMainBranch>,
    /// Clone links.
    #[serde(default)]
    pub links: BitbucketLinks,
    /// Created date.
    #[serde(default)]
    pub created_on: Option<String>,
    /// Updated date.
    #[serde(default)]
    pub updated_on: Option<String>,
    /// Size in bytes.
    #[serde(default)]
    pub size: Option<u64>,
    /// Language.
    #[serde(default)]
    pub language: Option<String>,
    /// Fork policy.
    #[serde(default)]
    pub fork_policy: Option<String>,
}

/// Bitbucket main branch.
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct BitbucketMainBranch {
    /// Branch name.
    pub name: String,
    /// Branch type.
    #[serde(rename = "type")]
    pub branch_type: String,
}

/// Bitbucket links.
#[derive(Debug, Clone, Default, Deserialize, serde::Serialize)]
pub struct BitbucketLinks {
    /// Clone links.
    #[serde(default)]
    pub clone: Vec<BitbucketCloneLink>,
    /// HTML link.
    #[serde(default)]
    pub html: Option<BitbucketLink>,
}

/// Bitbucket clone link.
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct BitbucketCloneLink {
    /// Clone URL.
    pub href: String,
    /// Clone type (https, ssh).
    pub name: String,
}

/// Bitbucket link.
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct BitbucketLink {
    /// Link URL.
    pub href: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = BitbucketConfig::default();
        assert_eq!(config.api_url.as_str(), BITBUCKET_API_URL);
        assert!(config.token.is_none());
    }

    #[test]
    fn test_config_with_app_password() {
        let config = BitbucketConfig::with_app_password("user".into(), "pass".into());
        assert_eq!(config.username, Some("user".into()));
        assert_eq!(config.token, Some("pass".into()));
    }
}
