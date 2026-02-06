//! Git credentials handling for SSH, tokens, and credential helpers.

use crate::error::{Result, VcsError};
use crate::types::VcsCredentials;
use dashmap::DashMap;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use tracing::{debug, trace, warn};

/// Global credential cache shared across operations.
static CREDENTIAL_CACHE: std::sync::LazyLock<DashMap<String, CachedCredential>> =
    std::sync::LazyLock::new(DashMap::new);

/// Cached credential with expiry.
#[derive(Debug, Clone)]
struct CachedCredential {
    credentials: VcsCredentials,
    /// Timestamp when cached (for potential TTL).
    #[allow(dead_code)]
    cached_at: std::time::Instant,
}

/// SSH known hosts entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnownHost {
    /// Hostname.
    pub host: String,
    /// Key type (ssh-rsa, ssh-ed25519, etc.).
    pub key_type: String,
    /// Public key (base64).
    pub public_key: String,
}

/// Credential manager for VCS operations.
#[derive(Debug)]
pub struct CredentialManager {
    /// SSH key paths to try.
    ssh_keys: Vec<PathBuf>,
    /// Auth.json credentials by host.
    auth_json: Arc<RwLock<HashMap<String, AuthJsonEntry>>>,
    /// Use SSH agent.
    use_ssh_agent: bool,
    /// Known hosts file path.
    known_hosts_path: Option<PathBuf>,
    /// Allow unknown hosts.
    allow_unknown_hosts: bool,
}

/// Entry from auth.json file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthJsonEntry {
    /// Authentication type.
    #[serde(rename = "type")]
    pub auth_type: Option<String>,
    /// Username.
    pub username: Option<String>,
    /// Password or token.
    pub password: Option<String>,
    /// OAuth token.
    pub token: Option<String>,
    /// Bearer token.
    pub bearer: Option<String>,
}

impl Default for CredentialManager {
    fn default() -> Self {
        Self::new()
    }
}

impl CredentialManager {
    /// Create a new credential manager with default settings.
    #[must_use]
    pub fn new() -> Self {
        let home = dirs_home();
        let ssh_dir = home.as_ref().map(|h| h.join(".ssh"));

        let mut ssh_keys = Vec::new();
        if let Some(ssh_dir) = &ssh_dir {
            // Common SSH key paths in order of preference
            for key_name in &["id_ed25519", "id_rsa", "id_ecdsa", "id_dsa"] {
                let key_path = ssh_dir.join(key_name);
                if key_path.exists() {
                    ssh_keys.push(key_path);
                }
            }
        }

        let known_hosts_path = ssh_dir.map(|d| d.join("known_hosts"));

        Self {
            ssh_keys,
            auth_json: Arc::new(RwLock::new(HashMap::new())),
            use_ssh_agent: true,
            known_hosts_path,
            allow_unknown_hosts: false,
        }
    }

    /// Add an SSH key path.
    pub fn add_ssh_key(&mut self, path: PathBuf) {
        if !self.ssh_keys.contains(&path) {
            self.ssh_keys.push(path);
        }
    }

