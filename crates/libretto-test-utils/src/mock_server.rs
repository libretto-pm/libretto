//! HTTP mock server utilities for testing Packagist and other API interactions.

use crate::fixtures::Fixtures;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Mock Packagist server for testing package resolution.
#[derive(Debug)]
pub struct MockPackagist {
    server: MockServer,
    packages: Arc<RwLock<HashMap<String, Value>>>,
}

impl MockPackagist {
    /// Start a new mock Packagist server.
    pub async fn start() -> Self {
        let server = MockServer::start().await;
        let packages = Arc::new(RwLock::new(HashMap::new()));

        Self { server, packages }
    }

    /// Get the base URL of the mock server.
    #[must_use]
    pub fn url(&self) -> String {
        self.server.uri()
    }

    /// Register a package with the mock server.
    pub async fn register_package(&self, name: &str, versions: Vec<(&str, Value)>) {
        let parts: Vec<&str> = name.split('/').collect();
        let vendor = parts.first().unwrap_or(&"vendor");
        let package_name = parts.get(1).unwrap_or(&"package");

        let mut version_map = serde_json::Map::new();
        for (version, metadata) in versions {
            let mut full_metadata = metadata.clone();
            if let Value::Object(ref mut obj) = full_metadata {
                obj.insert("name".to_string(), json!(name));
                obj.insert("version".to_string(), json!(version));
            }
            version_map.insert(version.to_string(), full_metadata);
        }

        let response = json!({
            "package": {
                "name": name,
                "description": format!("{} package", package_name),
                "versions": version_map,
                "type": "library",
                "repository": format!("https://github.com/{}/{}", vendor, package_name)
            }
        });

        // Store package data
        {
            let mut packages = self.packages.write().await;
            packages.insert(name.to_string(), response.clone());
        }

        // Register the mock
        Mock::given(method("GET"))
            .and(path(format!("/packages/{name}.json")))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response))
            .mount(&self.server)
            .await;
    }

    /// Register a simple package with default metadata.
    pub async fn register_simple_package(&self, name: &str, versions: Vec<&str>) {
        let version_data: Vec<(&str, Value)> = versions
            .into_iter()
            .map(|v| {
                (
                    v,
                    json!({
                        "require": { "php": ">=7.4" },
                        "type": "library",
                        "autoload": {
                            "psr-4": { "Vendor\\Package\\": "src/" }
                        }
                    }),
                )
            })
            .collect();

        self.register_package(name, version_data).await;
    }

    /// Register a package with dependencies.
    pub async fn register_package_with_deps(
        &self,
        name: &str,
        version: &str,
        deps: HashMap<&str, &str>,
    ) {
        let require: serde_json::Map<String, Value> = deps
            .into_iter()
            .map(|(k, v)| (k.to_string(), json!(v)))
            .collect();

        self.register_package(
            name,
            vec![(
                version,
                json!({
                    "require": require,
                    "type": "library"
                }),
            )],
        )
        .await;
    }

    /// Register the packages.json root endpoint.
    pub async fn register_root(&self) {
        let packages = self.packages.read().await;
        let package_names: Vec<String> = packages.keys().cloned().collect();

        let includes: serde_json::Map<String, Value> = package_names
            .iter()
            .map(|name| {
                (
                    format!("p2/{name}.json"),
                    json!({ "sha256": format!("{:064x}", name.len()) }),
                )
            })
            .collect();

        let response = json!({
            "packages": {},
            "includes": includes,
            "metadata-url": format!("{}/p2/%package%.json", self.url()),
            "providers-url": format!("{}/p/%package%$%hash%.json", self.url()),
            "search": format!("{}/search.json?q=%query%", self.url()),
            "list": format!("{}/packages/list.json", self.url())
        });

        Mock::given(method("GET"))
            .and(path("/packages.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response))
            .mount(&self.server)
            .await;
    }

    /// Register search endpoint.
    pub async fn register_search(&self, results: Vec<(&str, &str)>) {
        let search_results: Vec<Value> = results
            .into_iter()
            .map(|(name, description)| {
                json!({
                    "name": name,
                    "description": description,
                    "url": format!("https://packagist.org/packages/{}", name),
                    "repository": format!("https://github.com/{}", name),
                    "downloads": 10000,
                    "favers": 100
                })
            })
            .collect();

        Mock::given(method("GET"))
            .and(path("/search.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "results": search_results,
                "total": search_results.len()
            })))
            .mount(&self.server)
            .await;
    }

    /// Register 404 response for non-existent package.
    pub async fn register_not_found(&self, name: &str) {
        Mock::given(method("GET"))
            .and(path(format!("/packages/{name}.json")))
            .respond_with(ResponseTemplate::new(404).set_body_json(json!({
                "status": "error",
                "message": "Package not found"
            })))
            .mount(&self.server)
            .await;
    }

    /// Register rate limit response.
    pub async fn register_rate_limit(&self, name: &str) {
        Mock::given(method("GET"))
            .and(path(format!("/packages/{name}.json")))
            .respond_with(
                ResponseTemplate::new(429)
                    .set_body_json(json!({
                        "status": "error",
                        "message": "Rate limit exceeded"
                    }))
                    .insert_header("Retry-After", "60"),
            )
            .mount(&self.server)
            .await;
    }

    /// Register security advisories endpoint.
    pub async fn register_advisories(&self, advisories: Value) {
        Mock::given(method("GET"))
            .and(path("/api/security-advisories"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&advisories))
            .mount(&self.server)
            .await;

        Mock::given(method("POST"))
            .and(path("/api/security-advisories"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&advisories))
            .mount(&self.server)
            .await;
    }

    /// Register default advisories.
    pub async fn register_default_advisories(&self) {
        self.register_advisories(Fixtures::security_advisory_response())
            .await;
    }

    /// Get received requests count.
    pub async fn received_requests(&self) -> usize {
        self.server
            .received_requests()
            .await
            .unwrap_or_default()
            .len()
    }

    /// Verify expected requests were made.
    pub async fn verify(&self) {
        // Verification happens automatically when mocks are set with expect()
    }
}

