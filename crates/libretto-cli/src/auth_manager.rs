//! Unified authentication manager for all Composer-compatible auth types.
//!
//! Provides interactive prompts and credential management for:
//! - GitHub OAuth (promptable)
//! - GitLab OAuth (promptable)
//! - GitLab Token (promptable)
//! - Bitbucket OAuth (promptable)
//! - Forgejo Token (promptable)
//! - HTTP Basic (promptable)
//! - Bearer tokens (not promptable)
//! - Custom headers (not promptable)
//! - Client certificates (not promptable)

use crate::output::{info, success, warning};
use anyhow::{Context, Result};
use dialoguer::{Input, Password, theme::ColorfulTheme};
use libretto_config::auth::{Credential, CredentialStore};
use std::io::IsTerminal;
use std::path::Path;

/// Authentication type that can be prompted interactively.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptableAuthType {
    /// GitHub OAuth token.
    GitHubOAuth,
    /// GitLab OAuth token.
    GitLabOAuth,
    /// GitLab private token.
    GitLabToken,
    /// Bitbucket OAuth (consumer key/secret).
    BitbucketOAuth,
    /// Forgejo/Gitea token.
    ForgejoToken,
    /// HTTP Basic authentication.
    HttpBasic,
}

/// Unified authentication manager.
#[derive(Debug)]
pub struct AuthManager {
    /// Credential store with loaded auth configs.
    store: CredentialStore,
    /// Machine hostname for token naming.
    hostname: String,
    /// Whether we're in interactive mode.
    interactive: bool,
}

impl AuthManager {
    /// Create a new auth manager.
    #[must_use]
    pub fn new() -> Self {
        Self::with_project_root(None)
    }

    /// Create an auth manager with a specific project root.
    #[must_use]
    pub fn with_project_root(project_root: Option<&Path>) -> Self {
        let store = CredentialStore::with_project_root(project_root);
        let hostname = gethostname::gethostname().to_string_lossy().to_string();
        let interactive = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();

        Self {
            store,
            hostname,
            interactive,
        }
    }

    /// Get the credential store.
    #[must_use]
    pub fn store(&self) -> &CredentialStore {
        &self.store
    }

    /// Get a mutable reference to the credential store.
    pub fn store_mut(&mut self) -> &mut CredentialStore {
        &mut self.store
    }

    /// Get credential for a domain.
    #[must_use]
    pub fn get_credential(&self, domain: &str) -> Option<Credential> {
        self.store.get_credential(domain)
    }

    /// Prompt for a specific authentication type.
    pub fn prompt_for_auth_type(
        &mut self,
        domain: &str,
        auth_type: PromptableAuthType,
        reason: &str,
    ) -> Result<Option<Credential>> {
        if !self.interactive {
            warning("Non-interactive mode - cannot prompt for credentials.");
            info(&format!(
                "Set credentials manually with: libretto config --global <auth-type>.{domain} <value>"
            ));
            return Ok(None);
        }

        if !reason.is_empty() {
            eprintln!();
            info(reason);
            eprintln!();
        }

        match auth_type {
            PromptableAuthType::GitHubOAuth => self.prompt_github_oauth(domain),
            PromptableAuthType::GitLabOAuth => self.prompt_gitlab_oauth(domain),
            PromptableAuthType::GitLabToken => self.prompt_gitlab_token(domain),
            PromptableAuthType::BitbucketOAuth => self.prompt_bitbucket_oauth(domain),
            PromptableAuthType::ForgejoToken => self.prompt_forgejo_token(domain),
            PromptableAuthType::HttpBasic => self.prompt_http_basic(domain),
        }
    }

    // ========== GitHub OAuth ==========

    /// Prompt for GitHub OAuth token.
    fn prompt_github_oauth(&mut self, domain: &str) -> Result<Option<Credential>> {
        self.show_github_instructions();

        let token = self.read_token_input("Token (hidden)")?;

        if token.is_empty() {
            self.show_abort_message("github-oauth", domain);
            return Ok(None);
        }

        // Validate token format
        if !Self::validate_github_token(&token) {
            warning(
                "Token format appears invalid. GitHub tokens should start with 'ghp_', 'gho_', 'ghu_', 'ghs_', 'ghr_', or 'github_pat_'.",
            );
            info("Proceeding anyway...");
        }

        // Save the token
        self.store.config_mut().set_github_oauth(domain, &token);
        self.store.save_global()?;

        success("Token saved successfully!");

        Ok(Some(Credential::GitHubOAuth(token)))
    }

