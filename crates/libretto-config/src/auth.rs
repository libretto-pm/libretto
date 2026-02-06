//! Authentication configuration and credential management.
//!
//! Provides full Composer-compatible authentication support including:
//! - HTTP Basic authentication
//! - Bearer tokens
//! - GitHub OAuth tokens
//! - GitLab OAuth and private tokens
//! - Bitbucket OAuth
//! - Custom HTTP headers
//! - Client TLS certificates
//! - Forgejo tokens

use crate::error::{ConfigError, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// Authentication configuration from auth.json.
/// Fully compatible with Composer's auth.json format.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", default)]
pub struct AuthConfig {
    /// HTTP Basic auth credentials by domain.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub http_basic: BTreeMap<String, HttpBasicCredentials>,

    /// Bearer tokens by domain.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub bearer: BTreeMap<String, BearerToken>,

    /// Custom HTTP headers by domain.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub custom_headers: BTreeMap<String, Vec<String>>,

    /// GitHub OAuth tokens by domain.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub github_oauth: BTreeMap<String, String>,

    /// GitLab OAuth tokens by domain.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub gitlab_oauth: BTreeMap<String, GitLabOAuthToken>,

    /// GitLab private tokens by domain.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub gitlab_token: BTreeMap<String, GitLabToken>,

    /// Bitbucket OAuth credentials by domain.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub bitbucket_oauth: BTreeMap<String, BitbucketOAuthCredentials>,

    /// Client TLS certificates by domain.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub client_certificate: BTreeMap<String, ClientCertificate>,

    /// Forgejo tokens by domain.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub forgejo_token: BTreeMap<String, ForgejoToken>,
}

/// HTTP Basic credentials.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpBasicCredentials {
    /// Username.
    pub username: String,
    /// Password.
    pub password: String,
}

/// Bearer token (simple string or extended format).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BearerToken {
    /// Simple token string.
    Simple(String),
    /// Token with additional options.
    Extended {
        /// Token value.
        token: String,
    },
}

impl BearerToken {
    /// Get the token value.
    #[must_use]
    pub fn token(&self) -> &str {
        match self {
            Self::Simple(s) => s,
            Self::Extended { token } => token,
        }
    }
}

/// GitLab OAuth token.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum GitLabOAuthToken {
    /// Simple token string.
    Simple(String),
    /// Token object.
    Extended {
        /// OAuth token.
        token: String,
    },
}

impl GitLabOAuthToken {
    /// Get the token value.
    #[must_use]
    pub fn token(&self) -> &str {
        match self {
            Self::Simple(s) => s,
            Self::Extended { token } => token,
        }
    }
}

/// GitLab private token.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum GitLabToken {
    /// Simple token string.
    Simple(String),
    /// Token with additional options.
    Extended {
        /// Token value.
        token: String,
        /// Token type (e.g., "private-token", "job-token").
        #[serde(rename = "token-type", skip_serializing_if = "Option::is_none")]
        token_type: Option<String>,
    },
}

impl GitLabToken {
    /// Get the token value.
    #[must_use]
    pub fn token(&self) -> &str {
        match self {
            Self::Simple(s) => s,
            Self::Extended { token, .. } => token,
        }
    }
}

/// Bitbucket OAuth credentials.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BitbucketOAuthCredentials {
    /// Consumer key.
    #[serde(rename = "consumer-key")]
    pub consumer_key: String,
    /// Consumer secret.
    #[serde(rename = "consumer-secret")]
    pub consumer_secret: String,
}

/// Client TLS certificate configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientCertificate {
    /// Path to the certificate file (required).
    pub local_cert: String,
    /// Path to the private key file (optional if combined with cert).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_pk: Option<String>,
    /// Passphrase for the private key (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub passphrase: Option<String>,
}

/// Forgejo/Gitea token credentials.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForgejoToken {
    /// Username.
    pub username: String,
    /// Access token.
    pub token: String,
}

