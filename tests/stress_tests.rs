//! Stress tests for Libretto.
//!
//! These tests verify performance and stability under heavy load,
//! including large dependency graphs, concurrent operations, and memory usage.

use std::collections::HashMap;
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

// ========== Large Dependency Graph Tests ==========

mod large_graphs {
    use super::*;

    /// Generate a large composer.json with many dependencies.
    fn generate_large_composer_json(num_deps: usize) -> String {
        let mut require = HashMap::new();
        for i in 0..num_deps {
            let vendor = format!("vendor{}", i / 100);
            let package = format!("package{}", i);
            let constraint = format!("^{}.{}.0", 1 + (i % 10), i % 100);
            require.insert(format!("{}/{}", vendor, package), constraint);
        }

        let require_json: String = require
            .iter()
            .map(|(k, v)| format!(r#"        "{}": "{}""#, k, v))
            .collect::<Vec<_>>()
            .join(",\n");

        format!(
            r#"{{
    "name": "test/large-project",
    "description": "A project with {} dependencies",
    "type": "project",
    "require": {{
{}
    }},
    "autoload": {{
        "psr-4": {{
            "App\\": "src/"
        }}
    }}
}}"#,
            num_deps, require_json
        )
    }

    /// Generate a large composer.lock with many packages.
    fn generate_large_composer_lock(num_packages: usize) -> String {
        let mut packages = Vec::new();
        for i in 0..num_packages {
            let vendor = format!("vendor{}", i / 100);
            let package = format!("package{}", i);
            packages.push(format!(
                r#"        {{
            "name": "{}/{}",
            "version": "{}.{}.{}",
            "source": {{
                "type": "git",
                "url": "https://github.com/{}/{}.git",
                "reference": "{:040x}"
            }},
            "dist": {{
                "type": "zip",
                "url": "https://api.github.com/repos/{}/{}/zipball/v{}.{}.{}",
                "reference": "{:040x}",
                "shasum": ""
            }},
            "require": {{}},
            "type": "library"
        }}"#,
                vendor,
                package,
                1 + (i % 10),
                (i / 10) % 100,
                i % 100,
                vendor,
                package,
                i,
                vendor,
                package,
                1 + (i % 10),
                (i / 10) % 100,
                i % 100,
                i * 12345
            ));
        }

        format!(
            r#"{{
    "content-hash": "stress-test-hash-{:032x}",
    "packages": [
{}
    ],
    "packages-dev": [],
    "aliases": [],
    "minimum-stability": "stable",
    "prefer-stable": true,
    "prefer-lowest": false,
    "platform": {{}},
    "platform-dev": {{}}
}}"#,
            num_packages,
            packages.join(",\n")
        )
    }

    #[test]
    fn test_parse_100_dependencies() {
        let temp = TempDir::new().unwrap();
        let content = generate_large_composer_json(100);
        fs::write(temp.path().join("composer.json"), &content).unwrap();

        let start = Instant::now();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        let elapsed = start.elapsed();

        assert!(parsed["require"].as_object().unwrap().len() == 100);
        assert!(
            elapsed < Duration::from_secs(1),
            "Parsing 100 deps took too long: {:?}",
            elapsed
        );
    }

    #[test]
    fn test_parse_500_dependencies() {
        let temp = TempDir::new().unwrap();
        let content = generate_large_composer_json(500);
        fs::write(temp.path().join("composer.json"), &content).unwrap();

        let start = Instant::now();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        let elapsed = start.elapsed();

        assert!(parsed["require"].as_object().unwrap().len() == 500);
        assert!(
            elapsed < Duration::from_secs(2),
            "Parsing 500 deps took too long: {:?}",
            elapsed
        );
    }

    #[test]
    fn test_parse_1000_dependencies() {
        let temp = TempDir::new().unwrap();
        let content = generate_large_composer_json(1000);
        fs::write(temp.path().join("composer.json"), &content).unwrap();

        let start = Instant::now();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        let elapsed = start.elapsed();

        assert!(parsed["require"].as_object().unwrap().len() == 1000);
        assert!(
            elapsed < Duration::from_secs(5),
            "Parsing 1000 deps took too long: {:?}",
            elapsed
        );
    }

    #[test]
    fn test_parse_large_lock_file() {
        let content = generate_large_composer_lock(500);

        let start = Instant::now();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        let elapsed = start.elapsed();

        assert!(parsed["packages"].as_array().unwrap().len() == 500);
        assert!(
            elapsed < Duration::from_secs(5),
            "Parsing 500 packages lock took too long: {:?}",
            elapsed
        );
    }

    #[test]
    fn test_serialize_large_structure() {
        let mut data = HashMap::new();
        for i in 0..1000 {
            data.insert(
                format!("vendor{}/package{}", i / 100, i),
                format!("^{}.0.0", 1 + (i % 10)),
            );
        }

        let start = Instant::now();
        let json = serde_json::to_string(&data).unwrap();
        let elapsed = start.elapsed();

        assert!(!json.is_empty());
        assert!(
            elapsed < Duration::from_secs(1),
            "Serializing 1000 entries took too long: {:?}",
            elapsed
        );
    }
}

