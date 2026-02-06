//! Lock file generation using `sonic_rs` for maximum performance.

use anyhow::Result;
use libretto_resolver::Resolution;
use sonic_rs::{JsonContainerTrait, JsonValueTrait, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Generate a composer.lock file from resolution results.
pub fn generate_lock_file(
    lock_path: &PathBuf,
    resolution: &Resolution,
    composer: &Value,
) -> Result<()> {
    let mut packages: Vec<BTreeMap<String, Value>> = Vec::new();
    let mut packages_dev: Vec<BTreeMap<String, Value>> = Vec::new();

    for pkg in &resolution.packages {
        let mut entry: BTreeMap<String, Value> = BTreeMap::new();

        entry.insert("name".to_string(), Value::from(pkg.name.as_str()));
        entry.insert(
            "version".to_string(),
            Value::from(pkg.version.to_string().as_str()),
        );

        // Source information
        if let Some(ref url) = pkg.source_url {
            let mut source: BTreeMap<String, Value> = BTreeMap::new();
            source.insert(
                "type".to_string(),
                Value::from(pkg.source_type.as_deref().unwrap_or("git")),
            );
            source.insert("url".to_string(), Value::from(url.as_str()));
            source.insert(
                "reference".to_string(),
                Value::from(pkg.source_reference.as_deref().unwrap_or("")),
            );
            entry.insert(
                "source".to_string(),
                sonic_rs::to_value(&source).unwrap_or_default(),
            );
        }

        // Dist information
        if let Some(ref url) = pkg.dist_url {
            let mut dist: BTreeMap<String, Value> = BTreeMap::new();
            dist.insert(
                "type".to_string(),
                Value::from(pkg.dist_type.as_deref().unwrap_or("zip")),
            );
            dist.insert("url".to_string(), Value::from(url.as_str()));
            dist.insert(
                "reference".to_string(),
                Value::from(pkg.source_reference.as_deref().unwrap_or("")),
            );
            dist.insert(
                "shasum".to_string(),
                Value::from(pkg.dist_shasum.as_deref().unwrap_or("")),
            );
            entry.insert(
                "dist".to_string(),
                sonic_rs::to_value(&dist).unwrap_or_default(),
            );
        }

        // Require dependencies
        if let Some(ref require) = pkg.require {
            let mut req_map: BTreeMap<String, String> = BTreeMap::new();
            for (name, constraint) in require {
                req_map.insert(name.clone(), constraint.clone());
            }
            if !req_map.is_empty() {
                entry.insert(
                    "require".to_string(),
                    sonic_rs::to_value(&req_map).unwrap_or_default(),
                );
            }
        }

        // Require-dev dependencies
        if let Some(ref require_dev) = pkg.require_dev {
            let mut req_map: BTreeMap<String, String> = BTreeMap::new();
            for (name, constraint) in require_dev {
                req_map.insert(name.clone(), constraint.clone());
            }
            if !req_map.is_empty() {
                entry.insert(
                    "require-dev".to_string(),
                    sonic_rs::to_value(&req_map).unwrap_or_default(),
                );
            }
        }

        // Suggest
        if let Some(ref suggest) = pkg.suggest {
            let mut sug_map: BTreeMap<String, String> = BTreeMap::new();
            for (name, desc) in suggest {
                sug_map.insert(name.clone(), desc.clone());
            }
            if !sug_map.is_empty() {
                entry.insert(
                    "suggest".to_string(),
                    sonic_rs::to_value(&sug_map).unwrap_or_default(),
                );
            }
        }

        // Type
        let pkg_type = pkg.package_type.as_deref().unwrap_or("library");
        entry.insert("type".to_string(), Value::from(pkg_type));

        // Extra
        if let Some(ref extra) = pkg.extra {
            entry.insert("extra".to_string(), extra.clone());
        }

        // Autoload
        if let Some(ref autoload) = pkg.autoload {
            entry.insert("autoload".to_string(), autoload.clone());
        }

        // Notification URL
        let notif_url = pkg
            .notification_url
            .as_deref()
            .unwrap_or("https://packagist.org/downloads/");
        entry.insert("notification-url".to_string(), Value::from(notif_url));

        // License
        if let Some(ref license) = pkg.license {
            entry.insert(
                "license".to_string(),
                sonic_rs::to_value(license).unwrap_or_default(),
            );
        }

        // Authors
        if let Some(ref authors) = pkg.authors {
            entry.insert("authors".to_string(), authors.clone());
        }

        // Provide/Replace/Conflict
        if let Some(ref provide) = pkg.provide {
            let mut deps: BTreeMap<String, Value> = BTreeMap::new();
            for (name, version) in provide {
                deps.insert(name.clone(), Value::from(version.as_str()));
            }
            entry.insert(
                "provide".to_string(),
                sonic_rs::to_value(&deps).unwrap_or_default(),
            );
        }

        if let Some(ref replace) = pkg.replace {
            let mut deps: BTreeMap<String, Value> = BTreeMap::new();
            for (name, version) in replace {
                deps.insert(name.clone(), Value::from(version.as_str()));
            }
            entry.insert(
                "replace".to_string(),
                sonic_rs::to_value(&deps).unwrap_or_default(),
            );
        }

        if let Some(ref conflict) = pkg.conflict {
            let mut deps: BTreeMap<String, Value> = BTreeMap::new();
            for (name, version) in conflict {
                deps.insert(name.clone(), Value::from(version.as_str()));
            }
            entry.insert(
                "conflict".to_string(),
                sonic_rs::to_value(&deps).unwrap_or_default(),
            );
        }

        // Description
        if let Some(ref desc) = pkg.description {
            entry.insert("description".to_string(), Value::from(desc.as_str()));
        }

        // Homepage
        if let Some(ref homepage) = pkg.homepage {
            entry.insert("homepage".to_string(), Value::from(homepage.as_str()));
        }

        // Keywords
        if let Some(ref keywords) = pkg.keywords {
            entry.insert(
                "keywords".to_string(),
                sonic_rs::to_value(keywords).unwrap_or_default(),
            );
        }

        // Support
        if let Some(ref support) = pkg.support {
            entry.insert("support".to_string(), support.clone());
        }

        // Funding
        if let Some(ref funding) = pkg.funding {
            entry.insert("funding".to_string(), funding.clone());
        }

        // Time
        if let Some(ref time) = pkg.time {
            entry.insert("time".to_string(), Value::from(time.as_str()));
        }

        // Bin
        if let Some(ref bin) = pkg.bin
            && !bin.is_empty()
        {
            entry.insert(
                "bin".to_string(),
                sonic_rs::to_value(bin).unwrap_or_default(),
            );
        }

        if pkg.is_dev {
            packages_dev.push(entry);
        } else {
            packages.push(entry);
        }
    }

    // Sort packages by name for deterministic output
    packages.sort_by(|a, b| {
        let name_a = a.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let name_b = b.get("name").and_then(|v| v.as_str()).unwrap_or("");
        name_a.cmp(name_b)
    });
    packages_dev.sort_by(|a, b| {
        let name_a = a.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let name_b = b.get("name").and_then(|v| v.as_str()).unwrap_or("");
        name_a.cmp(name_b)
    });

    // Extract platform requirements from composer.json
    let mut platform: BTreeMap<String, String> = BTreeMap::new();
    let mut platform_dev: BTreeMap<String, String> = BTreeMap::new();

    if let Some(req) = composer.get("require").and_then(|v| v.as_object()) {
        for (name, constraint) in req {
            if (name == "php" || name.starts_with("ext-") || name.starts_with("lib-"))
                && let Some(c) = constraint.as_str()
            {
                platform.insert(name.to_string(), c.to_string());
            }
        }
    }
    if let Some(req) = composer.get("require-dev").and_then(|v| v.as_object()) {
        for (name, constraint) in req {
            if (name == "php" || name.starts_with("ext-") || name.starts_with("lib-"))
                && let Some(c) = constraint.as_str()
            {
                platform_dev.insert(name.to_string(), c.to_string());
            }
        }
    }

    let content_hash =
        libretto_core::ContentHash::from_bytes(sonic_rs::to_string(composer)?.as_bytes());

    let min_stability = composer
        .get("minimum-stability")
        .and_then(|v| v.as_str())
        .unwrap_or("stable");
    let prefer_stable = composer
        .get("prefer-stable")
        .and_then(sonic_rs::JsonValueTrait::as_bool)
        .unwrap_or(false);

    // Build lock structure
    let mut lock: BTreeMap<String, Value> = BTreeMap::new();

    let readme = vec![
        "This file locks the dependencies of your project to a known state",
        "Read more about it at https://getcomposer.org/doc/01-basic-usage.md#installing-dependencies",
        "This file is @generated automatically",
    ];
    lock.insert(
        "_readme".to_string(),
        sonic_rs::to_value(&readme).unwrap_or_default(),
    );
    lock.insert(
        "content-hash".to_string(),
        Value::from(content_hash.to_hex().as_str()),
    );
    lock.insert(
        "packages".to_string(),
        sonic_rs::to_value(&packages).unwrap_or_default(),
    );
    lock.insert(
        "packages-dev".to_string(),
        sonic_rs::to_value(&packages_dev).unwrap_or_default(),
    );
    lock.insert(
        "aliases".to_string(),
        sonic_rs::to_value::<Vec<String>>(&vec![]).unwrap_or_default(),
    );
    lock.insert("minimum-stability".to_string(), Value::from(min_stability));
    lock.insert(
        "stability-flags".to_string(),
        sonic_rs::to_value::<Vec<String>>(&vec![]).unwrap_or_default(),
    );
    lock.insert("prefer-stable".to_string(), Value::from(prefer_stable));
    lock.insert("prefer-lowest".to_string(), Value::from(false));
    lock.insert(
        "platform".to_string(),
        sonic_rs::to_value(&platform).unwrap_or_default(),
    );
    lock.insert(
        "platform-dev".to_string(),
        sonic_rs::to_value(&platform_dev).unwrap_or_default(),
    );
    lock.insert("plugin-api-version".to_string(), Value::from("2.6.0"));

    let output = sonic_rs::to_string_pretty(&lock)?;
    std::fs::write(lock_path, format!("{output}\n"))?;

    Ok(())
}
