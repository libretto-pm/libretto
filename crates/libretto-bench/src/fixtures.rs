//! Test fixtures and data generators for benchmarks.

use rand::prelude::*;
use std::collections::BTreeMap;

/// Generate synthetic composer.json content.
#[must_use]
pub fn generate_composer_json(num_dependencies: usize) -> String {
    let mut rng = rand::rng();
    let mut require = BTreeMap::new();

    for i in 0..num_dependencies {
        let vendor = format!("vendor{}", i / 10);
        let package = format!("package{i}");
        let constraint = match rng.random_range(0..5) {
            0 => format!("^{}.0", rng.random_range(1..10)),
            1 => format!("~{}.{}", rng.random_range(1..10), rng.random_range(0..10)),
            2 => format!(">={}.0", rng.random_range(1..5)),
            3 => "*".to_string(),
            _ => format!(
                "{}.{}.{}",
                rng.random_range(1..10),
                rng.random_range(0..10),
                rng.random_range(0..20)
            ),
        };
        require.insert(format!("{vendor}/{package}"), constraint);
    }

    serde_json::json!({
        "name": "bench/project",
        "description": "Benchmark test project",
        "type": "project",
        "require": require,
        "require-dev": {},
        "autoload": {
            "psr-4": {
                "App\\": "src/"
            }
        }
    })
    .to_string()
}

/// Generate synthetic composer.lock content.
#[must_use]
pub fn generate_composer_lock(num_packages: usize) -> String {
    let mut packages = Vec::new();

    for i in 0..num_packages {
        let vendor = format!("vendor{}", i / 10);
        let package = format!("package{i}");
        packages.push(serde_json::json!({
            "name": format!("{vendor}/{package}"),
            "version": format!("{}.{}.{}", 1 + i / 100, (i / 10) % 10, i % 10),
            "source": {
                "type": "git",
                "url": format!("https://github.com/{vendor}/{package}.git"),
                "reference": format!("{:040x}", i)
            },
            "dist": {
                "type": "zip",
                "url": format!("https://api.github.com/repos/{vendor}/{package}/zipball/v1.0.{i}"),
                "reference": format!("{:040x}", i),
                "shasum": format!("{:064x}", i * 12345)
            },
            "require": {},
            "type": "library",
            "autoload": {
                "psr-4": {
                    format!("{}\\{}\\", vendor.replace("vendor", "Vendor"), package.replace("package", "Package")): "src/"
                }
            }
        }));
    }

    serde_json::json!({
        "packages": packages,
        "packages-dev": [],
        "aliases": [],
        "minimum-stability": "stable",
        "prefer-stable": true,
        "prefer-lowest": false,
        "platform": {},
        "platform-dev": {},
        "content-hash": "0123456789abcdef0123456789abcdef"
    })
    .to_string()
}

/// Generate synthetic PHP class file content.
#[must_use]
pub fn generate_php_class(namespace: &str, class_name: &str) -> String {
    format!(
        r"<?php

namespace {namespace};

use SomeOther\Dependency;

/**
 * {class_name} class for benchmarking.
 */
class {class_name}
{{
    private string $name;
    private int $value;

    public function __construct(string $name, int $value = 0)
    {{
        $this->name = $name;
        $this->value = $value;
    }}

    public function getName(): string
    {{
        return $this->name;
    }}

    public function getValue(): int
    {{
        return $this->value;
    }}

    public function setValue(int $value): self
    {{
        $this->value = $value;
        return $this;
    }}
}}
"
    )
}

/// Generate dependency graph for resolver benchmarks.
#[derive(Debug, Clone)]
pub struct DependencyGraph {
    /// Package names.
    pub packages: Vec<String>,
    /// Dependencies (`package_index` -> list of (`dep_index`, constraint)).
    pub dependencies: Vec<Vec<(usize, String)>>,
}

impl DependencyGraph {
    /// Generate a simple linear dependency chain.
    #[must_use]
    pub fn linear(size: usize) -> Self {
        let packages: Vec<String> = (0..size).map(|i| format!("pkg{i}")).collect();
        let mut dependencies = vec![vec![]; size];

        for (i, deps) in dependencies.iter_mut().enumerate().take(size).skip(1) {
            deps.push((i - 1, "^1.0".to_string()));
        }

        Self {
            packages,
            dependencies,
        }
    }

