//! Random data generators for property-based testing and fuzz testing.

use rand::prelude::*;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

/// Generate a random vendor name.
#[must_use]
pub fn random_vendor_name() -> String {
    let mut rng = rand::thread_rng();
    let prefixes = ["acme", "vendor", "company", "org", "example", "test"];
    let prefix = prefixes.choose(&mut rng).unwrap();
    format!("{}{}", prefix, rng.gen_range(1..1000))
}

/// Generate a random package name.
#[must_use]
pub fn random_package_name() -> String {
    let mut rng = rand::thread_rng();
    let names = [
        "library",
        "framework",
        "toolkit",
        "helper",
        "utils",
        "core",
        "common",
        "base",
        "sdk",
        "client",
        "server",
        "api",
        "service",
        "module",
        "plugin",
        "extension",
        "adapter",
    ];
    let name = names.choose(&mut rng).unwrap();
    format!("{}{}", name, rng.gen_range(1..100))
}

/// Generate a random full package name (vendor/package).
#[must_use]
pub fn random_full_package_name() -> String {
    format!("{}/{}", random_vendor_name(), random_package_name())
}

/// Generate a random semantic version.
#[must_use]
pub fn random_semver() -> String {
    let mut rng = rand::thread_rng();
    format!(
        "{}.{}.{}",
        rng.gen_range(0..20),
        rng.gen_range(0..50),
        rng.gen_range(0..100)
    )
}

/// Generate a random version with optional pre-release.
#[must_use]
pub fn random_version_with_prerelease() -> String {
    let mut rng = rand::thread_rng();
    let version = random_semver();

    if rng.gen_bool(0.3) {
        let prerelease = ["alpha", "beta", "rc", "dev"].choose(&mut rng).unwrap();
        let num = rng.gen_range(1..10);
        format!("{version}-{prerelease}.{num}")
    } else {
        version
    }
}

/// Generate a random version constraint.
#[must_use]
pub fn random_constraint() -> String {
    let mut rng = rand::thread_rng();
    let major = rng.gen_range(1..10);
    let minor = rng.gen_range(0..20);
    let patch = rng.gen_range(0..50);

    match rng.gen_range(0..10) {
        0 => format!("^{major}.{minor}"),
        1 => format!("^{major}.{minor}.{patch}"),
        2 => format!("~{major}.{minor}"),
        3 => format!("~{major}.{minor}.{patch}"),
        4 => format!(">={major}.{minor}"),
        5 => format!(">={major}.{minor} <{}.0", major + 1),
        6 => format!("{major}.{minor}.*"),
        7 => format!("{major}.*"),
        8 => "*".to_string(),
        _ => format!("{major}.{minor}.{patch}"),
    }
}

/// Generate a random complex constraint (with OR).
#[must_use]
pub fn random_complex_constraint() -> String {
    let mut rng = rand::thread_rng();

    if rng.gen_bool(0.3) {
        format!("{} || {}", random_constraint(), random_constraint())
    } else {
        random_constraint()
    }
}

/// Generate a random stability flag.
#[must_use]
pub fn random_stability() -> &'static str {
    let mut rng = rand::thread_rng();
    let stabilities = ["stable", "RC", "beta", "alpha", "dev"];
    stabilities.choose(&mut rng).unwrap()
}

/// Configuration for dependency graph generation.
#[derive(Debug, Clone)]
pub struct GraphConfig {
    /// Number of packages.
    pub package_count: usize,
    /// Average dependencies per package.
    pub avg_deps: usize,
    /// Maximum dependency depth.
    pub max_depth: usize,
    /// Allow circular dependencies (for testing detection).
    pub allow_cycles: bool,
    /// Probability of conflicting constraints.
    pub conflict_probability: f64,
}

impl Default for GraphConfig {
    fn default() -> Self {
        Self {
            package_count: 50,
            avg_deps: 3,
            max_depth: 10,
            allow_cycles: false,
            conflict_probability: 0.0,
        }
    }
}

impl GraphConfig {
    /// Create a simple graph configuration.
    #[must_use]
    pub fn simple() -> Self {
        Self {
            package_count: 10,
            avg_deps: 2,
            max_depth: 3,
            ..Default::default()
        }
    }

    /// Create a complex graph configuration.
    #[must_use]
    pub fn complex() -> Self {
        Self {
            package_count: 100,
            avg_deps: 5,
            max_depth: 15,
            ..Default::default()
        }
    }

