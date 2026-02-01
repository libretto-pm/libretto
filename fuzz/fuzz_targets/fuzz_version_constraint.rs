#![no_main]

use libfuzzer_sys::fuzz_target;
use libretto_core::VersionConstraint;
use semver::Version;

fuzz_target!(|data: &[u8]| {
    // Try to parse as UTF-8 string
    if let Ok(s) = std::str::from_utf8(data) {
        // Create a version constraint from arbitrary input
        let constraint = VersionConstraint::new(s);

        // Test that matches() doesn't panic for various versions
        let test_versions = [
            Version::new(0, 0, 0),
            Version::new(0, 0, 1),
            Version::new(0, 1, 0),
            Version::new(1, 0, 0),
            Version::new(1, 2, 3),
            Version::new(2, 0, 0),
            Version::new(10, 20, 30),
            Version::new(99, 99, 99),
        ];

        for version in &test_versions {
            // Should not panic
            let _ = constraint.matches(version);
        }

        // Test Display trait
        let _ = constraint.to_string();

        // Test as_str()
        let _ = constraint.as_str();

        // Test Clone
        let cloned = constraint.clone();
        let _ = cloned.matches(&Version::new(1, 0, 0));

        // Test PartialEq
        let same = VersionConstraint::new(s);
        let _ = constraint == same;
    }
});
