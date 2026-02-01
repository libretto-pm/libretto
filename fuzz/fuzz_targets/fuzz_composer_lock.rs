#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use libretto_core::json::{from_json, to_json};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Arbitrary composer.lock structure for structured fuzzing.
#[derive(Debug, Clone, Arbitrary, Serialize, Deserialize)]
struct FuzzComposerLock {
    #[serde(rename = "content-hash")]
    content_hash: Option<String>,
    packages: Vec<FuzzPackage>,
    #[serde(rename = "packages-dev")]
    packages_dev: Vec<FuzzPackage>,
    aliases: Vec<FuzzAlias>,
    #[serde(rename = "minimum-stability")]
    minimum_stability: Option<String>,
    #[serde(rename = "prefer-stable")]
    prefer_stable: Option<bool>,
    #[serde(rename = "prefer-lowest")]
    prefer_lowest: Option<bool>,
    platform: HashMap<String, String>,
    #[serde(rename = "platform-dev")]
    platform_dev: HashMap<String, String>,
}

#[derive(Debug, Clone, Arbitrary, Serialize, Deserialize)]
struct FuzzPackage {
    name: String,
    version: String,
    source: Option<FuzzSource>,
    dist: Option<FuzzDist>,
    require: HashMap<String, String>,
    #[serde(rename = "require-dev")]
    require_dev: HashMap<String, String>,
    #[serde(rename = "type")]
    package_type: Option<String>,
    autoload: Option<FuzzAutoload>,
    license: Option<Vec<String>>,
    description: Option<String>,
}

#[derive(Debug, Clone, Arbitrary, Serialize, Deserialize)]
struct FuzzSource {
    #[serde(rename = "type")]
    source_type: String,
    url: String,
    reference: String,
}

#[derive(Debug, Clone, Arbitrary, Serialize, Deserialize)]
struct FuzzDist {
    #[serde(rename = "type")]
    dist_type: String,
    url: String,
    reference: String,
    shasum: Option<String>,
}

#[derive(Debug, Clone, Arbitrary, Serialize, Deserialize)]
struct FuzzAutoload {
    #[serde(rename = "psr-4")]
    psr4: Option<HashMap<String, String>>,
    classmap: Option<Vec<String>>,
    files: Option<Vec<String>>,
}

#[derive(Debug, Clone, Arbitrary, Serialize, Deserialize)]
struct FuzzAlias {
    package: String,
    version: String,
    alias: String,
    #[serde(rename = "alias_normalized")]
    alias_normalized: String,
}

fuzz_target!(|data: FuzzComposerLock| {
    // Serialize to JSON
    if let Ok(json_str) = to_json(&data) {
        // Verify it's valid JSON by parsing back
        let result = from_json::<serde_json::Value>(&json_str);
        assert!(
            result.is_ok(),
            "Serialized lock JSON should be valid: {}",
            json_str
        );

        // Test that we can parse it back
        let _ = from_json::<FuzzComposerLock>(&json_str);
    }
});
