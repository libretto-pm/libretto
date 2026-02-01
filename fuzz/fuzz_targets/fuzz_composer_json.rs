#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use libretto_core::json::{from_json, to_json};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Arbitrary composer.json structure for structured fuzzing.
#[derive(Debug, Clone, Arbitrary, Serialize, Deserialize)]
struct FuzzComposerJson {
    name: Option<String>,
    description: Option<String>,
    version: Option<String>,
    #[serde(rename = "type")]
    package_type: Option<String>,
    keywords: Option<Vec<String>>,
    homepage: Option<String>,
    license: Option<String>,
    require: HashMap<String, String>,
    #[serde(rename = "require-dev")]
    require_dev: HashMap<String, String>,
    autoload: Option<FuzzAutoload>,
    #[serde(rename = "minimum-stability")]
    minimum_stability: Option<String>,
    #[serde(rename = "prefer-stable")]
    prefer_stable: Option<bool>,
}

#[derive(Debug, Clone, Arbitrary, Serialize, Deserialize)]
struct FuzzAutoload {
    #[serde(rename = "psr-4")]
    psr4: Option<HashMap<String, String>>,
    #[serde(rename = "psr-0")]
    psr0: Option<HashMap<String, String>>,
    classmap: Option<Vec<String>>,
    files: Option<Vec<String>>,
}

fuzz_target!(|data: FuzzComposerJson| {
    // Serialize to JSON
    if let Ok(json_str) = to_json(&data) {
        // Verify it's valid JSON by parsing back
        let result = from_json::<serde_json::Value>(&json_str);
        assert!(
            result.is_ok(),
            "Serialized JSON should be valid: {}",
            json_str
        );

        // Test that we can parse it back to a similar structure
        // (may not be identical due to Option handling)
        let _ = from_json::<FuzzComposerJson>(&json_str);
    }
});