/// Mock GitHub API server for testing VCS operations.
#[derive(Debug)]
pub struct MockGitHub {
    server: MockServer,
}

impl MockGitHub {
    /// Start a new mock GitHub server.
    pub async fn start() -> Self {
        let server = MockServer::start().await;
        Self { server }
    }

    /// Get the base URL.
    #[must_use]
    pub fn url(&self) -> String {
        self.server.uri()
    }

    /// Register a repository.
    pub async fn register_repo(&self, owner: &str, repo: &str) {
        Mock::given(method("GET"))
            .and(path(format!("/repos/{owner}/{repo}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": 12345,
                "name": repo,
                "full_name": format!("{}/{}", owner, repo),
                "private": false,
                "html_url": format!("https://github.com/{}/{}", owner, repo),
                "clone_url": format!("https://github.com/{}/{}.git", owner, repo),
                "ssh_url": format!("git@github.com:{}/{}.git", owner, repo),
                "default_branch": "main"
            })))
            .mount(&self.server)
            .await;
    }

    /// Register a release/tag.
    pub async fn register_release(&self, owner: &str, repo: &str, tag: &str, zipball_data: &[u8]) {
        // Tag endpoint
        Mock::given(method("GET"))
            .and(path(format!("/repos/{owner}/{repo}/git/refs/tags/{tag}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "ref": format!("refs/tags/{}", tag),
                "object": {
                    "sha": "abc123def456",
                    "type": "commit"
                }
            })))
            .mount(&self.server)
            .await;

        // Zipball endpoint
        Mock::given(method("GET"))
            .and(path(format!("/repos/{owner}/{repo}/zipball/{tag}")))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(zipball_data.to_vec())
                    .insert_header("Content-Type", "application/zip"),
            )
            .mount(&self.server)
            .await;
    }

    /// Register rate limit headers.
    pub async fn register_rate_limit_headers(&self) {
        // This would be added to all responses
    }
}

/// Mock download server for testing package downloads.
#[derive(Debug)]
pub struct MockDownloadServer {
    server: MockServer,
}

impl MockDownloadServer {
    /// Start a new mock download server.
    pub async fn start() -> Self {
        let server = MockServer::start().await;
        Self { server }
    }

    /// Get the base URL.
    #[must_use]
    pub fn url(&self) -> String {
        self.server.uri()
    }