    /// Show GitHub token creation instructions.
    fn show_github_instructions(&self) {
        let date = chrono::Local::now().format("%Y-%m-%d+%H%M");
        let name = format!("Libretto+on+{}+{}", self.hostname, date);

        let public_url =
            format!("https://github.com/settings/personal-access-tokens/new?name={name}");
        let private_url = format!(
            "https://github.com/settings/personal-access-tokens/new?contents=read&name={name}"
        );
        let classic_url =
            format!("https://github.com/settings/tokens/new?scopes=repo&description={name}");

        println!("You need to provide a GitHub access token.");
        println!("Tokens will be stored in plain text for future use by Libretto.");
        println!("Due to the security risk of tokens being exfiltrated, use tokens with short");
        println!("expiration times and only the minimum permissions necessary.");
        println!();
        println!("Carefully consider the following options in order:");
        println!();
        println!(
            "1. When you don't use 'vcs' type 'repositories' in composer.json and do not need"
        );
        println!(
            "   to clone source or download dist files from private GitHub repositories over HTTPS,"
        );
        println!("   use a fine-grained token with read-only access to public information.");
        println!("   Use the following URL to create such a token:");
        println!("   {public_url}");
        println!();
        println!(
            "2. When all relevant _private_ GitHub repositories belong to a single user or organisation,"
        );
        println!("   use a fine-grained token with repository \"content\" read-only permissions.");
        println!(
            "   You can start with the following URL, but you may need to change the resource owner"
        );
        println!(
            "   to the right user or organisation. Additionally, you can scope permissions down to"
        );
        println!("   apply only to selected repositories.");
        println!("   {private_url}");
        println!();
        println!(
            "3. A \"classic\" token grants broad permissions on your behalf to all repositories"
        );
        println!(
            "   accessible by you. This may include write permissions, even though not needed by Libretto."
        );
        println!(
            "   Use it only when you need to access private repositories across multiple organisations"
        );
        println!(
            "   at the same time and using directory-specific authentication sources is not an option."
        );
        println!("   You can generate a classic token here:");
        println!("   {classic_url}");
        println!();
        println!("For additional information, check:");
        println!(
            "https://getcomposer.org/doc/articles/authentication-for-private-packages.md#github-oauth"
        );
    }

    /// Validate GitHub token format.
    fn validate_github_token(token: &str) -> bool {
        token.starts_with("ghp_")
            || token.starts_with("gho_")
            || token.starts_with("ghu_")
            || token.starts_with("ghs_")
            || token.starts_with("ghr_")
            || token.starts_with("github_pat_")
            || (token.len() == 40 && token.chars().all(|c| c.is_ascii_hexdigit()))
    }

    // ========== GitLab OAuth ==========

    /// Prompt for GitLab OAuth token.
    fn prompt_gitlab_oauth(&mut self, domain: &str) -> Result<Option<Credential>> {
        self.show_gitlab_oauth_instructions(domain);

        let token = self.read_token_input("Token (hidden)")?;

        if token.is_empty() {
            self.show_abort_message("gitlab-oauth", domain);
            return Ok(None);
        }

        // Save the token
        self.store.config_mut().set_gitlab_oauth(domain, &token);
        self.store.save_global()?;

        success("Token saved successfully!");

        Ok(Some(Credential::GitLabOAuth(token)))
    }

    /// Show GitLab OAuth instructions.
    fn show_gitlab_oauth_instructions(&self, domain: &str) {
        let base_url = if domain == "gitlab.com" {
            "https://gitlab.com".to_string()
        } else {
            format!("https://{domain}")
        };

        println!("You need to provide a GitLab OAuth token to access {domain}.");
        println!();
        println!("Head to {base_url}/-/user_settings/personal_access_tokens");
        println!("to retrieve a token. It needs the \"read_api\" scope.");
        println!();
        println!("For additional information, check:");
        println!(
            "https://getcomposer.org/doc/articles/authentication-for-private-packages.md#gitlab-oauth"
        );
    }

    // ========== GitLab Token ==========

    /// Prompt for GitLab private token.
    fn prompt_gitlab_token(&mut self, domain: &str) -> Result<Option<Credential>> {
        self.show_gitlab_token_instructions(domain);

        let token = self.read_token_input("Token (hidden)")?;

        if token.is_empty() {
            self.show_abort_message("gitlab-token", domain);
            return Ok(None);
        }

        // Save the token
        self.store.config_mut().set_gitlab_token(domain, &token);
        self.store.save_global()?;

        success("Token saved successfully!");

        Ok(Some(Credential::GitLabToken(token)))
    }

