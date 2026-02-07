//! Installer-paths support for custom package installation locations.
//!
//! This module implements the `extra.installer-paths` configuration from composer.json,
//! which allows packages to be installed to custom locations based on their type or name.
//!
//! # Example Configuration
//!
//! ```json
//! {
//!     "extra": {
//!         "installer-paths": {
//!             "wp-content/plugins/{$name}/": ["type:wordpress-plugin"],
//!             "wp-content/themes/{$name}/": ["type:wordpress-theme"],
//!             "modules/{$name}/": ["type:drupal-module"],
//!             "custom/location/": ["vendor/package-name"]
//!         }
//!     }
//! }
//! ```

use sonic_rs::{JsonContainerTrait, JsonValueTrait, Value};
use std::path::{Path, PathBuf};
use tracing::debug;

/// Installer paths configuration parsed from composer.json.
#[derive(Debug, Clone, Default)]
pub struct InstallerPaths {
    /// Map of path template -> list of matchers
    paths: Vec<(String, Vec<PathMatcher>)>,
}

/// A matcher for determining if a package should be installed to a custom path.
#[derive(Debug, Clone)]
enum PathMatcher {
    /// Match by package type (e.g., "type:wordpress-plugin")
    Type(String),
    /// Match by exact package name (e.g., "vendor/package")
    Package(String),
    /// Match by vendor prefix (e.g., "vendor/*")
    Vendor(String),
}

impl InstallerPaths {
    /// Parse installer-paths from composer.json.
    pub fn from_composer(composer: &Value) -> Self {
        let mut paths = Vec::new();

        if let Some(extra) = composer.get("extra").and_then(|e| e.as_object())
            && let Some(installer_paths) = extra.get(&"installer-paths").and_then(|p| p.as_object())
            {
                for (path_template, matchers) in installer_paths {
                    let mut path_matchers = Vec::new();

                    if let Some(arr) = matchers.as_array() {
                        for matcher in arr {
                            if let Some(m) = matcher.as_str()
                                && let Some(matcher) = parse_matcher(m) {
                                    path_matchers.push(matcher);
                                }
                        }
                    }

                    if !path_matchers.is_empty() {
                        paths.push((path_template.to_string(), path_matchers));
                    }
                }
            }

        debug!(
            "Parsed {} installer-paths rules",
            paths.iter().map(|(_, m)| m.len()).sum::<usize>()
        );

        Self { paths }
    }

    /// Get the installation path for a package.
    ///
    /// Returns `None` if the package should be installed to the default vendor location.
    pub fn get_path(
        &self,
        base_dir: &Path,
        package_name: &str,
        package_type: Option<&str>,
    ) -> Option<PathBuf> {
        for (path_template, matchers) in &self.paths {
            for matcher in matchers {
                if matcher.matches(package_name, package_type) {
                    let resolved = resolve_path_template(path_template, package_name);
                    let full_path = base_dir.join(&resolved);
                    debug!(
                        package = %package_name,
                        path = %full_path.display(),
                        "Using custom installer-path"
                    );
                    return Some(full_path);
                }
            }
        }
        None
    }

    /// Check if any installer-paths are configured.
    #[must_use]
    #[allow(dead_code)] // Used in tests
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }
}

impl PathMatcher {
    /// Check if this matcher matches the given package.
    fn matches(&self, package_name: &str, package_type: Option<&str>) -> bool {
        match self {
            Self::Type(t) => package_type.is_some_and(|pt| pt == t),
            Self::Package(p) => package_name == p,
            Self::Vendor(v) => {
                package_name.starts_with(v)
                    && package_name
                        .get(v.len()..)
                        .is_some_and(|s| s.starts_with('/'))
            }
        }
    }
}

/// Parse a matcher string into a `PathMatcher`.
fn parse_matcher(s: &str) -> Option<PathMatcher> {
    if let Some(type_name) = s.strip_prefix("type:") {
        Some(PathMatcher::Type(type_name.to_string()))
    } else if s.ends_with("/*") {
        let vendor = s.strip_suffix("/*")?;
        Some(PathMatcher::Vendor(vendor.to_string()))
    } else if s.contains('/') {
        Some(PathMatcher::Package(s.to_string()))
    } else {
        None
    }
}

/// Resolve path template variables.
///
/// Supported variables:
/// - `{$name}` - Package name without vendor prefix
/// - `{$vendor}` - Vendor name
/// - `{$type}` - Package type (not commonly used in paths)
fn resolve_path_template(template: &str, package_name: &str) -> String {
    let (vendor, name) = package_name.split_once('/').unwrap_or(("", package_name));

    template
        .replace("{$name}", name)
        .replace("{$vendor}", vendor)
        .replace("{$package}", package_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_matcher_type() {
        let m = parse_matcher("type:wordpress-plugin").unwrap();
        assert!(matches!(m, PathMatcher::Type(t) if t == "wordpress-plugin"));
    }

    #[test]
    fn test_parse_matcher_package() {
        let m = parse_matcher("vendor/package").unwrap();
        assert!(matches!(m, PathMatcher::Package(p) if p == "vendor/package"));
    }

    #[test]
    fn test_parse_matcher_vendor() {
        let m = parse_matcher("wpackagist-plugin/*").unwrap();
        assert!(matches!(m, PathMatcher::Vendor(v) if v == "wpackagist-plugin"));
    }

    #[test]
    fn test_type_matcher_matches() {
        let m = PathMatcher::Type("wordpress-plugin".to_string());
        assert!(m.matches("vendor/my-plugin", Some("wordpress-plugin")));
        assert!(!m.matches("vendor/my-plugin", Some("library")));
        assert!(!m.matches("vendor/my-plugin", None));
    }

    #[test]
    fn test_package_matcher_matches() {
        let m = PathMatcher::Package("vendor/package".to_string());
        assert!(m.matches("vendor/package", None));
        assert!(!m.matches("vendor/other", None));
    }

    #[test]
    fn test_vendor_matcher_matches() {
        let m = PathMatcher::Vendor("wpackagist-plugin".to_string());
        assert!(m.matches("wpackagist-plugin/my-plugin", None));
        assert!(!m.matches("wpackagist-theme/my-theme", None));
        assert!(!m.matches("wpackagist-plugin", None)); // No slash after prefix
    }

    #[test]
    fn test_resolve_path_template() {
        assert_eq!(
            resolve_path_template("wp-content/plugins/{$name}/", "wpackagist-plugin/akismet"),
            "wp-content/plugins/akismet/"
        );
        assert_eq!(
            resolve_path_template("modules/{$vendor}/{$name}/", "drupal/views"),
            "modules/drupal/views/"
        );
    }

    #[test]
    fn test_installer_paths_from_composer() {
        let composer: Value = sonic_rs::json!({
            "extra": {
                "installer-paths": {
                    "wp-content/plugins/{$name}/": ["type:wordpress-plugin"],
                    "custom/": ["vendor/special-package"]
                }
            }
        });

        let paths = InstallerPaths::from_composer(&composer);
        assert!(!paths.is_empty());

        let base = Path::new("/project");

        // Should match wordpress-plugin type
        let path = paths.get_path(base, "wpackagist-plugin/akismet", Some("wordpress-plugin"));
        assert_eq!(
            path,
            Some(PathBuf::from("/project/wp-content/plugins/akismet/"))
        );

        // Should match specific package
        let path = paths.get_path(base, "vendor/special-package", Some("library"));
        assert_eq!(path, Some(PathBuf::from("/project/custom/")));

        // Should not match regular library
        let path = paths.get_path(base, "vendor/regular", Some("library"));
        assert!(path.is_none());
    }
}