    /// Load credentials from auth.json file.
    ///
    /// # Errors
    /// Returns error if file cannot be read or parsed.
    pub fn load_auth_json(&self, path: &Path) -> Result<()> {
        let content = std::fs::read_to_string(path).map_err(|e| VcsError::io(path, e))?;

        #[derive(Deserialize)]
        struct AuthJson {
            #[serde(rename = "http-basic")]
            http_basic: Option<HashMap<String, AuthJsonEntry>>,
            #[serde(rename = "github-oauth")]
            github_oauth: Option<HashMap<String, String>>,
            #[serde(rename = "gitlab-oauth")]
            gitlab_oauth: Option<HashMap<String, String>>,
            #[serde(rename = "gitlab-token")]
            gitlab_token: Option<HashMap<String, String>>,
            #[serde(rename = "bitbucket-oauth")]
            bitbucket_oauth: Option<HashMap<String, BitbucketAuth>>,
            bearer: Option<HashMap<String, String>>,
        }

        #[derive(Deserialize)]
        struct BitbucketAuth {
            consumer_key: Option<String>,
            consumer_secret: Option<String>,
        }

        let auth: AuthJson = sonic_rs::from_str(&content)
            .map_err(|e| VcsError::git(format!("failed to parse auth.json: {e}")))?;

        let mut entries = self.auth_json.write();

        // Process http-basic auth
        if let Some(http_basic) = auth.http_basic {
            for (host, entry) in http_basic {
                entries.insert(host, entry);
            }
        }

        // Process GitHub OAuth tokens
        if let Some(github_oauth) = auth.github_oauth {
            for (host, token) in github_oauth {
                entries.insert(
                    host,
                    AuthJsonEntry {
                        auth_type: Some("github-oauth".to_string()),
                        username: Some("x-access-token".to_string()),
                        password: None,
                        token: Some(token),
                        bearer: None,
                    },
                );
            }
        }

        // Process GitLab OAuth tokens
        if let Some(gitlab_oauth) = auth.gitlab_oauth {
            for (host, token) in gitlab_oauth {
                entries.insert(
                    host,
                    AuthJsonEntry {
                        auth_type: Some("gitlab-oauth".to_string()),
                        username: Some("oauth2".to_string()),
                        password: None,
                        token: Some(token),
                        bearer: None,
                    },
                );
            }
        }

        // Process GitLab tokens
        if let Some(gitlab_token) = auth.gitlab_token {
            for (host, token) in gitlab_token {
                entries.insert(
                    host,
                    AuthJsonEntry {
                        auth_type: Some("gitlab-token".to_string()),
                        username: None,
                        password: None,
                        token: Some(token),
                        bearer: None,
                    },
                );
            }
        }

        // Process bearer tokens
        if let Some(bearers) = auth.bearer {
            for (host, token) in bearers {
                entries.insert(
                    host,
                    AuthJsonEntry {
                        auth_type: Some("bearer".to_string()),
                        username: None,
                        password: None,
                        token: None,
                        bearer: Some(token),
                    },
                );
            }
        }

        // Process Bitbucket OAuth (consumer key/secret for OAuth 1.0)
        if let Some(bitbucket_oauth) = auth.bitbucket_oauth {
            for (host, bb_auth) in bitbucket_oauth {
                if let (Some(key), Some(secret)) = (bb_auth.consumer_key, bb_auth.consumer_secret) {
                    entries.insert(
                        host,
                        AuthJsonEntry {
                            auth_type: Some("bitbucket-oauth".to_string()),
                            username: Some(key),
                            password: Some(secret),
                            token: None,
                            bearer: None,
                        },
                    );
                }
            }
        }

        debug!(count = entries.len(), "loaded auth.json credentials");
        Ok(())
    }

    /// Get credentials for a host.
    #[must_use]
    pub fn get_credentials(&self, host: &str) -> VcsCredentials {
        // Check cache first
        if let Some(cached) = CREDENTIAL_CACHE.get(host) {
            trace!(host, "using cached credentials");
            return cached.credentials.clone();
        }

        // Check auth.json entries
        let entries = self.auth_json.read();
        if let Some(entry) = entries.get(host) {
            let creds = self.entry_to_credentials(entry);
            self.cache_credentials(host, creds.clone());
            return creds;
        }

        // Check for wildcard matches (e.g., *.github.com)
        for (pattern, entry) in entries.iter() {
            if let Some(suffix) = pattern.strip_prefix('*')
                && host.ends_with(suffix)
            {
                let creds = self.entry_to_credentials(entry);
                self.cache_credentials(host, creds.clone());
                return creds;
            }
        }

        drop(entries);

        // Try SSH agent if enabled
        if self.use_ssh_agent && self.ssh_agent_available() {
            trace!(host, "using ssh agent");
            return VcsCredentials::SshAgent;
        }

        // Try SSH keys
        if let Some(key_path) = self.find_ssh_key() {
            trace!(host, key = ?key_path, "using ssh key");
            return VcsCredentials::ssh_key(key_path, None);
        }

        VcsCredentials::None
    }