impl AuthConfig {
    /// Load auth config from file.
    ///
    /// # Errors
    /// Returns error if file cannot be read or parsed.
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path).map_err(|e| ConfigError::io(path, e))?;
        sonic_rs::from_str(&content).map_err(|e| ConfigError::json(path, &e))
    }

    /// Load auth config from file, returning default if not found.
    #[must_use]
    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }

    /// Save auth config to file.
    ///
    /// # Errors
    /// Returns error if file cannot be written.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| ConfigError::io(parent, e))?;
        }
        let content = sonic_rs::to_string_pretty(self)?;
        std::fs::write(path, content).map_err(|e| ConfigError::io(path, e))
    }

    /// Merge another auth config into this one (other takes precedence).
    pub fn merge(&mut self, other: &Self) {
        for (k, v) in &other.http_basic {
            self.http_basic.insert(k.clone(), v.clone());
        }
        for (k, v) in &other.bearer {
            self.bearer.insert(k.clone(), v.clone());
        }
        for (k, v) in &other.custom_headers {
            self.custom_headers.insert(k.clone(), v.clone());
        }
        for (k, v) in &other.github_oauth {
            self.github_oauth.insert(k.clone(), v.clone());
        }
        for (k, v) in &other.gitlab_oauth {
            self.gitlab_oauth.insert(k.clone(), v.clone());
        }
        for (k, v) in &other.gitlab_token {
            self.gitlab_token.insert(k.clone(), v.clone());
        }
        for (k, v) in &other.bitbucket_oauth {
            self.bitbucket_oauth.insert(k.clone(), v.clone());
        }
        for (k, v) in &other.client_certificate {
            self.client_certificate.insert(k.clone(), v.clone());
        }
        for (k, v) in &other.forgejo_token {
            self.forgejo_token.insert(k.clone(), v.clone());
        }
    }

    /// Check if config is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.http_basic.is_empty()
            && self.bearer.is_empty()
            && self.custom_headers.is_empty()
            && self.github_oauth.is_empty()
            && self.gitlab_oauth.is_empty()
            && self.gitlab_token.is_empty()
            && self.bitbucket_oauth.is_empty()
            && self.client_certificate.is_empty()
            && self.forgejo_token.is_empty()
    }

    // ========== Getters ==========

    /// Get HTTP Basic credentials for a domain.
    #[must_use]
    pub fn get_http_basic(&self, domain: &str) -> Option<&HttpBasicCredentials> {
        self.http_basic.get(domain).or_else(|| {
            self.http_basic
                .get(domain.strip_prefix("www.").unwrap_or(domain))
        })
    }

    /// Get bearer token for a domain.
    #[must_use]
    pub fn get_bearer(&self, domain: &str) -> Option<&str> {
        self.bearer
            .get(domain)
            .or_else(|| {
                self.bearer
                    .get(domain.strip_prefix("www.").unwrap_or(domain))
            })
            .map(BearerToken::token)
    }

    /// Get custom headers for a domain.
    #[must_use]
    pub fn get_custom_headers(&self, domain: &str) -> Option<&[String]> {
        self.custom_headers
            .get(domain)
            .or_else(|| {
                self.custom_headers
                    .get(domain.strip_prefix("www.").unwrap_or(domain))
            })
            .map(Vec::as_slice)
    }

    /// Get GitHub OAuth token for a domain.
    #[must_use]
    pub fn get_github_oauth(&self, domain: &str) -> Option<&str> {
        self.github_oauth
            .get(domain)
            .or_else(|| {
                self.github_oauth
                    .get(domain.strip_prefix("www.").unwrap_or(domain))
            })
            .map(String::as_str)
    }

    /// Get GitLab OAuth token for a domain.
    #[must_use]
    pub fn get_gitlab_oauth(&self, domain: &str) -> Option<&str> {
        self.gitlab_oauth
            .get(domain)
            .or_else(|| {
                self.gitlab_oauth
                    .get(domain.strip_prefix("www.").unwrap_or(domain))
            })
            .map(GitLabOAuthToken::token)
    }

    /// Get GitLab private token for a domain.
    #[must_use]
    pub fn get_gitlab_token(&self, domain: &str) -> Option<&str> {
        self.gitlab_token
            .get(domain)
            .or_else(|| {
                self.gitlab_token
                    .get(domain.strip_prefix("www.").unwrap_or(domain))
            })
            .map(GitLabToken::token)
    }

    /// Get Bitbucket OAuth credentials for a domain.
    #[must_use]
    pub fn get_bitbucket_oauth(&self, domain: &str) -> Option<&BitbucketOAuthCredentials> {
        self.bitbucket_oauth.get(domain).or_else(|| {
            self.bitbucket_oauth
                .get(domain.strip_prefix("www.").unwrap_or(domain))
        })
    }

    /// Get client certificate for a domain.
    #[must_use]
    pub fn get_client_certificate(&self, domain: &str) -> Option<&ClientCertificate> {
        self.client_certificate.get(domain).or_else(|| {
            self.client_certificate
                .get(domain.strip_prefix("www.").unwrap_or(domain))
        })
    }

    /// Get Forgejo token for a domain.
    #[must_use]
    pub fn get_forgejo_token(&self, domain: &str) -> Option<&ForgejoToken> {
        self.forgejo_token.get(domain).or_else(|| {
            self.forgejo_token
                .get(domain.strip_prefix("www.").unwrap_or(domain))
        })
    }

    // ========== Setters ==========

    /// Set HTTP Basic credentials for a domain.
    pub fn set_http_basic(
        &mut self,
        domain: impl Into<String>,
        username: impl Into<String>,
        password: impl Into<String>,
    ) {
        self.http_basic.insert(
            domain.into(),
            HttpBasicCredentials {
                username: username.into(),
                password: password.into(),
            },
        );
    }

    /// Set bearer token for a domain.
    pub fn set_bearer(&mut self, domain: impl Into<String>, token: impl Into<String>) {
        self.bearer
            .insert(domain.into(), BearerToken::Simple(token.into()));
    }

    /// Set custom headers for a domain.
    pub fn set_custom_headers(&mut self, domain: impl Into<String>, headers: Vec<String>) {
        self.custom_headers.insert(domain.into(), headers);
    }

    /// Set GitHub OAuth token for a domain.
    pub fn set_github_oauth(&mut self, domain: impl Into<String>, token: impl Into<String>) {
        self.github_oauth.insert(domain.into(), token.into());
    }

    /// Set GitLab OAuth token for a domain.
    pub fn set_gitlab_oauth(&mut self, domain: impl Into<String>, token: impl Into<String>) {
        self.gitlab_oauth
            .insert(domain.into(), GitLabOAuthToken::Simple(token.into()));
    }

    /// Set GitLab private token for a domain.
    pub fn set_gitlab_token(&mut self, domain: impl Into<String>, token: impl Into<String>) {
        self.gitlab_token
            .insert(domain.into(), GitLabToken::Simple(token.into()));
    }

    /// Set Bitbucket OAuth credentials for a domain.
    pub fn set_bitbucket_oauth(
        &mut self,
        domain: impl Into<String>,
        consumer_key: impl Into<String>,
        consumer_secret: impl Into<String>,
    ) {
        self.bitbucket_oauth.insert(
            domain.into(),
            BitbucketOAuthCredentials {
                consumer_key: consumer_key.into(),
                consumer_secret: consumer_secret.into(),
            },
        );
    }

    /// Set client certificate for a domain.
    pub fn set_client_certificate(
        &mut self,
        domain: impl Into<String>,
        local_cert: impl Into<String>,
        local_pk: Option<String>,
        passphrase: Option<String>,
    ) {
        self.client_certificate.insert(
            domain.into(),
            ClientCertificate {
                local_cert: local_cert.into(),
                local_pk,
                passphrase,
            },
        );
    }

    /// Set Forgejo token for a domain.
    pub fn set_forgejo_token(
        &mut self,
        domain: impl Into<String>,
        username: impl Into<String>,
        token: impl Into<String>,
    ) {
        self.forgejo_token.insert(
            domain.into(),
            ForgejoToken {
                username: username.into(),
                token: token.into(),
            },
        );
    }

    /// Remove all credentials for a domain.
    pub fn remove_domain(&mut self, domain: &str) {
        self.http_basic.remove(domain);
        self.bearer.remove(domain);
        self.custom_headers.remove(domain);
        self.github_oauth.remove(domain);
        self.gitlab_oauth.remove(domain);
        self.gitlab_token.remove(domain);
        self.bitbucket_oauth.remove(domain);
        self.client_certificate.remove(domain);
        self.forgejo_token.remove(domain);
    }

    /// Get the best credential for a domain, checking all auth types.
    /// Priority order matches Composer's behavior.
    #[must_use]
    pub fn get_credential_for_domain(&self, domain: &str) -> Option<Credential> {
        // Check GitHub OAuth first for github.com domains
        if (domain.contains("github.com") || domain.contains("api.github.com"))
            && let Some(token) = self.get_github_oauth("github.com")
        {
            return Some(Credential::GitHubOAuth(token.to_string()));
        }

        // Check GitLab domains
        if domain.contains("gitlab") {
            if let Some(token) = self.get_gitlab_oauth(domain) {
                return Some(Credential::GitLabOAuth(token.to_string()));
            }
            if let Some(token) = self.get_gitlab_token(domain) {
                return Some(Credential::GitLabToken(token.to_string()));
            }
        }

        // Check Bitbucket
        if domain.contains("bitbucket")
            && let Some(cred) = self.get_bitbucket_oauth(domain)
        {
            return Some(Credential::BitbucketOAuth {
                consumer_key: cred.consumer_key.clone(),
                consumer_secret: cred.consumer_secret.clone(),
            });
        }

        // Check Forgejo/Gitea
        if let Some(cred) = self.get_forgejo_token(domain) {
            return Some(Credential::ForgejoToken {
                username: cred.username.clone(),
                token: cred.token.clone(),
            });
        }

        // Check bearer token
        if let Some(token) = self.get_bearer(domain) {
            return Some(Credential::Bearer(token.to_string()));
        }

        // Check HTTP Basic
        if let Some(cred) = self.get_http_basic(domain) {
            return Some(Credential::HttpBasic {
                username: cred.username.clone(),
                password: cred.password.clone(),
            });
        }

        // Check custom headers
        if let Some(headers) = self.get_custom_headers(domain) {
            return Some(Credential::CustomHeaders(headers.to_vec()));
        }

        None
    }
}