    /// Create a stress test configuration.
    #[must_use]
    pub fn stress() -> Self {
        Self {
            package_count: 1000,
            avg_deps: 8,
            max_depth: 50,
            ..Default::default()
        }
    }

    /// Enable cycles for testing cycle detection.
    #[must_use]
    pub fn with_cycles(mut self) -> Self {
        self.allow_cycles = true;
        self
    }

    /// Add conflict probability.
    #[must_use]
    pub fn with_conflicts(mut self, probability: f64) -> Self {
        self.conflict_probability = probability;
        self
    }
}

/// A generated package for testing.
#[derive(Debug, Clone)]
pub struct GeneratedPackage {
    /// Package name (vendor/package).
    pub name: String,
    /// Available versions.
    pub versions: Vec<String>,
    /// Dependencies for each version: version -> [(`dep_name`, constraint)].
    pub dependencies: HashMap<String, Vec<(String, String)>>,
}

/// A generated dependency graph for testing resolution.
#[derive(Debug, Clone)]
pub struct GeneratedGraph {
    /// All packages in the graph.
    pub packages: Vec<GeneratedPackage>,
    /// Root dependencies (for composer.json require).
    pub root_deps: Vec<(String, String)>,
    /// Expected resolution (if deterministic).
    pub expected_resolution: Option<HashMap<String, String>>,
}

impl GeneratedGraph {
    /// Generate a random dependency graph.
    #[must_use]
    pub fn generate(config: &GraphConfig) -> Self {
        let mut rng = rand::thread_rng();
        let mut packages = Vec::with_capacity(config.package_count);
        let mut package_names: Vec<String> = Vec::new();

        // Generate packages
        for i in 0..config.package_count {
            let name = format!("generated{}/pkg{}", i / 10, i);
            let version_count = rng.gen_range(1..=5);
            let versions: Vec<String> = (0..version_count)
                .map(|v| format!("{}.{}.0", v + 1, rng.gen_range(0..10)))
                .collect();

            packages.push(GeneratedPackage {
                name: name.clone(),
                versions,
                dependencies: HashMap::new(),
            });
            package_names.push(name);
        }

        // Generate dependencies (ensuring acyclic if configured)
        for (i, package) in packages.iter_mut().enumerate().take(config.package_count) {
            let max_deps = (config.avg_deps * 2).min(i);
            let dep_count = if i == 0 {
                0
            } else {
                rng.gen_range(0..=max_deps)
            };

            let mut deps_for_pkg: HashMap<String, Vec<(String, String)>> = HashMap::new();

            for version in &package.versions {
                let mut version_deps = Vec::new();
                let mut used_deps = HashSet::new();

                for _ in 0..dep_count {
                    // Only depend on packages with lower index (ensures no cycles)
                    let dep_idx = if config.allow_cycles {
                        rng.gen_range(0..config.package_count)
                    } else {
                        rng.gen_range(0..i)
                    };

                    if used_deps.insert(dep_idx) {
                        let dep_name = &package_names[dep_idx];
                        let constraint = if rng.gen_bool(config.conflict_probability) {
                            // Generate potentially conflicting constraint
                            "<1.0".to_string()
                        } else {
                            random_constraint()
                        };
                        version_deps.push((dep_name.clone(), constraint));
                    }
                }

                deps_for_pkg.insert(version.clone(), version_deps);
            }

            package.dependencies = deps_for_pkg;
        }

        // Generate root dependencies (last few packages)
        let root_count = (config.package_count / 10).clamp(1, 10);
        let root_deps: Vec<(String, String)> = packages
            .iter()
            .rev()
            .take(root_count)
            .map(|pkg| (pkg.name.clone(), "^1.0".to_string()))
            .collect();

        Self {
            packages,
            root_deps,
            expected_resolution: None,
        }
    }

    /// Convert to Packagist-like package responses.
    #[must_use]
    pub fn to_package_responses(&self) -> HashMap<String, Value> {
        self.packages
            .iter()
            .map(|pkg| {
                let mut versions = serde_json::Map::new();

                for version in &pkg.versions {
                    let deps = pkg.dependencies.get(version).cloned().unwrap_or_default();
                    let require: serde_json::Map<String, Value> = deps
                        .into_iter()
                        .map(|(name, constraint)| (name, json!(constraint)))
                        .collect();

                    versions.insert(
                        version.clone(),
                        json!({
                            "name": pkg.name,
                            "version": version,
                            "require": require,
                            "type": "library"
                        }),
                    );
                }

                let response = json!({
                    "package": {
                        "name": pkg.name,
                        "versions": versions
                    }
                });

                (pkg.name.clone(), response)
            })
            .collect()
    }