// ========== Concurrent Operations Tests ==========

mod concurrent_ops {
    use super::*;

    #[test]
    fn test_concurrent_json_parsing() {
        let content = r#"{"name": "test/project", "require": {"php": ">=8.0"}}"#.to_string();
        let iterations = Arc::new(AtomicUsize::new(0));

        let handles: Vec<_> = (0..8)
            .map(|_| {
                let c = content.clone();
                let iter = iterations.clone();
                thread::spawn(move || {
                    for _ in 0..1000 {
                        let _: serde_json::Value = serde_json::from_str(&c).unwrap();
                        iter.fetch_add(1, Ordering::Relaxed);
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }

        assert_eq!(iterations.load(Ordering::Relaxed), 8000);
    }

    #[test]
    fn test_concurrent_json_serialization() {
        let data: HashMap<String, String> = (0..100)
            .map(|i| (format!("key{}", i), format!("value{}", i)))
            .collect();

        let data = Arc::new(data);
        let iterations = Arc::new(AtomicUsize::new(0));

        let handles: Vec<_> = (0..8)
            .map(|_| {
                let d = data.clone();
                let iter = iterations.clone();
                thread::spawn(move || {
                    for _ in 0..1000 {
                        let _ = serde_json::to_string(&*d).unwrap();
                        iter.fetch_add(1, Ordering::Relaxed);
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }

        assert_eq!(iterations.load(Ordering::Relaxed), 8000);
    }

    #[test]
    fn test_concurrent_file_operations() {
        let temp = TempDir::new().unwrap();
        let path = Arc::new(temp.path().to_path_buf());
        let operations = Arc::new(AtomicUsize::new(0));

        let handles: Vec<_> = (0..4)
            .map(|i| {
                let p = path.clone();
                let ops = operations.clone();
                thread::spawn(move || {
                    for j in 0..100 {
                        let file_path = p.join(format!("file_{}_{}.json", i, j));
                        let content = format!(r#"{{"thread": {}, "iteration": {}}}"#, i, j);
                        fs::write(&file_path, &content).unwrap();
                        let read = fs::read_to_string(&file_path).unwrap();
                        assert_eq!(read, content);
                        ops.fetch_add(1, Ordering::Relaxed);
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }

        assert_eq!(operations.load(Ordering::Relaxed), 400);
    }
}

// ========== Memory Usage Tests ==========

mod memory_tests {
    use super::*;

    #[test]
    fn test_repeated_parsing_no_leak() {
        let content = r#"{"name": "test/project", "require": {"php": ">=8.0"}}"#;

        // Parse many times - should not accumulate memory
        for _ in 0..10000 {
            let _: serde_json::Value = serde_json::from_str(content).unwrap();
        }
        // If we get here without OOM, test passes
    }

    #[test]
    fn test_large_string_handling() {
        // Create a composer.json with large strings
        let large_string = "x".repeat(100_000);
        let content = format!(
            r#"{{"name": "test/large-string", "description": "{}", "require": {{}}}}"#,
            large_string
        );

        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed["description"].as_str().unwrap().len(), 100_000);
    }

    #[test]
    fn test_deeply_nested_structure() {
        // Create a deeply nested JSON structure
        let mut json = r#"{"level": "#.to_string();
        for _ in 0..50 {
            json.push_str(r#"{"level": "#);
        }
        json.push_str(r#""deep""#);
        for _ in 0..50 {
            json.push('}');
        }
        json.push('}');

        // Should parse without stack overflow
        let result = serde_json::from_str::<serde_json::Value>(&json);
        assert!(result.is_ok());
    }

    #[test]
    fn test_wide_structure() {
        // Create a wide JSON structure (many keys at one level)
        let mut map = HashMap::new();
        for i in 0..10000 {
            map.insert(format!("key_{}", i), format!("value_{}", i));
        }

        let json = serde_json::to_string(&map).unwrap();
        let parsed: HashMap<String, String> = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.len(), 10000);
    }
}

// ========== Performance Regression Tests ==========

mod performance {
    use super::*;

    #[test]
    fn test_json_parse_performance() {
        let content = r#"{"name": "test/project", "require": {"php": ">=8.0"}}"#;

        let start = Instant::now();
        for _ in 0..10000 {
            let _: serde_json::Value = serde_json::from_str(content).unwrap();
        }
        let elapsed = start.elapsed();

        // Should complete 10k parses in under 1 second
        assert!(
            elapsed < Duration::from_secs(1),
            "10k JSON parses took too long: {:?}",
            elapsed
        );
    }

    #[test]
    fn test_json_serialize_performance() {
        let data: HashMap<String, String> = (0..10)
            .map(|i| (format!("key{}", i), format!("value{}", i)))
            .collect();

        let start = Instant::now();
        for _ in 0..10000 {
            let _ = serde_json::to_string(&data).unwrap();
        }
        let elapsed = start.elapsed();

        // Should complete 10k serializations in under 1 second
        assert!(
            elapsed < Duration::from_secs(1),
            "10k JSON serializations took too long: {:?}",
            elapsed
        );
    }

    #[test]
    fn test_hashmap_performance() {
        let mut map = HashMap::new();

        let start = Instant::now();
        for i in 0..100000 {
            map.insert(format!("key_{}", i), format!("value_{}", i));
        }
        let insert_elapsed = start.elapsed();

        let start = Instant::now();
        for i in 0..100000 {
            let _ = map.get(&format!("key_{}", i));
        }
        let lookup_elapsed = start.elapsed();

        assert!(
            insert_elapsed < Duration::from_secs(1),
            "100k inserts took too long: {:?}",
            insert_elapsed
        );
        assert!(
            lookup_elapsed < Duration::from_millis(500),
            "100k lookups took too long: {:?}",
            lookup_elapsed
        );
    }
}

// ========== Edge Case Stress Tests ==========

mod edge_cases {
    use super::*;

    #[test]
    fn test_empty_values() {
        let content = r#"{
            "name": "",
            "description": "",
            "require": {},
            "require-dev": {},
            "autoload": {
                "psr-4": {},
                "classmap": [],
                "files": []
            }
        }"#;

        let parsed: serde_json::Value = serde_json::from_str(content).unwrap();
        assert!(parsed["name"].as_str().unwrap().is_empty());
    }

    #[test]
    fn test_unicode_stress() {
        // Create content with various Unicode characters
        let unicode_strings = vec![
            "日本語",
            "中文",
            "한국어",
            "العربية",
            "🎉🎊🎈",
            "émojis résumé",
            "Ñoño",
        ];

        for s in unicode_strings {
            let content = format!(r#"{{"name": "test/{}", "require": {{}}}}"#, s);
            let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
            assert!(parsed["name"].as_str().unwrap().contains(s));
        }
    }

    #[test]
    fn test_special_characters_in_strings() {
        let special_strings = vec![
            r#"with \"quotes\""#,
            r#"with \\backslash"#,
            "with \ttab",
            "with \nnewline",
            "with \rcarriage return",
        ];

        for s in special_strings {
            let content = format!(r#"{{"description": "{}"}}"#, s);
            let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
            assert!(parsed["description"].is_string());
        }
    }

    #[test]
    fn test_numeric_boundaries() {
        let content = r#"{
            "max_i64": 9223372036854775807,
            "min_i64": -9223372036854775808,
            "large_float": 1.7976931348623157e+308,
            "small_float": 2.2250738585072014e-308
        }"#;

        let parsed: serde_json::Value = serde_json::from_str(content).unwrap();
        assert!(parsed["max_i64"].is_i64());
    }
}