    /// Register a downloadable zip file.
    pub async fn register_zip(&self, path_str: &str, content: &[u8]) {
        Mock::given(method("GET"))
            .and(path(path_str))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(content.to_vec())
                    .insert_header("Content-Type", "application/zip")
                    .insert_header("Content-Length", content.len().to_string()),
            )
            .mount(&self.server)
            .await;
    }

    /// Register a downloadable tar.gz file.
    pub async fn register_tarball(&self, path_str: &str, content: &[u8]) {
        Mock::given(method("GET"))
            .and(path(path_str))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(content.to_vec())
                    .insert_header("Content-Type", "application/gzip")
                    .insert_header("Content-Length", content.len().to_string()),
            )
            .mount(&self.server)
            .await;
    }

    /// Register a slow response (for timeout testing).
    pub async fn register_slow(&self, path_str: &str, delay_ms: u64) {
        Mock::given(method("GET"))
            .and(path(path_str))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(vec![0u8; 100])
                    .set_delay(std::time::Duration::from_millis(delay_ms)),
            )
            .mount(&self.server)
            .await;
    }

    /// Register an error response.
    pub async fn register_error(&self, path_str: &str, status: u16, message: &str) {
        Mock::given(method("GET"))
            .and(path(path_str))
            .respond_with(ResponseTemplate::new(status).set_body_json(json!({
                "error": message
            })))
            .mount(&self.server)
            .await;
    }

    /// Register a redirect.
    pub async fn register_redirect(&self, from: &str, to: &str) {
        Mock::given(method("GET"))
            .and(path(from))
            .respond_with(ResponseTemplate::new(302).insert_header("Location", to))
            .mount(&self.server)
            .await;
    }
}

/// Create a minimal valid ZIP file for testing.
#[must_use]
pub fn create_minimal_zip() -> Vec<u8> {
    // Minimal valid ZIP file (empty archive)
    vec![
        0x50, 0x4B, 0x05, 0x06, // End of central directory signature
        0x00, 0x00, // Number of this disk
        0x00, 0x00, // Disk with central directory
        0x00, 0x00, // Number of entries on this disk
        0x00, 0x00, // Total number of entries
        0x00, 0x00, 0x00, 0x00, // Size of central directory
        0x00, 0x00, 0x00, 0x00, // Offset to central directory
        0x00, 0x00, // Comment length
    ]
}

/// Create a ZIP file with a single file.
pub fn create_zip_with_file(filename: &str, content: &[u8]) -> Vec<u8> {
    use std::io::{Cursor, Write};

    let mut buf = Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file(filename, options).unwrap();
        zip.write_all(content).unwrap();
        zip.finish().unwrap();
    }
    buf.into_inner()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_packagist_register_package() {
        let mock = MockPackagist::start().await;

        mock.register_simple_package("vendor/package", vec!["1.0.0", "2.0.0"])
            .await;

        // Verify the mock responds correctly
        let client = reqwest::Client::new();
        let response = client
            .get(format!("{}/packages/vendor/package.json", mock.url()))
            .send()
            .await
            .unwrap();

        assert!(response.status().is_success());

        let json: Value = response.json().await.unwrap();
        assert_eq!(json["package"]["name"], "vendor/package");
    }

    #[tokio::test]
    async fn test_mock_packagist_not_found() {
        let mock = MockPackagist::start().await;

        mock.register_not_found("nonexistent/package").await;

        let client = reqwest::Client::new();
        let response = client
            .get(format!("{}/packages/nonexistent/package.json", mock.url()))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), 404);
    }

    #[test]
    fn test_create_minimal_zip() {
        let zip = create_minimal_zip();
        assert!(!zip.is_empty());
        // Verify ZIP signature
        assert_eq!(&zip[0..4], &[0x50, 0x4B, 0x05, 0x06]);
    }

    #[test]
    fn test_create_zip_with_file() {
        let content = b"Hello, World!";
        let zip = create_zip_with_file("test.txt", content);

        // Verify it's a valid ZIP
        let cursor = std::io::Cursor::new(zip);
        let mut archive = zip::ZipArchive::new(cursor).unwrap();
        assert_eq!(archive.len(), 1);

        let mut file = archive.by_name("test.txt").unwrap();
        let mut extracted = Vec::new();
        std::io::Read::read_to_end(&mut file, &mut extracted).unwrap();
        assert_eq!(extracted, content);
    }
}
