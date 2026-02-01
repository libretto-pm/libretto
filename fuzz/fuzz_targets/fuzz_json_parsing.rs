#![no_main]

use libfuzzer_sys::fuzz_target;
use libretto_core::json::{from_json, from_json_slice, to_json, to_json_pretty};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A structure that mirrors common composer.json fields.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct ComposerJson {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(rename = "type", default)]
    package_type: Option<String>,
    #[serde(default)]
    require: HashMap<String, String>,
    #[serde(rename = "require-dev", default)]
    require_dev: HashMap<String, String>,
}

fuzz_target!(|data: &[u8]| {
    // Try to parse as JSON from bytes
    if let Ok(parsed) = from_json_slice::<ComposerJson>(data) {
        // If parsing succeeded, try to serialize back
        if let Ok(json_str) = to_json(&parsed) {
            // Parse the serialized output
            if let Ok(reparsed) = from_json::<ComposerJson>(&json_str) {
                // Values should be equal after roundtrip
                assert_eq!(parsed, reparsed);
            }
        }

        // Also test pretty printing
        if let Ok(pretty) = to_json_pretty(&parsed) {
            if let Ok(reparsed) = from_json::<ComposerJson>(&pretty) {
                assert_eq!(parsed, reparsed);
            }
        }
    }

    // Also try parsing as a string
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = from_json::<ComposerJson>(s);

        // Try parsing as generic JSON value
        let _ = from_json::<serde_json::Value>(s);
    }
});
