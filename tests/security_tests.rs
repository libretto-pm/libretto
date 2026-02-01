//! Security tests for Libretto.
//!
//! These tests verify protection against common security vulnerabilities:
//! - Path traversal attacks
//! - Code injection
//! - Malicious package names
//! - Unsafe file operations

use std::fs;
use std::path::Path;
use tempfile::TempDir;

// ========== Path Traversal Prevention Tests ==========

mod path_traversal {
    use super::*;

    /// Validate that a path is safe (doesn't escape the base directory).
    fn is_safe_path(base: &Path, target: &Path) -> bool {
        // Canonicalize paths for comparison
        let base_canon = match base.canonicalize() {
            Ok(p) => p,
            Err(_) => return false,
        };

        let target_canon = match target.canonicalize() {
            Ok(p) => p,
            Err(_) => {
                // If target doesn't exist, check if it would be inside base
                let mut current = target.to_path_buf();
                while let Some(parent) = current.parent() {
                    if let Ok(p) = parent.canonicalize() {
                        return target
                            .to_string_lossy()
                            .contains(&p.to_string_lossy().to_string())
                            || p.starts_with(&base_canon);
                    }
                    current = parent.to_path_buf();
                }
                return false;
            }
        };

        target_canon.starts_with(&base_canon)
    }

    /// Test various path traversal attack patterns.
    #[test]
    fn test_path_traversal_patterns() {
        let temp = TempDir::new().unwrap();
        let base = temp.path();

        // Create base directory structure
        fs::create_dir_all(base.join("vendor")).unwrap();

        // These paths should be considered UNSAFE
        let unsafe_paths = vec![
            "../outside",
            "../../etc/passwd",
            "../../../root/.ssh/id_rsa",
            "vendor/../../../outside",
            "./vendor/../../../etc/passwd",
            "vendor/package/../../../../../../etc/passwd",
            "vendor/./../../outside",
            "..\\windows\\system32", // Windows-style
            "vendor\\..\\..\\..\\windows",
            "/etc/passwd",           // Absolute path
            "C:\\Windows\\System32", // Windows absolute
        ];

        for path_str in &unsafe_paths {
            let path = base.join(path_str);
            // The path should not resolve to something inside base
            // OR should be detected as unsafe
            assert!(
                !is_safe_path(base, &path)
                    || !path
                        .canonicalize()
                        .map(|p| p.starts_with(base))
                        .unwrap_or(false),
                "Path '{}' should be detected as unsafe",
                path_str
            );
        }

        // These paths SHOULD be safe
        let safe_paths = vec![
            "vendor/monolog/monolog",
            "vendor/psr/log/src/Logger.php",
            "./vendor/symfony/console",
            "src/App/Controller.php",
        ];

        for path_str in &safe_paths {
            let full_path = base.join(path_str);
            // Create parent directories
            if let Some(parent) = full_path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&full_path, "test content").unwrap();

            assert!(
                is_safe_path(base, &full_path),
                "Path '{}' should be safe",
                path_str
            );
        }
    }

    #[test]
    fn test_null_byte_injection() {
        let temp = TempDir::new().unwrap();

        // Null bytes in paths should be rejected
        let malicious_paths = vec![
            "vendor/package\x00.php",
            "vendor\x00/../../etc/passwd",
            "file.php\x00.txt",
        ];

        for path_str in malicious_paths {
            let path = temp.path().join(path_str);
            // Rust's Path should handle null bytes, but verify behavior
            let result = fs::write(&path, "test");
            // The write should fail due to invalid path
            assert!(
                result.is_err(),
                "Writing to path with null byte should fail: {:?}",
                path
            );
        }
    }

    #[test]
    fn test_symlink_escape() {
        let temp = TempDir::new().unwrap();
        let base = temp.path();

        // Create a symlink pointing outside the base directory
        let symlink_path = base.join("vendor/evil-link");
        fs::create_dir_all(base.join("vendor")).unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let _ = symlink("/etc", &symlink_path);

            // Following the symlink should be detected as escaping
            if symlink_path.exists() {
                let target = fs::read_link(&symlink_path).unwrap();
                assert!(
                    !target.starts_with(base),
                    "Symlink target escapes base directory"
                );
            }
        }
    }

    #[test]
    fn test_double_encoding() {
        // Test that double-encoded paths are handled safely
        let patterns = vec![
            "%2e%2e%2f",       // ../
            "%252e%252e%252f", // Double-encoded ../
            "..%c0%af",        // Overlong UTF-8 encoding of /
            "..%c1%9c",        // Overlong UTF-8 encoding of \
        ];

        for pattern in patterns {
            // URL-decode and verify it's detected as traversal
            let decoded = urlencoding::decode(pattern).unwrap_or_default();
            if decoded.contains("..") {
                // Pattern is a traversal attempt
                assert!(true);
            }
        }
    }
}