    /// Generate composer.json for the root project.
    #[must_use]
    pub fn to_composer_json(&self) -> Value {
        let require: serde_json::Map<String, Value> = self
            .root_deps
            .iter()
            .map(|(name, constraint)| (name.clone(), json!(constraint)))
            .collect();

        json!({
            "name": "test/generated-project",
            "type": "project",
            "require": require
        })
    }
}

/// Generate random PHP class name.
#[must_use]
pub fn random_class_name() -> String {
    let mut rng = rand::thread_rng();
    let prefixes = [
        "Abstract", "Base", "Default", "Custom", "Simple", "Advanced", "Core", "",
    ];
    let names = [
        "Controller",
        "Service",
        "Repository",
        "Factory",
        "Handler",
        "Manager",
        "Provider",
        "Adapter",
        "Listener",
        "Command",
        "Query",
        "Validator",
        "Transformer",
    ];

    let prefix = prefixes.choose(&mut rng).unwrap();
    let name = names.choose(&mut rng).unwrap();
    format!("{prefix}{name}")
}

/// Generate random PHP namespace.
#[must_use]
pub fn random_namespace() -> String {
    let mut rng = rand::thread_rng();
    let depth = rng.gen_range(1..=4);
    let parts: Vec<String> = (0..depth)
        .map(|_| {
            let names = [
                "App",
                "Core",
                "Domain",
                "Infrastructure",
                "Application",
                "Http",
                "Console",
                "Service",
                "Model",
                "Repository",
                "Event",
                "Command",
            ];
            (*names.choose(&mut rng).unwrap()).to_string()
        })
        .collect();

    parts.join("\\")
}

/// Generate a random autoload configuration.
#[must_use]
pub fn random_autoload_config() -> Value {
    let mut rng = rand::thread_rng();
    let mut config = serde_json::Map::new();

    // PSR-4
    if rng.gen_bool(0.9) {
        let mut psr4 = serde_json::Map::new();
        let ns_count = rng.gen_range(1..=3);
        for _ in 0..ns_count {
            let ns = format!("{}\\", random_namespace());
            psr4.insert(ns, json!("src/"));
        }
        config.insert("psr-4".to_string(), json!(psr4));
    }

    // Classmap
    if rng.gen_bool(0.3) {
        config.insert("classmap".to_string(), json!(["lib/"]));
    }

    // Files
    if rng.gen_bool(0.2) {
        config.insert("files".to_string(), json!(["src/helpers.php"]));
    }

    json!(config)
}

/// Generate random SHA-256 hash.
#[must_use]
pub fn random_sha256() -> String {
    let mut rng = rand::thread_rng();
    let bytes: Vec<u8> = (0..32).map(|_| rng.r#gen::<u8>()).collect();
    hex::encode(bytes)
}

/// Generate random git reference.
#[must_use]
pub fn random_git_ref() -> String {
    let mut rng = rand::thread_rng();
    let bytes: Vec<u8> = (0..20).map(|_| rng.r#gen::<u8>()).collect();
    hex::encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_random_package_name() {
        let name = random_full_package_name();
        assert!(name.contains('/'));
    }

    #[test]
    fn test_random_semver() {
        let version = random_semver();
        assert_eq!(version.split('.').count(), 3);
    }

    #[test]
    fn test_random_constraint() {
        let constraint = random_constraint();
        assert!(!constraint.is_empty());
    }

    #[test]
    fn test_generate_graph() {
        let config = GraphConfig::simple();
        let graph = GeneratedGraph::generate(&config);

        assert_eq!(graph.packages.len(), config.package_count);
        assert!(!graph.root_deps.is_empty());
    }

    #[test]
    fn test_graph_to_composer_json() {
        let config = GraphConfig::simple();
        let graph = GeneratedGraph::generate(&config);
        let composer_json = graph.to_composer_json();

        assert!(composer_json["require"].is_object());
    }

    #[test]
    fn test_random_autoload_config() {
        let config = random_autoload_config();
        assert!(config.is_object());
    }
}