    /// Convert auth.json entry to credentials.
    fn entry_to_credentials(&self, entry: &AuthJsonEntry) -> VcsCredentials {
        if let Some(bearer) = &entry.bearer {
            return VcsCredentials::UserPass {
                username: "bearer".to_string(),
                password: bearer.clone(),
            };
        }

        if let Some(token) = &entry.token {
            let username = entry
                .username
                .clone()
                .unwrap_or_else(|| "x-access-token".to_string());
            return VcsCredentials::UserPass {
                username,
                password: token.clone(),
            };
        }

        if let (Some(username), Some(password)) = (&entry.username, &entry.password) {
            return VcsCredentials::UserPass {
                username: username.clone(),
                password: password.clone(),
            };
        }

        VcsCredentials::None
    }

    /// Cache credentials for a host.
    fn cache_credentials(&self, host: &str, credentials: VcsCredentials) {
        CREDENTIAL_CACHE.insert(
            host.to_string(),
            CachedCredential {
                credentials,
                cached_at: std::time::Instant::now(),
            },
        );
    }

    /// Check if SSH agent is available.
    fn ssh_agent_available(&self) -> bool {
        std::env::var("SSH_AUTH_SOCK").is_ok()
    }

    /// Find an available SSH key.
    fn find_ssh_key(&self) -> Option<PathBuf> {
        self.ssh_keys.iter().find(|p| p.exists()).cloned()
    }

    /// Verify SSH host key.
    ///
    /// # Errors
    /// Returns error if host key verification fails.
    pub fn verify_host_key(&self, host: &str, key_type: &str, public_key: &[u8]) -> Result<bool> {
        let Some(known_hosts_path) = &self.known_hosts_path else {
            if self.allow_unknown_hosts {
                warn!(host, "skipping host key verification (no known_hosts)");
                return Ok(true);
            }
            return Err(VcsError::HostKeyVerification {
                host: host.to_string(),
            });
        };

        if !known_hosts_path.exists() {
            if self.allow_unknown_hosts {
                warn!(host, "known_hosts file not found, allowing");
                return Ok(true);
            }
            return Err(VcsError::HostKeyVerification {
                host: host.to_string(),
            });
        }

        let content = std::fs::read_to_string(known_hosts_path)
            .map_err(|e| VcsError::io(known_hosts_path, e))?;

        let public_key_b64 = base64_encode(public_key);

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 3 {
                continue;
            }

            let hosts = parts[0];
            let entry_key_type = parts[1];
            let entry_public_key = parts[2];

            // Check if host matches
            let host_matches = hosts.split(',').any(|h| {
                let h = h.trim();
                if h.starts_with('[') {
                    // [host]:port format
                    h.contains(host)
                } else {
                    h == host || (h.starts_with('*') && host.ends_with(&h[1..]))
                }
            });

            if host_matches && entry_key_type == key_type && entry_public_key == public_key_b64 {
                debug!(host, "host key verified");
                return Ok(true);
            }
        }

        if self.allow_unknown_hosts {
            warn!(host, "unknown host key, allowing");
            return Ok(true);
        }

