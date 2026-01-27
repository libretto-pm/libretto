//! HTTP client with HTTP/2 multiplexing and connection pooling.
//!
//! Provides a high-performance HTTP client optimized for parallel downloads.

use crate::config::{AuthConfig, DownloadConfig};
use crate::error::{DownloadError, Result};
use reqwest::{
    header::{HeaderMap, HeaderValue, ACCEPT, ACCEPT_ENCODING, AUTHORIZATION, RANGE, USER_AGENT},
    Client, Response, StatusCode,
};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, trace};
use url::Url;

/// High-performance HTTP client with connection pooling and HTTP/2 support.
#[derive(Clone)]
pub struct HttpClient {
    client: Client,
    config: Arc<DownloadConfig>,
    auth: Option<Arc<AuthConfig>>,
}

impl std::fmt::Debug for HttpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpClient")
            .field("client", &"reqwest::Client")
            .field("config", &self.config)
            .field("auth", &self.auth.is_some())
            .finish()
    }
}

impl HttpClient {
    /// Create a new HTTP client with the given configuration.
    ///
    /// # Errors
    /// Returns error if client cannot be built.
    pub fn new(config: DownloadConfig, auth: Option<AuthConfig>) -> Result<Self> {
        let mut builder = Client::builder()
            .connect_timeout(config.connect_timeout)
            .read_timeout(config.read_timeout)
            .timeout(config.total_timeout)
            .pool_max_idle_per_host(config.max_connections_per_host)
            .tcp_keepalive(if config.keep_alive {
                Some(config.keep_alive_timeout)
            } else {
                None
            })
            .tcp_nodelay(true)
            .gzip(true)
            .brotli(true)
            .deflate(true)
            .zstd(true)
            .redirect(reqwest::redirect::Policy::limited(10))
            .use_rustls_tls();

        // Enable HTTP/2 multiplexing
        if config.http2_multiplexing {
            builder = builder
                .http2_prior_knowledge()
                .http2_adaptive_window(config.http2_adaptive_window)
                .http2_initial_stream_window_size(config.http2_initial_stream_window)
                .http2_initial_connection_window_size(config.http2_initial_connection_window);
        }

        // Configure proxy from config or environment
        if let Some(ref proxy_url) = config.proxy {
            let proxy =
                reqwest::Proxy::all(proxy_url).map_err(|e| DownloadError::Config(e.to_string()))?;
            builder = builder.proxy(proxy);
        } else {
            // Use system proxy settings from environment
            builder = builder.proxy(reqwest::Proxy::custom(|url| {
                // Check HTTPS_PROXY, HTTP_PROXY, NO_PROXY
                let scheme = url.scheme();
                let host = url.host_str().unwrap_or("");

                // Check NO_PROXY first
                if let Ok(no_proxy) =
                    std::env::var("NO_PROXY").or_else(|_| std::env::var("no_proxy"))
                {
                    for pattern in no_proxy.split(',') {
                        let pattern = pattern.trim();
                        if pattern == "*" || host.ends_with(pattern) || host == pattern {
                            return None;
                        }
                    }
                }

                // Get proxy URL based on scheme
                let proxy_var = if scheme == "https" {
                    std::env::var("HTTPS_PROXY")
                        .or_else(|_| std::env::var("https_proxy"))
                        .or_else(|_| std::env::var("HTTP_PROXY"))
                        .or_else(|_| std::env::var("http_proxy"))
                } else {
                    std::env::var("HTTP_PROXY").or_else(|_| std::env::var("http_proxy"))
                };

                proxy_var.ok().and_then(|p| p.parse::<reqwest::Url>().ok())
            }));
        }

        let client = builder
            .build()
            .map_err(|e| DownloadError::Config(e.to_string()))?;

        Ok(Self {
            client,
            config: Arc::new(config),
            auth: auth.map(Arc::new),
        })
    }

    /// Create a client with default configuration.
    ///
    /// # Errors
    /// Returns error if client cannot be built.
    pub fn with_defaults() -> Result<Self> {
        Self::new(DownloadConfig::default(), None)
    }

    /// Get the underlying reqwest client.
    #[must_use]
    pub const fn inner(&self) -> &Client {
        &self.client
    }

    /// Get the configuration.
    #[must_use]
    pub const fn config(&self) -> &Arc<DownloadConfig> {
        &self.config
    }

    /// Send a GET request.
    ///
    /// # Errors
    /// Returns error if request fails.
    pub async fn get(&self, url: &Url) -> Result<Response> {
        let mut headers = self.default_headers();
        self.add_auth_headers(&mut headers, url);

        debug!(url = %url, "GET request");

        let response = self
            .client
            .get(url.as_str())
            .headers(headers)
            .send()
            .await?;

        self.check_response(response).await
    }

    /// Send a GET request with Range header for resuming downloads.
    ///
    /// # Errors
    /// Returns error if request fails.
    pub async fn get_range(&self, url: &Url, start: u64, end: Option<u64>) -> Result<Response> {
        let mut headers = self.default_headers();
        self.add_auth_headers(&mut headers, url);

        let range_value = end.map_or_else(
            || format!("bytes={start}-"),
            |e| format!("bytes={start}-{e}"),
        );

        headers.insert(
            RANGE,
            HeaderValue::from_str(&range_value)
                .map_err(|e| DownloadError::Config(e.to_string()))?,
        );

        debug!(url = %url, range = %range_value, "GET range request");

        let response = self
            .client
            .get(url.as_str())
            .headers(headers)
            .send()
            .await?;

        self.check_response(response).await
    }

