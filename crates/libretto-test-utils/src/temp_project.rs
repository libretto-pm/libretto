//! Temporary project creation and management for integration tests.
//!
//! This module provides utilities to create isolated test projects
//! with composer.json, vendor directories, and package structures.

use anyhow::{Context, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};
use tempfile::{TempDir, tempdir};
use tokio::fs;

use crate::fixtures::Fixtures;

/// A temporary project directory for testing.
///
/// The project is automatically cleaned up when this struct is dropped.
#[derive(Debug)]
pub struct TempProject {
    /// The temporary directory containing the project.
    dir: TempDir,
    /// Path to composer.json.
    composer_json_path: PathBuf,
    /// Path to composer.lock (if exists).
    composer_lock_path: PathBuf,
    /// Path to vendor directory.
    vendor_path: PathBuf,
}

impl TempProject {
    /// Create a new temporary project builder.
    #[must_use]
    pub fn new() -> TempProjectBuilder {
        TempProjectBuilder::default()
    }

    /// Get the root directory of the project.
    #[must_use]
    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// Get the path to composer.json.
    #[must_use]
    pub fn composer_json_path(&self) -> &Path {
        &self.composer_json_path
    }

    /// Get the path to composer.lock.
    #[must_use]
    pub fn composer_lock_path(&self) -> &Path {
        &self.composer_lock_path
    }

    /// Get the path to vendor directory.
    #[must_use]
    pub fn vendor_path(&self) -> &Path {
        &self.vendor_path
    }

    /// Check if composer.lock exists.
    pub async fn has_lock_file(&self) -> bool {
        fs::metadata(&self.composer_lock_path).await.is_ok()
    }

    /// Check if vendor directory exists.
    pub async fn has_vendor(&self) -> bool {
        fs::metadata(&self.vendor_path).await.is_ok()
    }

    /// Read composer.json content.
    pub async fn read_composer_json(&self) -> Result<Value> {
        let content = fs::read_to_string(&self.composer_json_path)
            .await
            .context("Failed to read composer.json")?;
        serde_json::from_str(&content).context("Failed to parse composer.json")
    }

    /// Read composer.lock content.
    pub async fn read_composer_lock(&self) -> Result<Value> {
        let content = fs::read_to_string(&self.composer_lock_path)
            .await
            .context("Failed to read composer.lock")?;
        serde_json::from_str(&content).context("Failed to parse composer.lock")
    }

    /// Write content to composer.json.
    pub async fn write_composer_json(&self, content: &Value) -> Result<()> {
        let json = serde_json::to_string_pretty(content)?;
        fs::write(&self.composer_json_path, json)
            .await
            .context("Failed to write composer.json")
    }

    /// Write content to composer.lock.
    pub async fn write_composer_lock(&self, content: &Value) -> Result<()> {
        let json = serde_json::to_string_pretty(content)?;
        fs::write(&self.composer_lock_path, json)
            .await
            .context("Failed to write composer.lock")
    }