/// Credential type for authentication.
#[derive(Debug, Clone)]
pub enum Credential {
    /// HTTP Basic authentication.
    HttpBasic {
        /// Username.
        username: String,
        /// Password.
        password: String,
    },
    /// Bearer token.
    Bearer(String),
    /// Custom HTTP headers.
    CustomHeaders(Vec<String>),
    /// GitHub OAuth token.
    GitHubOAuth(String),
    /// GitLab OAuth token.
    GitLabOAuth(String),
    /// GitLab private token.
    GitLabToken(String),
    /// Bitbucket OAuth.
    BitbucketOAuth {
        /// Consumer key.
        consumer_key: String,
        /// Consumer secret.
        consumer_secret: String,
    },
    /// Forgejo/Gitea token.
    ForgejoToken {
        /// Username.
        username: String,
        /// Access token.
        token: String,
    },
}

impl Credential {
    /// Get credential as HTTP Authorization header value.
    /// Returns None for `CustomHeaders` (which need special handling).
    #[must_use]
    pub fn as_authorization_header(&self) -> Option<String> {
        match self {
            Self::HttpBasic { username, password } => {
                let encoded = base64_encode(format!("{username}:{password}").as_bytes());
                Some(format!("Basic {encoded}"))
            }
            Self::Bearer(token)
            | Self::GitHubOAuth(token)
            | Self::GitLabOAuth(token)
            | Self::GitLabToken(token) => Some(format!("Bearer {token}")),
            Self::BitbucketOAuth {
                consumer_key,
                consumer_secret,
            } => {
                // Bitbucket uses OAuth, but for simple API access we use Basic auth with the credentials
                let encoded = base64_encode(format!("{consumer_key}:{consumer_secret}").as_bytes());
                Some(format!("Basic {encoded}"))
            }
            Self::ForgejoToken { token, .. } => Some(format!("token {token}")),
            Self::CustomHeaders(_) => None, // Custom headers need special handling
        }
    }