    /// Send a HEAD request to get content info without downloading.
    ///
    /// # Errors
    /// Returns error if request fails.
    pub async fn head(&self, url: &Url) -> Result<Response> {
        let mut headers = self.default_headers();
        self.add_auth_headers(&mut headers, url);

        trace!(url = %url, "HEAD request");

        let response = self
            .client
            .head(url.as_str())
            .headers(headers)
            .send()
            .await?;

        self.check_response(response).await
    }

    /// Check if server supports range requests.
    ///
    /// # Errors
    /// Returns error if request fails.
    pub async fn supports_range(&self, url: &Url) -> Result<bool> {
        let response = self.head(url).await?;

        let accepts_ranges = response
            .headers()
            .get("accept-ranges")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v != "none");

        Ok(accepts_ranges)
    }

    /// Get content length from HEAD request.
    ///
    /// # Errors
    /// Returns error if request fails.
    pub async fn content_length(&self, url: &Url) -> Result<Option<u64>> {
        let response = self.head(url).await?;
        Ok(response.content_length())
    }

    fn default_headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();

        if let Ok(ua) = HeaderValue::from_str(&self.config.user_agent) {
            headers.insert(USER_AGENT, ua);
        }

        if let Ok(enc) = HeaderValue::from_str(&self.config.accept_encoding) {
            headers.insert(ACCEPT_ENCODING, enc);
        }

        headers.insert(ACCEPT, HeaderValue::from_static("*/*"));

        headers
    }

    fn add_auth_headers(&self, headers: &mut HeaderMap, url: &Url) {
        let Some(ref auth) = self.auth else {
            return;
        };

        let Some(host) = url.host_str() else {
            return;
        };

        // Check HTTP Basic auth
        if let Some(basic) = auth.get_http_basic(host) {
            let credentials = base64_encode(&format!("{}:{}", basic.username, basic.password));
            if let Ok(value) = HeaderValue::from_str(&format!("Basic {credentials}")) {
                headers.insert(AUTHORIZATION, value);
                return;
            }
        }

        // Check Bearer token
        if let Some(token) = auth.get_bearer(host) {
            if let Ok(value) = HeaderValue::from_str(&format!("Bearer {token}")) {
                headers.insert(AUTHORIZATION, value);
                return;
            }
        }

        // Check GitHub OAuth
        if host.contains("github.com") || host.contains("api.github.com") {
            if let Some(token) = auth.get_github_oauth("github.com") {
                if let Ok(value) = HeaderValue::from_str(&format!("token {token}")) {
                    headers.insert(AUTHORIZATION, value);
                    return;
                }
            }
        }

        // Check GitLab token
        if host.contains("gitlab.com") || host.contains("gitlab") {
            if let Some(token) = auth.get_gitlab_token(host) {
                if let Ok(value) = HeaderValue::from_str(token) {
                    headers.insert("PRIVATE-TOKEN", value);
                }
            }
        }
    }

    async fn check_response(&self, response: Response) -> Result<Response> {
        let status = response.status();

        if status.is_success() || status == StatusCode::PARTIAL_CONTENT {
            return Ok(response);
        }

        let url = response.url().to_string();

        match status {
            StatusCode::NOT_FOUND => Err(DownloadError::NotFound { url }),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                Err(DownloadError::Authentication {
                    domain: response.url().host_str().unwrap_or("unknown").to_string(),
                    message: format!("HTTP {}", status.as_u16()),
                })
            }
            StatusCode::TOO_MANY_REQUESTS => {
                let retry_after = response
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok())
                    .map(Duration::from_secs);

                Err(DownloadError::RateLimited {
                    domain: response.url().host_str().unwrap_or("unknown").to_string(),
                    retry_after,
                })
            }
            _ if status.is_server_error() => Err(DownloadError::ServerError {
                status: status.as_u16(),
                message: response.text().await.unwrap_or_else(|_| status.to_string()),
            }),
            _ => Err(DownloadError::network_with_status(
                format!("HTTP {status}"),
                status.as_u16(),
            )),
        }
    }
}

/// Simple base64 encoding for auth credentials.
fn base64_encode(input: &str) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let bytes = input.as_bytes();
    let mut result = String::with_capacity(bytes.len().div_ceil(3) * 4);

    for chunk in bytes.chunks(3) {
        let mut n = 0u32;
        for (i, &b) in chunk.iter().enumerate() {
            n |= u32::from(b) << (16 - i * 8);
        }

        result.push(CHARS[(n >> 18 & 0x3F) as usize] as char);
        result.push(CHARS[(n >> 12 & 0x3F) as usize] as char);

        if chunk.len() > 1 {
            result.push(CHARS[(n >> 6 & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }

        if chunk.len() > 2 {
            result.push(CHARS[(n & 0x3F) as usize] as char);
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
    fn base64_encode_test() {
        assert_eq!(base64_encode("hello"), "aGVsbG8=");
        assert_eq!(base64_encode("user:pass"), "dXNlcjpwYXNz");
    }

    #[tokio::test]
    async fn client_creation() {
        let client = HttpClient::with_defaults();
        assert!(client.is_ok());
    }

    #[test]
    fn client_debug() {
        let client = HttpClient::with_defaults().unwrap();
        let debug = format!("{client:?}");
        assert!(debug.contains("HttpClient"));
    }
}