    /// Generate a diamond dependency pattern.
    #[must_use]
    pub fn diamond() -> Self {
        // A -> B, C; B -> D; C -> D
        Self {
            packages: vec!["A", "B", "C", "D"]
                .into_iter()
                .map(String::from)
                .collect(),
            dependencies: vec![
                vec![(1, "^1.0".to_string()), (2, "^1.0".to_string())], // A
                vec![(3, "^1.0".to_string())],                          // B
                vec![(3, "^1.0".to_string())],                          // C
                vec![],                                                 // D
            ],
        }
    }

    /// Generate a complex random graph.
    #[must_use]
    pub fn complex(num_packages: usize, avg_deps: usize) -> Self {
        let mut rng = rand::rng();
        let packages: Vec<String> = (0..num_packages).map(|i| format!("pkg{i}")).collect();
        let mut dependencies = vec![vec![]; num_packages];

        for (i, dep_list) in dependencies.iter_mut().enumerate().take(num_packages) {
            let num_deps = rng.random_range(0..=avg_deps * 2).min(i);
            let mut added = std::collections::HashSet::new();

            for _ in 0..num_deps {
                let dep = rng.random_range(0..i);
                if added.insert(dep) {
                    let constraint = format!("^{}.0", rng.random_range(1..5));
                    dep_list.push((dep, constraint));
                }
            }
        }

        Self {
            packages,
            dependencies,
        }
    }

    /// Generate a graph with potential conflicts.
    #[must_use]
    pub fn with_conflicts(num_packages: usize) -> Self {
        let mut graph = Self::complex(num_packages, 3);
        let mut rng = rand::rng();

        // Add conflicting version constraints
        for i in (num_packages / 2)..num_packages {
            if !graph.dependencies[i].is_empty() && rng.random_bool(0.3) {
                let (dep, _) = graph.dependencies[i][0];
                // Add a conflicting constraint
                graph.dependencies[i].push((dep, format!("<{}.0", rng.random_range(1..3))));
            }
        }

        graph
    }
}

/// Laravel-like project fixture.
#[must_use]
pub fn laravel_like_dependencies() -> Vec<(&'static str, &'static str)> {
    vec![
        ("laravel/framework", "^10.0"),
        ("guzzlehttp/guzzle", "^7.0"),
        ("laravel/sanctum", "^3.0"),
        ("laravel/tinker", "^2.8"),
        ("doctrine/dbal", "^3.0"),
        ("predis/predis", "^2.0"),
        ("league/flysystem-aws-s3-v3", "^3.0"),
        ("spatie/laravel-permission", "^5.0"),
        ("barryvdh/laravel-debugbar", "^3.0"),
        ("nunomaduro/collision", "^7.0"),
    ]
}

/// Symfony-like project fixture.
#[must_use]
pub fn symfony_like_dependencies() -> Vec<(&'static str, &'static str)> {
    vec![
        ("symfony/console", "^6.0"),
        ("symfony/http-kernel", "^6.0"),
        ("symfony/routing", "^6.0"),
        ("symfony/yaml", "^6.0"),
        ("symfony/twig-bundle", "^6.0"),
        ("symfony/form", "^6.0"),
        ("symfony/validator", "^6.0"),
        ("symfony/security-bundle", "^6.0"),
        ("doctrine/orm", "^2.0"),
        ("doctrine/doctrine-bundle", "^2.0"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_composer_json() {
        let json = generate_composer_json(10);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed["require"].as_object().unwrap().len() == 10);
    }

    #[test]
    fn test_generate_composer_lock() {
        let json = generate_composer_lock(100);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed["packages"].as_array().unwrap().len() == 100);
    }

    #[test]
    fn test_dependency_graph_linear() {
        let graph = DependencyGraph::linear(10);
        assert_eq!(graph.packages.len(), 10);
        assert!(graph.dependencies[0].is_empty());
        assert_eq!(graph.dependencies[9].len(), 1);
    }

    #[test]
    fn test_dependency_graph_diamond() {
        let graph = DependencyGraph::diamond();
        assert_eq!(graph.packages.len(), 4);
        assert_eq!(graph.dependencies[0].len(), 2); // A depends on B and C
    }
}