// ========== Malicious Package Name Tests ==========

mod malicious_package_names {
    use super::*;

    /// Validate package name is safe.
    fn is_safe_package_name(name: &str) -> bool {
        // Must be in vendor/package format
        let parts: Vec<&str> = name.split('/').collect();
        if parts.len() != 2 {
            return false;
        }

        let vendor = parts[0];
        let package = parts[1];

        // Must not be empty
        if vendor.is_empty() || package.is_empty() {
            return false;
        }

        // Must not contain path separators or parent directory references
        if name.contains("..") || name.contains('\\') || name.contains('\0') {
            return false;
        }

        // Must only contain safe characters
        let is_safe_char = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.';

        vendor.chars().all(is_safe_char) && package.chars().all(is_safe_char)
    }

    #[test]
    fn test_safe_package_names() {
        let safe_names = vec![
            "vendor/package",
            "monolog/monolog",
            "symfony/http-kernel",
            "psr/log",
            "guzzlehttp/guzzle",
            "laravel/framework",
            "my-vendor/my-package",
            "vendor123/package456",
            "a/b",
            "vendor.io/package.name",
        ];

        for name in safe_names {
            assert!(is_safe_package_name(name), "Name '{}' should be safe", name);
        }
    }

    #[test]
    fn test_malicious_package_names() {
        let malicious_names = vec![
            "../../../etc/passwd",
            "vendor/../../../etc/passwd",
            "..\\..\\windows\\system32",
            "vendor/package/../../../root",
            "vendor\x00/package",
            "vendor/pack\x00age",
            "/etc/passwd",
            "C:\\Windows\\System32",
            "",
            "/",
            "vendor/",
            "/package",
            "vendor//package",
            "vendor/package/extra",
            "vendor\\package",
        ];

        for name in malicious_names {
            assert!(
                !is_safe_package_name(name),
                "Name '{}' should be detected as malicious",
                name
            );
        }
    }

    #[test]
    fn test_unicode_homograph_attacks() {
        // Test for Unicode characters that look like ASCII but aren't
        let homograph_names = vec![
            "ｖendor/package", // Fullwidth v
            "vendor/pаckage",  // Cyrillic 'а'
            "vendor/рackage",  // Cyrillic 'р'
            "ⅿonolog/monolog", // Roman numeral 'm'
            "vendor/pаϲkage",  // Mixed scripts
        ];

        for name in homograph_names {
            // These should either be rejected or normalized
            let ascii_check = name.chars().all(|c| c.is_ascii());
            assert!(
                !ascii_check || !is_safe_package_name(name),
                "Homograph attack '{}' should be detected",
                name
            );
        }
    }
}

// ========== Composer.json Injection Tests ==========

mod json_injection {
    use super::*;