    /// Show GitLab private token instructions.
    fn show_gitlab_token_instructions(&self, domain: &str) {
        let base_url = if domain == "gitlab.com" {
            "https://gitlab.com".to_string()
        } else {
            format!("https://{domain}")
        };

        println!("You need to provide a GitLab private token to access {domain}.");
        println!();
        println!("Head to {base_url}/-/user_settings/personal_access_tokens");
        println!("to retrieve a token. It needs the \"api\" or \"read_api\" scope.");
        println!();
        println!("For job tokens, you can use a CI_JOB_TOKEN environment variable.");
        println!();
        println!("For additional information, check:");
        println!(
            "https://getcomposer.org/doc/articles/authentication-for-private-packages.md#gitlab-token"
        );
    }

    // ========== Bitbucket OAuth ==========

    /// Prompt for Bitbucket OAuth credentials.
    fn prompt_bitbucket_oauth(&mut self, domain: &str) -> Result<Option<Credential>> {
        self.show_bitbucket_instructions(domain);

        let theme = ColorfulTheme::default();

        let consumer_key: String = Input::with_theme(&theme)
            .with_prompt("Consumer Key")
            .allow_empty(true)
            .interact_text()
            .context("Failed to read consumer key")?;

        if consumer_key.is_empty() {
            self.show_abort_message("bitbucket-oauth", domain);
            return Ok(None);
        }

        let consumer_secret = self.read_token_input("Consumer Secret (hidden)")?;

        if consumer_secret.is_empty() {
            self.show_abort_message("bitbucket-oauth", domain);
            return Ok(None);
        }

        // Save the credentials
        self.store
            .config_mut()
            .set_bitbucket_oauth(domain, &consumer_key, &consumer_secret);
        self.store.save_global()?;

        success("Bitbucket OAuth credentials saved successfully!");

        Ok(Some(Credential::BitbucketOAuth {
            consumer_key,
            consumer_secret,
        }))
    }

    /// Show Bitbucket OAuth instructions.
    fn show_bitbucket_instructions(&self, domain: &str) {
        println!("You need to provide Bitbucket OAuth consumer credentials to access {domain}.");
        println!();
        println!("Head to https://bitbucket.org/account/settings/app-passwords/");
        println!(
            "to create an App Password, or to https://support.atlassian.com/bitbucket-cloud/docs/use-oauth-on-bitbucket-cloud/"
        );
        println!("for information on setting up an OAuth consumer.");
        println!();
        println!("For App Passwords, use your Bitbucket username as the Consumer Key");
        println!("and the App Password as the Consumer Secret.");
        println!();
        println!("For additional information, check:");
        println!(
            "https://getcomposer.org/doc/articles/authentication-for-private-packages.md#bitbucket-oauth"
        );
    }

    // ========== Forgejo Token ==========

    /// Prompt for Forgejo/Gitea token.
    fn prompt_forgejo_token(&mut self, domain: &str) -> Result<Option<Credential>> {
        self.show_forgejo_instructions(domain);

        let theme = ColorfulTheme::default();

        let username: String = Input::with_theme(&theme)
            .with_prompt("Username")
            .allow_empty(true)
            .interact_text()
            .context("Failed to read username")?;

        if username.is_empty() {
            self.show_abort_message("forgejo-token", domain);
            return Ok(None);
        }

        let token = self.read_token_input("Token (hidden)")?;

        if token.is_empty() {
            self.show_abort_message("forgejo-token", domain);
            return Ok(None);
        }

        // Save the credentials
        self.store
            .config_mut()
            .set_forgejo_token(domain, &username, &token);
        self.store.save_global()?;

        success("Forgejo token saved successfully!");

        Ok(Some(Credential::ForgejoToken { username, token }))
    }

    /// Show Forgejo/Gitea token instructions.
    fn show_forgejo_instructions(&self, domain: &str) {
        println!("You need to provide Forgejo/Gitea credentials to access {domain}.");
        println!();
        println!("Head to https://{domain}/user/settings/applications");
        println!("to create an access token.");
        println!();
        println!("The token needs at least \"read:repository\" scope for private repos.");
        println!();
        println!("For additional information, check:");
        println!(
            "https://getcomposer.org/doc/articles/authentication-for-private-packages.md#forgejo-token"
        );
    }

    // ========== HTTP Basic ==========

