//! Proptest strategies for Libretto types.
//!
//! This module provides strategies for generating random instances
//! of Libretto domain types for property-based testing.

use proptest::prelude::*;
use serde_json::{Value, json};

/// Strategy for generating valid vendor names.
pub fn vendor_name_strategy() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9-]{2,20}".prop_map(|s| s.to_lowercase())
}

/// Strategy for generating valid package names.
pub fn package_name_strategy() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9-]{2,30}".prop_map(|s| s.to_lowercase())
}

/// Strategy for generating full package names (vendor/package).
pub fn full_package_name_strategy() -> impl Strategy<Value = String> {
    (vendor_name_strategy(), package_name_strategy())
        .prop_map(|(vendor, package)| format!("{vendor}/{package}"))
}

/// Strategy for generating semantic versions.
pub fn semver_strategy() -> impl Strategy<Value = String> {
    (0u32..100, 0u32..100, 0u32..1000)
        .prop_map(|(major, minor, patch)| format!("{major}.{minor}.{patch}"))
}

/// Strategy for generating semantic versions with pre-release.
pub fn semver_with_prerelease_strategy() -> impl Strategy<Value = String> {
    let prerelease = prop_oneof![
        Just("alpha".to_string()),
        Just("beta".to_string()),
        Just("rc".to_string()),
        Just("dev".to_string()),
        (1u32..20).prop_map(|n| format!("alpha.{n}")),
        (1u32..20).prop_map(|n| format!("beta.{n}")),
        (1u32..10).prop_map(|n| format!("rc.{n}")),
    ];

    prop_oneof![
        semver_strategy(),
        (semver_strategy(), prerelease).prop_map(|(v, pre)| format!("{v}-{pre}")),
    ]
}

/// Strategy for generating dev branch versions.
pub fn dev_version_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("dev-main".to_string()),
        Just("dev-master".to_string()),
        Just("dev-develop".to_string()),
        "[a-z][a-z0-9-]{2,15}".prop_map(|s| format!("dev-{}", s.to_lowercase())),
    ]
}

/// Strategy for generating any valid version string.
pub fn version_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        8 => semver_strategy(),
        1 => semver_with_prerelease_strategy(),
        1 => dev_version_strategy(),
    ]
}

/// Strategy for generating caret constraints (^x.y.z).
pub fn caret_constraint_strategy() -> impl Strategy<Value = String> {
    semver_strategy().prop_map(|v| format!("^{v}"))
}

/// Strategy for generating tilde constraints (~x.y.z).
pub fn tilde_constraint_strategy() -> impl Strategy<Value = String> {
    semver_strategy().prop_map(|v| format!("~{v}"))
}

/// Strategy for generating exact version constraints.
pub fn exact_constraint_strategy() -> impl Strategy<Value = String> {
    semver_strategy()
}

/// Strategy for generating range constraints.
pub fn range_constraint_strategy() -> impl Strategy<Value = String> {
    (semver_strategy(), semver_strategy()).prop_map(|(v1, v2)| {
        // Ensure v1 < v2 for valid range
        format!(">={v1} <{v2}")
    })
}

/// Strategy for generating wildcard constraints.
pub fn wildcard_constraint_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("*".to_string()),
        (0u32..20).prop_map(|major| format!("{major}.*")),
        (0u32..20, 0u32..50).prop_map(|(major, minor)| format!("{major}.{minor}.*")),
    ]
}

/// Strategy for generating any single version constraint.
pub fn single_constraint_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => caret_constraint_strategy(),
        3 => tilde_constraint_strategy(),
        1 => exact_constraint_strategy(),
        1 => range_constraint_strategy(),
        1 => wildcard_constraint_strategy(),
    ]
}

/// Strategy for generating complex constraints with OR (||).
pub fn complex_constraint_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        4 => single_constraint_strategy(),
        1 => (single_constraint_strategy(), single_constraint_strategy())
            .prop_map(|(c1, c2)| format!("{c1} || {c2}")),
    ]
}

/// Strategy for generating constraints with stability flags.
pub fn constraint_with_stability_strategy() -> impl Strategy<Value = String> {
    let stability = prop_oneof![
        Just("@stable"),
        Just("@RC"),
        Just("@beta"),
        Just("@alpha"),
        Just("@dev"),
    ];

    prop_oneof![
        3 => single_constraint_strategy(),
        1 => (single_constraint_strategy(), stability)
            .prop_map(|(c, s)| format!("{c}{s}")),
    ]
}

/// Strategy for generating PHP namespace strings.
pub fn namespace_strategy() -> impl Strategy<Value = String> {
    let segment = "[A-Z][a-zA-Z0-9]*";
    prop::collection::vec(segment, 1..=5).prop_map(|segments| segments.join("\\"))
}

/// Strategy for generating PHP class names.
pub fn class_name_strategy() -> impl Strategy<Value = String> {
    "[A-Z][a-zA-Z0-9]{2,30}".prop_map(String::from)
}

/// Strategy for generating full qualified class names.
pub fn fqcn_strategy() -> impl Strategy<Value = String> {
    (namespace_strategy(), class_name_strategy())
        .prop_map(|(ns, class)| format!("{ns}\\{class}"))
}

/// Strategy for generating valid composer.json structure.
pub fn composer_json_strategy() -> impl Strategy<Value = Value> {
    let name = full_package_name_strategy();
    let description = "[a-zA-Z0-9 ]{10,50}";
    let pkg_type = prop_oneof![
        Just("library"),
        Just("project"),
        Just("metapackage"),
        Just("composer-plugin"),
    ];

    (name, description, pkg_type).prop_map(|(name, desc, ptype)| {
        json!({
            "name": name,
            "description": desc,
            "type": ptype,
            "require": {},
            "autoload": {}
        })
    })
}