    /// Create a file in the project directory.
    pub async fn create_file(&self, relative_path: &str, content: &str) -> Result<PathBuf> {
        let path = self.dir.path().join(relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::write(&path, content).await?;
        Ok(path)
    }

    /// Create a directory in the project.
    pub async fn create_dir(&self, relative_path: &str) -> Result<PathBuf> {
        let path = self.dir.path().join(relative_path);
        fs::create_dir_all(&path).await?;
        Ok(path)
    }

    /// Check if a file exists in the project.
    pub async fn file_exists(&self, relative_path: &str) -> bool {
        let path = self.dir.path().join(relative_path);
        fs::metadata(&path).await.is_ok()
    }

    /// Read a file from the project.
    pub async fn read_file(&self, relative_path: &str) -> Result<String> {
        let path = self.dir.path().join(relative_path);
        fs::read_to_string(&path)
            .await
            .with_context(|| format!("Failed to read file: {relative_path}"))
    }

    /// List files in a directory.
    pub async fn list_files(&self, relative_path: &str) -> Result<Vec<PathBuf>> {
        let path = self.dir.path().join(relative_path);
        let mut entries = Vec::new();
        let mut read_dir = fs::read_dir(&path).await?;

        while let Some(entry) = read_dir.next_entry().await? {
            entries.push(entry.path());
        }

        Ok(entries)
    }

    /// Get the path to the autoloader file.
    #[must_use]
    pub fn autoload_path(&self) -> PathBuf {
        self.vendor_path.join("autoload.php")
    }

    /// Check if the autoloader exists.
    pub async fn has_autoloader(&self) -> bool {
        fs::metadata(self.autoload_path()).await.is_ok()
    }

    /// Create a vendor package structure.
    pub async fn create_vendor_package(
        &self,
        vendor: &str,
        package: &str,
        files: &[(&str, &str)],
    ) -> Result<PathBuf> {
        let package_path = self.vendor_path.join(vendor).join(package);
        fs::create_dir_all(&package_path).await?;

        for (file_name, content) in files {
            let file_path = package_path.join(file_name);
            if let Some(parent) = file_path.parent() {
                fs::create_dir_all(parent).await?;
            }
            fs::write(&file_path, content).await?;
        }

        Ok(package_path)
    }

    /// Create a PSR-4 source directory structure.
    pub async fn create_psr4_structure(
        &self,
        namespace_prefix: &str,
        base_path: &str,
        classes: &[(&str, &str)],
    ) -> Result<()> {
        let src_path = self.dir.path().join(base_path);
        fs::create_dir_all(&src_path).await?;

        for (class_name, content) in classes {
            // Convert namespace to path
            let relative_path = class_name.replace('\\', "/");
            let file_path = src_path.join(format!("{relative_path}.php"));

            if let Some(parent) = file_path.parent() {
                fs::create_dir_all(parent).await?;
            }

            let full_content = if content.starts_with("<?php") {
                content.to_string()
            } else {
                let full_namespace = if class_name.contains('\\') {
                    let parts: Vec<&str> = class_name.rsplitn(2, '\\').collect();
                    format!("{}\\{}", namespace_prefix, parts[1])
                } else {
                    namespace_prefix.to_string()
                };
                let simple_name = class_name.rsplit('\\').next().unwrap_or(class_name);
                Fixtures::php_class_content(&full_namespace, simple_name)
            };

            fs::write(&file_path, full_content).await?;
        }

        Ok(())
    }

    /// Keep the temporary directory (prevent cleanup on drop).
    /// Returns the path to the directory.
    #[must_use]
    pub fn persist(self) -> PathBuf {
        let path = self.dir.path().to_path_buf();
        std::mem::forget(self);
        path
    }
}

impl Default for TempProject {
    fn default() -> Self {
        futures::executor::block_on(async {
            TempProjectBuilder::default()
                .build()
                .await
                .expect("Failed to create default TempProject")
        })
    }
}

/// Builder for creating temporary projects.
#[derive(Debug, Default)]
pub struct TempProjectBuilder {
    composer_json: Option<Value>,
    composer_lock: Option<Value>,
    create_vendor: bool,
    create_src: bool,
    files: Vec<(String, String)>,
    php_classes: Vec<(String, String, String)>, // (namespace, class_name, content)
}

impl TempProjectBuilder {
    /// Set the composer.json content.
    #[must_use]
    pub fn with_composer_json(mut self, content: Value) -> Self {
        self.composer_json = Some(content);
        self
    }

    /// Set the composer.lock content.
    #[must_use]
    pub fn with_composer_lock(mut self, content: Value) -> Self {
        self.composer_lock = Some(content);
        self
    }

    /// Create an empty vendor directory.
    #[must_use]
    pub fn with_vendor(mut self) -> Self {
        self.create_vendor = true;
        self
    }

    /// Create a src directory.
    #[must_use]
    pub fn with_src(mut self) -> Self {
        self.create_src = true;
        self
    }

    /// Add a file to be created.
    #[must_use]
    pub fn with_file(mut self, path: impl Into<String>, content: impl Into<String>) -> Self {
        self.files.push((path.into(), content.into()));
        self
    }

    /// Add a PHP class to be created.
    #[must_use]
    pub fn with_php_class(
        mut self,
        namespace: impl Into<String>,
        class_name: impl Into<String>,
    ) -> Self {
        let ns = namespace.into();
        let name = class_name.into();
        let content = Fixtures::php_class_content(&ns, &name);
        self.php_classes.push((ns, name, content));
        self
    }

    /// Add a PHP class with custom content.
    #[must_use]
    pub fn with_php_class_content(
        mut self,
        namespace: impl Into<String>,
        class_name: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        self.php_classes
            .push((namespace.into(), class_name.into(), content.into()));
        self
    }

    /// Use the Laravel fixture.
    #[must_use]
    pub fn laravel_like(mut self) -> Self {
        self.composer_json = Some(Fixtures::laravel_composer_json());
        self.create_src = true;
        self
    }