    /// Prompt for HTTP Basic credentials.
    fn prompt_http_basic(&mut self, domain: &str) -> Result<Option<Credential>> {
        self.show_http_basic_instructions(domain);

        let theme = ColorfulTheme::default();

        let username: String = Input::with_theme(&theme)
            .with_prompt("Username")
            .allow_empty(true)
            .interact_text()
            .context("Failed to read username")?;

        if username.is_empty() {
            self.show_abort_message("http-basic", domain);
            return Ok(None);
        }

        let password = self.read_token_input("Password (hidden)")?;

        if password.is_empty() {
            self.show_abort_message("http-basic", domain);
            return Ok(None);
        }

        // Save the credentials
        self.store
            .config_mut()
            .set_http_basic(domain, &username, &password);
        self.store.save_global()?;

        success("HTTP Basic credentials saved successfully!");

        Ok(Some(Credential::HttpBasic { username, password }))
    }

    /// Show HTTP Basic instructions.
    fn show_http_basic_instructions(&self, domain: &str) {
        println!("You need to provide HTTP Basic credentials to access {domain}.");
        println!();
        println!("This is typically used for private Composer repositories like:");
        println!("- Private Packagist");
        println!("- Satis/Toran Proxy");
        println!("- Other private package repositories");
        println!();
        println!("For additional information, check:");
        println!(
            "https://getcomposer.org/doc/articles/authentication-for-private-packages.md#http-basic"
        );
    }

    // ========== Helpers ==========

    /// Read a token or password (hidden input).
    fn read_token_input(&self, prompt: &str) -> Result<String> {
        let theme = ColorfulTheme::default();

        Password::with_theme(&theme)
            .with_prompt(prompt)
            .allow_empty_password(true)
            .interact()
            .context("Failed to read input")
    }

    /// Show abort message with manual config hint.
    fn show_abort_message(&self, auth_type: &str, domain: &str) {
        warning("No credentials given, aborting.");
        info("You can also add credentials manually later by using:");
        info(&format!(
            "  libretto config --global {auth_type}.{domain} <value>"
        ));
    }

    /// Save the current auth config.
    pub fn save(&self) -> Result<()> {
        self.store.save_global().map_err(Into::into)
    }
}

impl Default for AuthManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Rate limit information for GitHub API.
#[derive(Debug, Clone)]
pub struct GitHubRateLimitInfo {
    /// The URL that was rate-limited.
    pub url: String,
    /// Maximum requests per hour.
    pub limit: u32,
    /// Unix timestamp when the rate limit resets.
    pub reset: u64,
}

impl GitHubRateLimitInfo {
    /// Format the reset time as a human-readable string.
    #[must_use]
    pub fn reset_time_string(&self) -> String {
        use chrono::{Local, TimeZone};
        if let Some(dt) = Local.timestamp_opt(self.reset as i64, 0).single() {
            dt.format("%Y-%m-%d %H:%M:%S").to_string()
        } else {
            format!("timestamp {}", self.reset)
        }
    }
}

/// Parse rate limit information from HTTP response headers.
pub fn parse_rate_limit_headers(
    url: &str,
    headers: &reqwest::header::HeaderMap,
) -> Option<GitHubRateLimitInfo> {
    let limit = headers
        .get("x-ratelimit-limit")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or(60);

    let reset = headers
        .get("x-ratelimit-reset")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    Some(GitHubRateLimitInfo {
        url: url.to_string(),
        limit,
        reset,
    })
}

/// Check if an error is a GitHub rate limit error.
pub fn is_github_rate_limit_error(url: &str, status: u16) -> bool {
    (url.contains("api.github.com")
        || url.contains("github.com")
        || url.contains("codeload.github.com"))
        && (status == 403 || status == 429)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_github_token() {
        assert!(AuthManager::validate_github_token(
            "ghp_xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
        ));
        assert!(AuthManager::validate_github_token("github_pat_xxx"));
        assert!(AuthManager::validate_github_token(
            "0123456789abcdef0123456789abcdef01234567"
        ));
        assert!(!AuthManager::validate_github_token("invalid"));
    }

    #[test]
    fn test_is_github_rate_limit_error() {
        assert!(is_github_rate_limit_error(
            "https://api.github.com/test",
            403
        ));
        assert!(is_github_rate_limit_error(
            "https://codeload.github.com/test",
            429
        ));
        assert!(!is_github_rate_limit_error(
            "https://packagist.org/test",
            403
        ));
        assert!(!is_github_rate_limit_error(
            "https://api.github.com/test",
            200
        ));
    }
}