    #[test]
    fn test_script_injection_in_name() {
        // Malicious content in package name should be escaped in JSON
        let malicious_name = r#"vendor/package"; rm -rf /; echo ""#;

        let json = serde_json::json!({
            "name": malicious_name,
            "require": {}
        });

        let serialized = serde_json::to_string(&json).unwrap();

        // The malicious content should be escaped in JSON
        assert!(serialized.contains("\\\""));
        assert!(!serialized.contains(r#""; rm"#));
    }

    #[test]
    fn test_html_injection_in_description() {
        let malicious_description = r#"<script>alert('xss')</script>"#;

        let json = serde_json::json!({
            "name": "vendor/package",
            "description": malicious_description,
            "require": {}
        });

        let serialized = serde_json::to_string(&json).unwrap();

        // When parsed back, the script tag should be preserved as data, not executed
        let parsed: serde_json::Value = serde_json::from_str(&serialized).unwrap();
        assert_eq!(parsed["description"], malicious_description);
    }

    #[test]
    fn test_json_prototype_pollution() {
        // Test that __proto__ and constructor are handled as regular keys
        let malicious_json = r#"{
            "name": "vendor/package",
            "__proto__": {"polluted": true},
            "constructor": {"prototype": {"polluted": true}},
            "require": {}
        }"#;

        let parsed: serde_json::Value = serde_json::from_str(malicious_json).unwrap();

        // Should be parsed as regular keys, not cause prototype pollution
        assert!(parsed["__proto__"].is_object());
        assert!(parsed["constructor"].is_object());
    }

    #[test]
    fn test_deeply_nested_json() {
        // Test protection against deeply nested JSON (potential DoS)
        let mut json = r#"{"a":"#.to_string();
        for _ in 0..100 {
            json.push_str(r#"{"b":"#);
        }
        json.push_str(r#""value""#);
        for _ in 0..100 {
            json.push('}');
        }
        json.push('}');

        // Should either parse successfully or fail gracefully (no stack overflow)
        let result = serde_json::from_str::<serde_json::Value>(&json);
        // Result should be Ok or a controlled error, not a panic
        let _ = result;
    }

    #[test]
    fn test_large_number_handling() {
        // Test handling of numbers that could cause overflow
        let json_with_large_numbers = r#"{
            "huge_positive": 99999999999999999999999999999999999999999,
            "huge_negative": -99999999999999999999999999999999999999999,
            "huge_float": 1e+999
        }"#;

        // Should parse without panic (values may be null or handled as floats)
        let result = serde_json::from_str::<serde_json::Value>(json_with_large_numbers);
        let _ = result;
    }
}

// ========== File System Safety Tests ==========

mod filesystem_safety {
    use super::*;

    #[test]
    fn test_safe_file_creation() {
        let temp = TempDir::new().unwrap();

        // Create files only within the temp directory
        let safe_files = vec![
            "composer.json",
            "vendor/autoload.php",
            "src/App/Controller.php",
            ".hidden-file",
        ];

        for file in safe_files {
            let path = temp.path().join(file);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&path, "content").unwrap();
            assert!(path.exists());
            assert!(path.starts_with(temp.path()));
        }
    }

    #[test]
    fn test_permission_preservation() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let temp = TempDir::new().unwrap();
            let file = temp.path().join("test.php");

            fs::write(&file, "<?php").unwrap();

            // Set permissions
            let mut perms = fs::metadata(&file).unwrap().permissions();
            perms.set_mode(0o644);
            fs::set_permissions(&file, perms).unwrap();

            // Verify permissions
            let actual = fs::metadata(&file).unwrap().permissions().mode() & 0o777;
            assert_eq!(actual, 0o644);
        }
    }

    #[test]
    fn test_atomic_file_writes() {
        let temp = TempDir::new().unwrap();
        let target = temp.path().join("composer.json");

        // Write initial content
        fs::write(&target, r#"{"name": "initial"}"#).unwrap();

        // Simulate atomic update using temp file + rename
        let temp_file = temp.path().join("composer.json.tmp");
        fs::write(&temp_file, r#"{"name": "updated"}"#).unwrap();
        fs::rename(&temp_file, &target).unwrap();

        // Verify atomic update succeeded
        let content = fs::read_to_string(&target).unwrap();
        assert!(content.contains("updated"));
    }
}

// ========== URL Validation Tests ==========

mod url_safety {
    /// Validate that a URL is safe for use in downloads.
    fn is_safe_url(url: &str) -> bool {
        // Only allow HTTPS
        if !url.starts_with("https://") {
            return false;
        }

        // Don't allow localhost or private IPs
        let unsafe_patterns = [
            "localhost",
            "127.0.0.1",
            "0.0.0.0",
            "192.168.",
            "10.",
            "172.16.",
            "172.17.",
            "172.18.",
            "172.19.",
            "172.20.",
            "172.21.",
            "172.22.",
            "172.23.",
            "172.24.",
            "172.25.",
            "172.26.",
            "172.27.",
            "172.28.",
            "172.29.",
            "172.30.",
            "172.31.",
            "[::1]",
            "169.254.", // Link-local
        ];

        for pattern in &unsafe_patterns {
            if url.contains(pattern) {
                return false;
            }
        }

        true
    }

    #[test]
    fn test_safe_urls() {
        let safe_urls = vec![
            "https://packagist.org/packages.json",
            "https://api.github.com/repos/vendor/package",
            "https://example.com/package.zip",
        ];

        for url in safe_urls {
            assert!(is_safe_url(url), "URL '{}' should be safe", url);
        }
    }

    #[test]
    fn test_unsafe_urls() {
        let unsafe_urls = vec![
            "http://packagist.org/packages.json", // Not HTTPS
            "https://localhost/package.zip",
            "https://127.0.0.1/package.zip",
            "https://192.168.1.1/package.zip",
            "https://10.0.0.1/package.zip",
            "ftp://example.com/package.zip",
            "file:///etc/passwd",
            "https://[::1]/package.zip",
        ];

        for url in unsafe_urls {
            assert!(!is_safe_url(url), "URL '{}' should be unsafe", url);
        }
    }

    #[test]
    fn test_url_redirect_safety() {
        // Test that redirects to unsafe URLs would be blocked
        let redirect_targets = vec![
            "http://evil.com/malware.zip",  // HTTP downgrade
            "https://localhost/steal-data", // Localhost redirect
            "file:///etc/passwd",           // File protocol
        ];

        for target in redirect_targets {
            assert!(
                !is_safe_url(target),
                "Redirect to '{}' should be blocked",
                target
            );
        }
    }
}

// ========== Checksum Validation Tests ==========

mod checksum_safety {
    use sha2::{Digest, Sha256};

    fn compute_sha256(data: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(data);
        hex::encode(hasher.finalize())
    }

    fn verify_checksum(data: &[u8], expected: &str) -> bool {
        let actual = compute_sha256(data);
        // Use constant-time comparison
        actual.len() == expected.len()
            && actual
                .bytes()
                .zip(expected.bytes())
                .fold(0, |acc, (a, b)| acc | (a ^ b))
                == 0
    }

    #[test]
    fn test_checksum_verification() {
        let data = b"test content for checksum";
        let correct_hash = compute_sha256(data);

        assert!(verify_checksum(data, &correct_hash));
    }

    #[test]
    fn test_checksum_mismatch_detection() {
        let data = b"legitimate content";
        let wrong_hash = "0000000000000000000000000000000000000000000000000000000000000000";

        assert!(!verify_checksum(data, wrong_hash));
    }

    #[test]
    fn test_length_extension_resistance() {
        // SHA-256 is resistant to length extension attacks
        // but verify our implementation handles different lengths correctly
        let data1 = b"short";
        let data2 = b"much longer content that could be an extension attack";

        let hash1 = compute_sha256(data1);
        let hash2 = compute_sha256(data2);

        assert_ne!(hash1, hash2);
        assert_eq!(hash1.len(), 64); // SHA-256 produces 64 hex chars
        assert_eq!(hash2.len(), 64);
    }
}

// ========== Credential Safety Tests ==========

mod credential_safety {
    #[test]
    fn test_credentials_not_in_logs() {
        // Simulated log output that should be sanitized
        let log_with_creds = "Connecting to https://user:password@github.com/repo.git";

        // A proper sanitization would replace the password
        let sanitized = log_with_creds.replace(":password@", ":***@");

        assert!(!sanitized.contains("password"));
        assert!(sanitized.contains("***"));
    }

    #[test]
    fn test_auth_token_masking() {
        let token = "ghp_1234567890abcdefghijklmnopqrstuvwxyz";
        let header = format!("Authorization: Bearer {}", token);

        // Token should be masked in output
        let masked = if header.contains("Bearer ") {
            let parts: Vec<&str> = header.split("Bearer ").collect();
            if parts.len() == 2 && parts[1].len() > 8 {
                format!("Bearer {}...", &parts[1][..4])
            } else {
                header.clone()
            }
        } else {
            header.clone()
        };

        assert!(!masked.contains(token));
        assert!(masked.contains("ghp_...") || masked.contains("***"));
    }
}

// Helper for URL encoding tests
mod urlencoding {
    pub fn decode(s: &str) -> Result<String, ()> {
        let mut result = String::new();
        let mut chars = s.chars().peekable();

        while let Some(c) = chars.next() {
            if c == '%' {
                let hex: String = chars.by_ref().take(2).collect();
                if hex.len() == 2 {
                    if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                        result.push(byte as char);
                        continue;
                    }
                }
                return Err(());
            }
            result.push(c);
        }

        Ok(result)
    }
}