    /// Get custom headers if this is a `CustomHeaders` credential.
    #[must_use]
    pub fn custom_headers(&self) -> Option<&[String]> {
        match self {
            Self::CustomHeaders(headers) => Some(headers),
            _ => None,
        }
    }
}

/// Credential store that manages loading auth from multiple sources.
///
/// Loads credentials from:
/// 1. Global auth.json (`~/.config/libretto/auth.json` or `COMPOSER_HOME/auth.json`)
/// 2. Project auth.json (in project root)
/// 3. Environment variable (`COMPOSER_AUTH`)
///
/// Project credentials take precedence over global credentials.
#[derive(Debug)]
pub struct CredentialStore {
    /// Merged auth configuration.
    config: AuthConfig,
}

impl CredentialStore {
    /// Create a new credential store loading from default locations.
    #[must_use]
    pub fn new() -> Self {
        Self::with_project_root(None)
    }

    /// Create a credential store with a specific project root.
    #[must_use]
    pub fn with_project_root(project_root: Option<&Path>) -> Self {
        let mut config = AuthConfig::default();

        // Load global auth.json
        if let Some(global_path) = Self::global_auth_path()
            && let Ok(global_config) = AuthConfig::load(&global_path)
        {
            config.merge(&global_config);
        }

        // Load project auth.json
        if let Some(root) = project_root {
            let project_auth = root.join("auth.json");
            if let Ok(project_config) = AuthConfig::load(&project_auth) {
                config.merge(&project_config);
            }
        }

        // Load from COMPOSER_AUTH environment variable
        if let Ok(auth_json) = std::env::var("COMPOSER_AUTH")
            && let Ok(env_config) = sonic_rs::from_str::<AuthConfig>(&auth_json)
        {
            config.merge(&env_config);
        }

        Self { config }
    }