    /// Use the Symfony fixture.
    #[must_use]
    pub fn symfony_like(mut self) -> Self {
        self.composer_json = Some(Fixtures::symfony_composer_json());
        self.create_src = true;
        self
    }

    /// Use a simple project fixture.
    #[must_use]
    pub fn simple(mut self) -> Self {
        self.composer_json = Some(Fixtures::simple_composer_json());
        self.create_src = true;
        self
    }

    /// Build the temporary project.
    pub async fn build(self) -> Result<TempProject> {
        let dir = tempdir().context("Failed to create temp directory")?;
        let root = dir.path();

        // Create composer.json
        let composer_json_path = root.join("composer.json");
        let composer_json = self
            .composer_json
            .unwrap_or_else(Fixtures::empty_composer_json);
        let json_content = serde_json::to_string_pretty(&composer_json)?;
        fs::write(&composer_json_path, json_content)
            .await
            .context("Failed to write composer.json")?;

        // Create composer.lock if provided
        let composer_lock_path = root.join("composer.lock");
        if let Some(lock) = self.composer_lock {
            let lock_content = serde_json::to_string_pretty(&lock)?;
            fs::write(&composer_lock_path, lock_content)
                .await
                .context("Failed to write composer.lock")?;
        }

        // Create vendor directory if requested
        let vendor_path = root.join("vendor");
        if self.create_vendor {
            fs::create_dir_all(&vendor_path)
                .await
                .context("Failed to create vendor directory")?;
        }

        // Create src directory if requested
        if self.create_src {
            fs::create_dir_all(root.join("src"))
                .await
                .context("Failed to create src directory")?;
        }

        // Create additional files
        for (path, content) in self.files {
            let file_path = root.join(&path);
            if let Some(parent) = file_path.parent() {
                fs::create_dir_all(parent).await?;
            }
            fs::write(&file_path, content).await?;
        }

        // Create PHP classes
        for (namespace, class_name, content) in self.php_classes {
            // Convert namespace to path
            let ns_path = namespace.replace('\\', "/");
            let file_path = root
                .join("src")
                .join(&ns_path)
                .join(format!("{class_name}.php"));
            if let Some(parent) = file_path.parent() {
                fs::create_dir_all(parent).await?;
            }
            fs::write(&file_path, content).await?;
        }

        Ok(TempProject {
            dir,
            composer_json_path,
            composer_lock_path,
            vendor_path,
        })
    }
}

/// Create multiple temporary projects for batch testing.
pub async fn create_test_projects(count: usize) -> Result<Vec<TempProject>> {
    let mut projects = Vec::with_capacity(count);
    for i in 0..count {
        let project = TempProject::new()
            .with_composer_json(serde_json::json!({
                "name": format!("test/project-{}", i),
                "type": "project",
                "require": {}
            }))
            .build()
            .await?;
        projects.push(project);
    }
    Ok(projects)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create_temp_project() {
        let project = TempProject::new().build().await.unwrap();

        assert!(project.path().exists());
        assert!(project.composer_json_path().exists());
    }

    #[tokio::test]
    async fn test_create_with_composer_lock() {
        let project = TempProject::new()
            .with_composer_lock(Fixtures::simple_composer_lock())
            .build()
            .await
            .unwrap();

        assert!(project.has_lock_file().await);
    }

    #[tokio::test]
    async fn test_create_vendor_package() {
        let project = TempProject::new().with_vendor().build().await.unwrap();

        project
            .create_vendor_package(
                "monolog",
                "monolog",
                &[
                    ("composer.json", r#"{"name": "monolog/monolog"}"#),
                    ("src/Logger.php", "<?php class Logger {}"),
                ],
            )
            .await
            .unwrap();

        assert!(
            project
                .file_exists("vendor/monolog/monolog/composer.json")
                .await
        );
        assert!(
            project
                .file_exists("vendor/monolog/monolog/src/Logger.php")
                .await
        );
    }

    #[tokio::test]
    async fn test_laravel_like_project() {
        let project = TempProject::new().laravel_like().build().await.unwrap();

        let composer_json = project.read_composer_json().await.unwrap();
        assert_eq!(composer_json["name"], "laravel/laravel");
    }

    #[tokio::test]
    async fn test_create_file() {
        let project = TempProject::new().build().await.unwrap();

        project
            .create_file("src/Models/User.php", "<?php class User {}")
            .await
            .unwrap();

        assert!(project.file_exists("src/Models/User.php").await);
        let content = project.read_file("src/Models/User.php").await.unwrap();
        assert!(content.contains("class User"));
    }
}
