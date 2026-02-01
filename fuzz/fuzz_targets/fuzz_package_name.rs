#![no_main]

use libfuzzer_sys::fuzz_target;

/// Validate package name format (vendor/package).
fn validate_package_name(name: &str) -> bool {
    // Package name format: vendor/package
    let parts: Vec<&str> = name.split('/').collect();
    if parts.len() != 2 {
        return false;
    }

    let vendor = parts[0];
    let package = parts[1];

    // Check vendor is not empty and starts with letter
    if vendor.is_empty() || !vendor.chars().next().unwrap().is_ascii_alphabetic() {
        return false;
    }

    // Check package is not empty and starts with letter
    if package.is_empty() || !package.chars().next().unwrap().is_ascii_alphabetic() {
        return false;
    }

    // Check characters are valid (alphanumeric, dash, underscore, dot)
    let valid_chars = |s: &str| {
        s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    };

    valid_chars(vendor) && valid_chars(package)
}

/// Normalize package name to lowercase.
fn normalize_package_name(name: &str) -> String {
    name.to_lowercase()
}

/// Parse package name into vendor and package parts.
fn parse_package_name(name: &str) -> Option<(&str, &str)> {
    let parts: Vec<&str> = name.split('/').collect();
    if parts.len() == 2 {
        Some((parts[0], parts[1]))
    } else {
        None
    }
}

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        // Test validation doesn't panic
        let is_valid = validate_package_name(s);

        // Test normalization doesn't panic
        let normalized = normalize_package_name(s);

        // If valid, test parsing
        if is_valid {
            let parsed = parse_package_name(s);
            assert!(
                parsed.is_some(),
                "Valid package name should be parseable: {}",
                s
            );

            // Normalized version should also be valid
            assert!(
                validate_package_name(&normalized),
                "Normalized name should be valid: {}",
                normalized
            );
        }

        // Test parsing doesn't panic even for invalid names
        let _ = parse_package_name(s);

        // Test with trimmed input
        let trimmed = s.trim();
        let _ = validate_package_name(trimmed);
        let _ = normalize_package_name(trimmed);
        let _ = parse_package_name(trimmed);
    }
});