/// Strategy for generating composer.json with dependencies.
pub fn composer_json_with_deps_strategy(max_deps: usize) -> impl Strategy<Value = Value> {
    let deps = prop::collection::hash_map(
        full_package_name_strategy(),
        single_constraint_strategy(),
        0..=max_deps,
    );

    (composer_json_strategy(), deps).prop_map(|(mut json, deps)| {
        if let Value::Object(ref mut obj) = json {
            let require: serde_json::Map<String, Value> = deps
                .into_iter()
                .map(|(k, v)| (k, Value::String(v)))
                .collect();
            obj.insert("require".to_string(), Value::Object(require));
        }
        json
    })
}

/// Strategy for generating autoload PSR-4 configuration.
pub fn psr4_autoload_strategy() -> impl Strategy<Value = Value> {
    let key_strategy = namespace_strategy().prop_map(|ns| format!("{ns}\\"));
    let value_strategy = "[a-z]{3,10}/".prop_map(|s| s.clone());

    prop::collection::hash_map(key_strategy, value_strategy, 1..=5).prop_map(|entries| {
        let map: serde_json::Map<String, Value> = entries
            .into_iter()
            .map(|(k, v)| (k, Value::String(v)))
            .collect();
        json!({ "psr-4": map })
    })
}

/// Strategy for generating lock file package entries.
pub fn lock_package_strategy() -> impl Strategy<Value = Value> {
    let name = full_package_name_strategy();
    let version = semver_strategy();
    let reference = "[a-f0-9]{40}";

    (name, version, reference).prop_map(|(name, version, reference)| {
        json!({
            "name": name,
            "version": version,
            "source": {
                "type": "git",
                "url": format!("https://github.com/{}.git", name),
                "reference": reference
            },
            "dist": {
                "type": "zip",
                "url": format!("https://api.github.com/repos/{}/zipball/{}", name, version),
                "reference": reference,
                "shasum": ""
            },
            "require": {},
            "type": "library"
        })
    })
}

/// Strategy for generating complete lock file.
pub fn composer_lock_strategy(max_packages: usize) -> impl Strategy<Value = Value> {
    let packages = prop::collection::vec(lock_package_strategy(), 0..=max_packages);
    let content_hash = "[a-f0-9]{32}";

    (packages, content_hash).prop_map(|(packages, hash)| {
        json!({
            "content-hash": hash,
            "packages": packages,
            "packages-dev": [],
            "aliases": [],
            "minimum-stability": "stable",
            "prefer-stable": true,
            "prefer-lowest": false,
            "platform": {},
            "platform-dev": {}
        })
    })
}

/// Strategy for generating URL strings.
pub fn url_strategy() -> impl Strategy<Value = String> {
    let domain = "[a-z]{3,15}\\.(com|org|io|net)";
    let path =
        prop::collection::vec("[a-z0-9-]{2,10}", 0..=4).prop_map(|segments| segments.join("/"));

    (domain, path).prop_map(|(domain, path)| {
        if path.is_empty() {
            format!("https://{domain}")
        } else {
            format!("https://{domain}/{path}")
        }
    })
}

/// Strategy for generating git URLs.
pub fn git_url_strategy() -> impl Strategy<Value = String> {
    let owner = "[a-z][a-z0-9-]{2,15}";
    let repo = "[a-z][a-z0-9-]{2,30}";

    (owner, repo).prop_map(|(owner, repo)| format!("https://github.com/{owner}/{repo}.git"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::strategy::ValueTree;
    use proptest::test_runner::TestRunner;

    #[test]
    fn test_vendor_name_strategy() {
        let mut runner = TestRunner::default();
        for _ in 0..100 {
            let name = vendor_name_strategy()
                .new_tree(&mut runner)
                .unwrap()
                .current();
            assert!(name.len() >= 3);
            assert!(name.chars().next().unwrap().is_ascii_lowercase());
        }
    }

    #[test]
    fn test_semver_strategy() {
        let mut runner = TestRunner::default();
        for _ in 0..100 {
            let version = semver_strategy().new_tree(&mut runner).unwrap().current();
            let parts: Vec<&str> = version.split('.').collect();
            assert_eq!(parts.len(), 3);
            assert!(parts.iter().all(|p: &&str| p.parse::<u32>().is_ok()));
        }
    }

    #[test]
    fn test_composer_json_strategy() {
        let mut runner = TestRunner::default();
        for _ in 0..20 {
            let json = composer_json_strategy()
                .new_tree(&mut runner)
                .unwrap()
                .current();
            assert!(json["name"].is_string());
            assert!(json["type"].is_string());
        }
    }

    proptest! {
        #[test]
        fn prop_full_package_name_valid(name in full_package_name_strategy()) {
            assert!(name.contains('/'));
            let parts: Vec<&str> = name.split('/').collect();
            assert_eq!(parts.len(), 2);
            assert!(!parts[0].is_empty());
            assert!(!parts[1].is_empty());
        }

        #[test]
        fn prop_semver_valid_format(version in semver_strategy()) {
            let parts: Vec<&str> = version.split('.').collect();
            assert_eq!(parts.len(), 3);
            for part in parts {
                assert!(part.parse::<u32>().is_ok());
            }
        }

        #[test]
        fn prop_constraint_not_empty(constraint in single_constraint_strategy()) {
            assert!(!constraint.is_empty());
        }

        #[test]
        fn prop_namespace_valid_format(ns in namespace_strategy()) {
            assert!(!ns.is_empty());
            // Each segment should start with uppercase
            for segment in ns.split('\\') {
                assert!(segment.chars().next().unwrap().is_ascii_uppercase());
            }
        }
    }
}