    /// Get the global auth.json path.
    #[must_use]
    pub fn global_auth_path() -> Option<std::path::PathBuf> {
        // Check COMPOSER_HOME first
        if let Ok(home) = std::env::var("COMPOSER_HOME") {
            return Some(std::path::PathBuf::from(home).join("auth.json"));
        }

        // Fall back to XDG config or platform default
        #[cfg(target_os = "linux")]
        {
            if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
                return Some(std::path::PathBuf::from(xdg).join("libretto/auth.json"));
            }
            if let Ok(home) = std::env::var("HOME") {
                return Some(std::path::PathBuf::from(home).join(".config/libretto/auth.json"));
            }
        }

        #[cfg(target_os = "macos")]
        {
            if let Ok(home) = std::env::var("HOME") {
                return Some(
                    std::path::PathBuf::from(home)
                        .join("Library/Application Support/libretto/auth.json"),
                );
            }
        }

        #[cfg(target_os = "windows")]
        {
            if let Ok(appdata) = std::env::var("APPDATA") {
                return Some(std::path::PathBuf::from(appdata).join("libretto/auth.json"));
            }
        }

        None
    }

    /// Get the underlying auth config.
    #[must_use]
    pub const fn config(&self) -> &AuthConfig {
        &self.config
    }

    /// Get mutable reference to the underlying auth config.
    pub const fn config_mut(&mut self) -> &mut AuthConfig {
        &mut self.config
    }

    /// Get credential for a domain.
    #[must_use]
    pub fn get_credential(&self, domain: &str) -> Option<Credential> {
        self.config.get_credential_for_domain(domain)
    }

    /// Save the current config to the global auth.json.
    ///
    /// # Errors
    /// Returns error if file cannot be written.
    pub fn save_global(&self) -> Result<()> {
        if let Some(path) = Self::global_auth_path() {
            self.config.save(&path)
        } else {
            Err(ConfigError::Other(
                "Could not determine global auth.json path".to_string(),
            ))
        }
    }

    /// Save the current config to a project auth.json.
    ///
    /// # Errors
    /// Returns error if file cannot be written.
    pub fn save_project(&self, project_root: &Path) -> Result<()> {
        let path = project_root.join("auth.json");
        self.config.save(&path)
    }
}

