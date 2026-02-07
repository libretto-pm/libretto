//! High-performance JSON operations using sonic-rs.

use crate::{Error, Result};
use serde::{Serialize, de::DeserializeOwned};

/// Deserialize JSON string.
///
/// # Errors
/// Returns error if JSON is invalid.
pub fn from_json<T: DeserializeOwned>(s: &str) -> Result<T> {
    sonic_rs::from_str(s).map_err(Error::from)
}

/// Deserialize JSON bytes.
///
/// # Errors
/// Returns error if JSON is invalid.
pub fn from_json_slice<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    sonic_rs::from_slice(bytes).map_err(Error::from)
}

/// Serialize to compact JSON.
///
/// # Errors
/// Returns error if serialization fails.
pub fn to_json<T: Serialize>(value: &T) -> Result<String> {
    sonic_rs::to_string(value).map_err(Error::from)
}

/// Serialize to pretty JSON.
///
/// # Errors
/// Returns error if serialization fails.
pub fn to_json_pretty<T: Serialize>(value: &T) -> Result<String> {
    sonic_rs::to_string_pretty(value).map_err(Error::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use serde::{Deserialize, Serialize};
    use std::collections::HashMap;

    #[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
    struct Test {
        name: String,
        value: i32,
    }

    #[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
    struct ComposerJson {
        name: String,
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

    #[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
    struct NestedStruct {
        id: u64,
        data: InnerData,
        tags: Vec<String>,
    }

    #[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
    struct InnerData {
        key: String,
        values: Vec<i32>,
    }

    // ========== Basic Unit Tests ==========

    #[test]
    fn roundtrip() {
        let orig = Test {
            name: "test".into(),
            value: 42,
        };
        let json = to_json(&orig).expect("serialization should succeed");
        let parsed: Test = from_json(&json).expect("deserialization should succeed");
        assert_eq!(orig, parsed);
    }

    #[test]
    fn pretty() {
        let val = Test {
            name: "x".into(),
            value: 1,
        };
        let pretty = to_json_pretty(&val).expect("pretty printing should succeed");
        assert!(pretty.contains('\n'));
    }

    #[test]
    fn test_from_json_slice() {
        let json = r#"{"name":"test","value":42}"#;
        let parsed: Test = from_json_slice(json.as_bytes()).expect("should parse from bytes");
        assert_eq!(parsed.name, "test");
        assert_eq!(parsed.value, 42);
    }

    #[test]
    fn test_composer_json_parsing() {
        let json = r#"{
            "name": "vendor/package",
            "description": "A test package",
            "type": "library",
            "require": {
                "php": ">=8.0",
                "monolog/monolog": "^3.0"
            },
            "require-dev": {
                "phpunit/phpunit": "^10.0"
            }
        }"#;

        let parsed: ComposerJson = from_json(json).expect("should parse composer.json");
        assert_eq!(parsed.name, "vendor/package");
        assert_eq!(parsed.require.get("php"), Some(&">=8.0".to_string()));
        assert_eq!(parsed.require_dev.len(), 1);
    }

    #[test]
    fn test_nested_struct() {
        let data = NestedStruct {
            id: 123,
            data: InnerData {
                key: "test".to_string(),
                values: vec![1, 2, 3],
            },
            tags: vec!["a".to_string(), "b".to_string()],
        };

        let json = to_json(&data).expect("should serialize");
        let parsed: NestedStruct = from_json(&json).expect("should deserialize");
        assert_eq!(data, parsed);
    }

    #[test]
    fn test_invalid_json_error() {
        let result: Result<Test> = from_json("{invalid json}");
        assert!(result.is_err());
    }

    #[test]
    fn test_type_mismatch_error() {
        let result: Result<Test> = from_json(r#"{"name": 123, "value": "not a number"}"#);
        assert!(result.is_err());
    }

    #[test]
    fn test_missing_field_error() {
        let result: Result<Test> = from_json(r#"{"name": "test"}"#);
        assert!(result.is_err());
    }

    #[test]
    fn test_unicode_handling() {
        let data = Test {
            name: "测试 🎉 émojis".to_string(),
            value: 42,
        };
        let json = to_json(&data).expect("should serialize unicode");
        let parsed: Test = from_json(&json).expect("should deserialize unicode");
        assert_eq!(data, parsed);
    }

    #[test]
    fn test_special_characters() {
        let data = Test {
            name: "tab:\t newline:\n quote:\" backslash:\\".to_string(),
            value: 0,
        };
        let json = to_json(&data).expect("should escape special chars");
        let parsed: Test = from_json(&json).expect("should unescape special chars");
        assert_eq!(data, parsed);
    }

    #[test]
    fn test_empty_collections() {
        let data = ComposerJson {
            name: "test/empty".to_string(),
            description: None,
            version: None,
            package_type: None,
            require: HashMap::new(),
            require_dev: HashMap::new(),
        };
        let json = to_json(&data).expect("should serialize empty collections");
        let parsed: ComposerJson = from_json(&json).expect("should deserialize");
        assert!(parsed.require.is_empty());
    }

    #[test]
    fn test_large_numbers() {
        #[derive(Debug, Serialize, Deserialize, PartialEq)]
        struct Numbers {
            big_int: i64,
            big_uint: u64,
            float: f64,
        }

        let data = Numbers {
            big_int: i64::MAX,
            big_uint: u64::MAX,
            float: std::f64::consts::PI,
        };
        let json = to_json(&data).expect("should serialize large numbers");
        let parsed: Numbers = from_json(&json).expect("should deserialize large numbers");
        assert_eq!(data.big_int, parsed.big_int);
        assert_eq!(data.big_uint, parsed.big_uint);
        assert!((data.float - parsed.float).abs() < f64::EPSILON);
    }

    // ========== Property-Based Tests ==========

    proptest! {
        /// Serialization followed by deserialization returns original value
        #[test]
        fn prop_roundtrip_string(s in "\\PC*") {
            let data = Test { name: s.clone(), value: 0 };
            let json = to_json(&data).expect("should serialize");
            let parsed: Test = from_json(&json).expect("should deserialize");
            prop_assert_eq!(data.name, parsed.name);
        }

        /// Serialization roundtrip preserves integer values
        #[test]
        fn prop_roundtrip_integer(v in i32::MIN..i32::MAX) {
            let data = Test { name: "test".to_string(), value: v };
            let json = to_json(&data).expect("should serialize");
            let parsed: Test = from_json(&json).expect("should deserialize");
            prop_assert_eq!(v, parsed.value);
        }

        /// Serialization roundtrip with arbitrary strings
        #[test]
        fn prop_roundtrip_any_string(name in ".*", value in i32::MIN..i32::MAX) {
            let data = Test { name, value };
            if let Ok(json) = to_json(&data)
                && let Ok(parsed) = from_json::<Test>(&json) {
                    prop_assert_eq!(data, parsed);
                }
        }

        /// Pretty printing always contains newlines for non-trivial structures
        #[test]
        fn prop_pretty_has_newlines(name in "[a-zA-Z]{1,20}", value in 0i32..1000) {
            let data = Test { name, value };
            let pretty = to_json_pretty(&data).expect("should pretty print");
            prop_assert!(pretty.contains('\n'), "Pretty output should contain newlines");
        }

        /// Compact JSON never contains unescaped newlines in values
        #[test]
        fn prop_compact_no_raw_newlines(name in "[a-zA-Z0-9]{1,50}", value in 0i32..1000) {
            let data = Test { name, value };
            let json = to_json(&data).expect("should serialize");
            // JSON should not have literal newlines (they should be escaped if in the value)
            prop_assert!(!json.contains('\n'), "Compact JSON should not contain newlines");
        }

        /// HashMap roundtrip preserves all keys and values
        #[test]
        fn prop_hashmap_roundtrip(
            entries in prop::collection::hash_map("[a-z]{1,10}", "[a-z0-9]{1,20}", 0..10)
        ) {
            let data = ComposerJson {
                name: "test/pkg".to_string(),
                description: None,
                version: None,
                package_type: None,
                require: entries.clone(),
                require_dev: HashMap::new(),
            };
            let json = to_json(&data).expect("should serialize");
            let parsed: ComposerJson = from_json(&json).expect("should deserialize");
            prop_assert_eq!(entries.len(), parsed.require.len());
            for (k, v) in entries {
                prop_assert_eq!(Some(&v), parsed.require.get(&k));
            }
        }

        /// Vec roundtrip preserves order and values
        #[test]
        fn prop_vec_roundtrip(values in prop::collection::vec(any::<i32>(), 0..100)) {
            let data = InnerData {
                key: "test".to_string(),
                values: values.clone(),
            };
            let json = to_json(&data).expect("should serialize");
            let parsed: InnerData = from_json(&json).expect("should deserialize");
            prop_assert_eq!(values, parsed.values);
        }

        /// Nested struct roundtrip
        #[test]
        fn prop_nested_roundtrip(
            id in 0u64..10000,
            key in "[a-z]{1,20}",
            values in prop::collection::vec(-1000i32..1000, 0..20),
            tags in prop::collection::vec("[a-z]{1,10}", 0..10)
        ) {
            let data = NestedStruct {
                id,
                data: InnerData { key, values },
                tags,
            };
            let json = to_json(&data).expect("should serialize");
            let parsed: NestedStruct = from_json(&json).expect("should deserialize");
            prop_assert_eq!(data, parsed);
        }

        /// from_json and from_json_slice produce identical results
        #[test]
        fn prop_slice_equals_str(name in "[a-zA-Z0-9]{1,30}", value in any::<i32>()) {
            let data = Test { name, value };
            let json = to_json(&data).expect("should serialize");

            let from_str: Test = from_json(&json).expect("should parse from str");
            let from_slice: Test = from_json_slice(json.as_bytes()).expect("should parse from slice");

            prop_assert_eq!(from_str, from_slice);
        }

        /// Serializing twice produces identical output (deterministic)
        #[test]
        fn prop_serialization_deterministic(name in "[a-z]{1,20}", value in any::<i32>()) {
            let data = Test { name, value };
            let json1 = to_json(&data).expect("should serialize");
            let json2 = to_json(&data).expect("should serialize again");
            prop_assert_eq!(json1, json2);
        }

        /// Pretty and compact parse to the same value
        #[test]
        fn prop_pretty_compact_equivalent(name in "[a-z]{1,20}", value in any::<i32>()) {
            let data = Test { name, value };
            let compact = to_json(&data).expect("should serialize compact");
            let pretty = to_json_pretty(&data).expect("should serialize pretty");

            let from_compact: Test = from_json(&compact).expect("should parse compact");
            let from_pretty: Test = from_json(&pretty).expect("should parse pretty");

            prop_assert_eq!(from_compact, from_pretty);
        }

        /// Package name format validation (vendor/package)
        #[test]
        fn prop_package_name_roundtrip(
            vendor in "[a-z][a-z0-9-]{2,20}",
            package in "[a-z][a-z0-9-]{2,30}"
        ) {
            let name = format!("{vendor}/{package}");
            let data = ComposerJson {
                name: name.clone(),
                description: None,
                version: None,
                package_type: None,
                require: HashMap::new(),
                require_dev: HashMap::new(),
            };
            let json = to_json(&data).expect("should serialize");
            let parsed: ComposerJson = from_json(&json).expect("should deserialize");
            prop_assert_eq!(name, parsed.name);
        }

        /// Version constraint strings roundtrip correctly
        #[test]
        fn prop_constraint_strings_roundtrip(
            major in 0u32..20,
            minor in 0u32..50,
            patch in 0u32..100
        ) {
            let constraints = vec![
                format!("^{}.{}.{}", major, minor, patch),
                format!("~{}.{}", major, minor),
                format!(">={}.{}.{}", major, minor, patch),
                format!("{}.{}.{}", major, minor, patch),
            ];

            for constraint in constraints {
                let mut require = HashMap::new();
                require.insert("test/pkg".to_string(), constraint.clone());

                let data = ComposerJson {
                    name: "test/project".to_string(),
                    description: None,
                    version: None,
                    package_type: None,
                    require,
                    require_dev: HashMap::new(),
                };

                let json = to_json(&data).expect("should serialize");
                let parsed: ComposerJson = from_json(&json).expect("should deserialize");
                prop_assert_eq!(
                    Some(&constraint),
                    parsed.require.get("test/pkg"),
                    "Constraint should roundtrip correctly"
                );
            }
        }
    }

    // ========== Edge Cases and Error Handling ==========

    #[test]
    fn test_deeply_nested_json() {
        // Test parsing deeply nested structures (should not stack overflow)
        let mut json = r#"{"a":"#.to_string();
        for _ in 0..50 {
            json.push_str(r#"{"b":"#);
        }
        json.push_str(r#""value""#);
        for _ in 0..50 {
            json.push('}');
        }
        json.push('}');

        // Should parse without panic (may fail due to depth limit, that's ok)
        let result: Result<sonic_rs::Value> = from_json(&json);
        // Just ensure no panic
        let _ = result;
    }

    #[test]
    fn test_large_array() {
        let arr: Vec<i32> = (0..10000).collect();
        let json = to_json(&arr).expect("should serialize large array");
        let parsed: Vec<i32> = from_json(&json).expect("should deserialize large array");
        assert_eq!(arr, parsed);
    }

    #[test]
    fn test_empty_object() {
        let json = "{}";
        let parsed: HashMap<String, String> = from_json(json).expect("should parse empty object");
        assert!(parsed.is_empty());
    }

    #[test]
    fn test_empty_array() {
        let json = "[]";
        let parsed: Vec<i32> = from_json(json).expect("should parse empty array");
        assert!(parsed.is_empty());
    }

    #[test]
    fn test_null_handling() {
        #[derive(Debug, Serialize, Deserialize, PartialEq)]
        struct WithOption {
            value: Option<String>,
        }

        let json_null = r#"{"value":null}"#;
        let json_missing = r"{}";

        let from_null: WithOption = from_json(json_null).expect("should parse null");
        let from_missing: WithOption = from_json(json_missing).expect("should parse missing");

        assert_eq!(from_null.value, None);
        assert_eq!(from_missing.value, None);
    }

    #[test]
    fn test_boolean_values() {
        #[derive(Debug, Serialize, Deserialize, PartialEq)]
        struct WithBool {
            flag: bool,
        }

        let true_json = r#"{"flag":true}"#;
        let false_json = r#"{"flag":false}"#;

        let from_true: WithBool = from_json(true_json).expect("should parse true");
        let from_false: WithBool = from_json(false_json).expect("should parse false");

        assert!(from_true.flag);
        assert!(!from_false.flag);
    }

    #[test]
    fn test_utf8_boundary() {
        // Test various UTF-8 boundary conditions
        let test_strings = vec![
            "\u{0000}",   // Null character
            "\u{007F}",   // Highest single-byte
            "\u{0080}",   // Lowest two-byte
            "\u{07FF}",   // Highest two-byte
            "\u{0800}",   // Lowest three-byte
            "\u{FFFF}",   // Highest three-byte
            "\u{10000}",  // Lowest four-byte (surrogate pair in UTF-16)
            "\u{10FFFF}", // Highest valid Unicode
        ];

        for s in test_strings {
            let data = Test {
                name: s.to_string(),
                value: 0,
            };
            if let Ok(json) = to_json(&data)
                && let Ok(parsed) = from_json::<Test>(&json)
            {
                assert_eq!(data.name, parsed.name);
            }
        }
    }
}
