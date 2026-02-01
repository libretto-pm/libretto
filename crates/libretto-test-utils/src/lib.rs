//! Comprehensive testing utilities for Libretto.
//!
//! This crate provides test helpers, fixtures, generators, and assertions
//! for testing all Libretto components.
//!
//! # Modules
//!
//! - [`fixtures`]: Pre-built test fixtures for common scenarios
//! - [`generators`]: Random data generators for property-based testing
//! - [`assertions`]: Custom assertion helpers for domain-specific checks
//! - [`mock_server`]: HTTP mock server utilities for testing API interactions
//! - [`temp_project`]: Temporary project creation and management
//! - [`git_utils`]: Git repository test utilities
//! - [`proptest_strategies`]: Proptest strategies for Libretto types
//!
//! # Example
//!
//! ```rust,no_run
//! use libretto_test_utils::temp_project::TempProject;
//! use libretto_test_utils::fixtures::Fixtures;
//!
//! #[tokio::test]
//! async fn test_install() {
//!     let project = TempProject::new()
//!         .with_composer_json(Fixtures::laravel_composer_json())
//!         .build()
//!         .await
//!         .unwrap();
//!
//!     // Run install command
//!     // Assert package installation
//! }
//! ```

#![warn(clippy::all)]
#![allow(clippy::module_name_repetitions)]

pub mod assertions;
pub mod fixtures;
pub mod generators;
pub mod git_utils;
pub mod mock_server;
pub mod proptest_strategies;
pub mod temp_project;

/// Re-export commonly used testing utilities.
pub mod prelude {
    pub use crate::assertions::*;
    pub use crate::fixtures::Fixtures;
    pub use crate::generators::*;
    pub use crate::mock_server::MockPackagist;
    pub use crate::temp_project::TempProject;

    // Re-export common testing crates
    pub use insta::{assert_json_snapshot, assert_snapshot};
    pub use pretty_assertions::{assert_eq, assert_ne};
    pub use proptest::prelude::*;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_module_exports() {
        // Verify all modules are accessible
        let _ = fixtures::Fixtures::empty_composer_json();
    }
}