impl Default for CredentialStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Simple base64 encoding.
fn base64_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut result = String::with_capacity(data.len().div_ceil(3) * 4);

    for chunk in data.chunks(3) {
        let b0 = chunk[0] as usize;
        let b1 = chunk.get(1).copied().unwrap_or(0) as usize;
        let b2 = chunk.get(2).copied().unwrap_or(0) as usize;

        result.push(ALPHABET[b0 >> 2] as char);
        result.push(ALPHABET[((b0 & 0x03) << 4) | (b1 >> 4)] as char);

        if chunk.len() > 1 {
            result.push(ALPHABET[((b1 & 0x0f) << 2) | (b2 >> 6)] as char);
        } else {
            result.push('=');
        }

        if chunk.len() > 2 {
            result.push(ALPHABET[b2 & 0x3f] as char);
        } else {
            result.push('=');
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_config_empty() {
        let config = AuthConfig::default();
        assert!(config.is_empty());
    }

    #[test]
    fn auth_config_set_get_http_basic() {
        let mut config = AuthConfig::default();
        config.set_http_basic("example.com", "user", "pass");

        let cred = config.get_http_basic("example.com").unwrap();
        assert_eq!(cred.username, "user");
        assert_eq!(cred.password, "pass");
    }

    #[test]
    fn auth_config_set_get_github_oauth() {
        let mut config = AuthConfig::default();
        config.set_github_oauth("github.com", "ghp_testtoken123");

        assert_eq!(
            config.get_github_oauth("github.com"),
            Some("ghp_testtoken123")
        );
    }

    #[test]
    fn auth_config_set_get_custom_headers() {
        let mut config = AuthConfig::default();
        config.set_custom_headers(
            "repo.example.org",
            vec![
                "API-TOKEN: secret123".to_string(),
                "X-Custom: value".to_string(),
            ],
        );

        let headers = config.get_custom_headers("repo.example.org").unwrap();
        assert_eq!(headers.len(), 2);
        assert_eq!(headers[0], "API-TOKEN: secret123");
    }

    #[test]
    fn auth_config_set_get_forgejo_token() {
        let mut config = AuthConfig::default();
        config.set_forgejo_token("codeberg.org", "myuser", "mytoken");

        let cred = config.get_forgejo_token("codeberg.org").unwrap();
        assert_eq!(cred.username, "myuser");
        assert_eq!(cred.token, "mytoken");
    }

    #[test]
    fn auth_config_set_get_client_certificate() {
        let mut config = AuthConfig::default();
        config.set_client_certificate(
            "repo.example.org",
            "/path/to/cert.pem",
            Some("/path/to/key.pem".to_string()),
            Some("secret".to_string()),
        );

        let cert = config.get_client_certificate("repo.example.org").unwrap();
        assert_eq!(cert.local_cert, "/path/to/cert.pem");
        assert_eq!(cert.local_pk, Some("/path/to/key.pem".to_string()));
        assert_eq!(cert.passphrase, Some("secret".to_string()));
    }

    #[test]
    fn auth_config_merge() {
        let mut config1 = AuthConfig::default();
        config1.set_github_oauth("github.com", "token1");

        let mut config2 = AuthConfig::default();
        config2.set_github_oauth("github.com", "token2");
        config2.set_gitlab_token("gitlab.com", "token3");

        config1.merge(&config2);

        assert_eq!(config1.get_github_oauth("github.com"), Some("token2"));
        assert_eq!(config1.get_gitlab_token("gitlab.com"), Some("token3"));
    }

    #[test]
    fn credential_authorization_header_basic() {
        let basic = Credential::HttpBasic {
            username: "user".to_string(),
            password: "pass".to_string(),
        };
        let header = basic.as_authorization_header().unwrap();
        assert!(header.starts_with("Basic "));
    }

    #[test]
    fn credential_authorization_header_bearer() {
        let bearer = Credential::Bearer("mytoken".to_string());
        assert_eq!(
            bearer.as_authorization_header(),
            Some("Bearer mytoken".to_string())
        );
    }

    #[test]
    fn credential_authorization_header_github() {
        let github = Credential::GitHubOAuth("ghp_token123".to_string());
        assert_eq!(
            github.as_authorization_header(),
            Some("Bearer ghp_token123".to_string())
        );
    }

    #[test]
    fn credential_authorization_header_forgejo() {
        let forgejo = Credential::ForgejoToken {
            username: "user".to_string(),
            token: "mytoken".to_string(),
        };
        assert_eq!(
            forgejo.as_authorization_header(),
            Some("token mytoken".to_string())
        );
    }

    #[test]
    fn credential_custom_headers() {
        let custom = Credential::CustomHeaders(vec!["X-API-Key: secret".to_string()]);
        assert!(custom.as_authorization_header().is_none());
        assert_eq!(custom.custom_headers().unwrap().len(), 1);
    }

    #[test]
    fn base64_encode_test() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"user:pass"), "dXNlcjpwYXNz");
    }

    #[test]
    fn get_credential_for_domain() {
        let mut config = AuthConfig::default();
        config.set_github_oauth("github.com", "ghp_token");
        config.set_http_basic("example.com", "user", "pass");

        // GitHub domain should return GitHub OAuth
        let cred = config.get_credential_for_domain("api.github.com").unwrap();
        matches!(cred, Credential::GitHubOAuth(_));

        // Other domain should return HTTP Basic
        let cred = config.get_credential_for_domain("example.com").unwrap();
        matches!(cred, Credential::HttpBasic { .. });
    }
}