        Err(VcsError::HostKeyVerification {
            host: host.to_string(),
        })
    }

    /// Get credentials using Git credential helper.
    ///
    /// # Errors
    /// Returns error if credential helper fails.
    pub fn get_from_credential_helper(&self, url: &str) -> Result<Option<VcsCredentials>> {
        // Try to get the credential helper from git config
        let output = Command::new("git")
            .args(["config", "--get", "credential.helper"])
            .output()
            .map_err(|e| VcsError::Command {
                command: "git config".to_string(),
                message: e.to_string(),
                exit_code: None,
            })?;

        if !output.status.success() {
            return Ok(None);
        }

        let helper = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if helper.is_empty() {
            return Ok(None);
        }

        debug!(helper = %helper, url = %url, "using credential helper");

        // Format the credential request
        let parsed_url = url::Url::parse(url).map_err(|e| VcsError::InvalidUrl {
            url: url.to_string(),
            reason: e.to_string(),
        })?;

        let protocol = parsed_url.scheme();
        let host = parsed_url.host_str().unwrap_or("");
        let path = parsed_url.path();

        let input = format!("protocol={protocol}\nhost={host}\npath={path}\n");

        // Run the credential helper
        let helper_cmd = if let Some(stripped) = helper.strip_prefix('!') {
            // Shell command
            stripped
        } else {
            &helper
        };

        let output = Command::new("git")
            .args(["credential", helper_cmd, "get"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write;
                if let Some(stdin) = child.stdin.as_mut() {
                    stdin.write_all(input.as_bytes())?;
                }
                child.wait_with_output()
            })
            .map_err(|e| VcsError::Command {
                command: format!("git credential {helper_cmd} get"),
                message: e.to_string(),
                exit_code: None,
            })?;

        if !output.status.success() {
            return Ok(None);
        }

        // Parse the output
        let response = String::from_utf8_lossy(&output.stdout);
        let mut username = None;
        let mut password = None;

        for line in response.lines() {
            if let Some((key, value)) = line.split_once('=') {
                match key {
                    "username" => username = Some(value.to_string()),
                    "password" => password = Some(value.to_string()),
                    _ => {}
                }
            }
        }

        if let (Some(user), Some(pass)) = (username, password) {
            Ok(Some(VcsCredentials::UserPass {
                username: user,
                password: pass,
            }))
        } else {
            Ok(None)
        }
    }

    /// Set to allow unknown hosts (less secure).
    pub const fn allow_unknown_hosts(&mut self, allow: bool) {
        self.allow_unknown_hosts = allow;
    }

    /// Disable SSH agent usage.
    pub const fn disable_ssh_agent(&mut self) {
        self.use_ssh_agent = false;
    }

    /// Clear cached credentials.
    pub fn clear_cache() {
        CREDENTIAL_CACHE.clear();
    }

    /// Clear cached credentials for a specific host.
    pub fn clear_cache_for_host(host: &str) {
        CREDENTIAL_CACHE.remove(host);
    }
}

/// Get home directory.
fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// Simple base64 encoding for host key comparison.
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
    fn credential_manager_default() {
        let manager = CredentialManager::new();
        assert!(manager.use_ssh_agent);
    }

    #[test]
    fn get_credentials_none() {
        let manager = CredentialManager::new();
        let creds = manager.get_credentials("unknown-host.example.com");
        // Will return SSH agent or key if available, otherwise None
        assert!(matches!(
            creds,
            VcsCredentials::SshAgent | VcsCredentials::SshKey { .. } | VcsCredentials::None
        ));
    }

    #[test]
    fn entry_to_credentials_token() {
        let manager = CredentialManager::new();
        let entry = AuthJsonEntry {
            auth_type: Some("github-oauth".to_string()),
            username: Some("x-access-token".to_string()),
            password: None,
            token: Some("ghp_test123".to_string()),
            bearer: None,
        };
        let creds = manager.entry_to_credentials(&entry);
        assert!(matches!(creds, VcsCredentials::UserPass { .. }));
    }

    #[test]
    fn cache_credentials() {
        let manager = CredentialManager::new();
        let creds = VcsCredentials::user_pass("user", "pass");
        manager.cache_credentials("test.example.com", creds);

        let cached = manager.get_credentials("test.example.com");
        assert!(matches!(cached, VcsCredentials::UserPass { .. }));

        CredentialManager::clear_cache();
    }

    #[test]
    fn base64_encode_test() {
        assert_eq!(base64_encode(b"hello"), "aGVsbG8=");
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"a"), "YQ==");
    }
}
