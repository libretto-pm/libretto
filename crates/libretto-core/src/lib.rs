//! Core types and utilities for Libretto package manager.
//!
//! This crate provides foundational types used throughout Libretto:
//! - Package metadata and identifiers
//! - Version constraints and resolution
//! - Content-addressable hashing
//! - High-performance JSON operations
//! - Error types

#![warn(clippy::all)]
#![allow(clippy::module_name_repetitions)]

pub mod error;
mod hash;
mod json;
mod package;
mod platform;
mod version;

pub use error::{Error, Result};
pub use hash::{ContentHash, ContentHasher};
pub use json::{from_json, from_json_slice, to_json, to_json_pretty};
pub use package::{Author, Dependency, Package, PackageId, PackageSource, PackageType};
pub use platform::is_platform_package_name;
pub use version::VersionConstraint;

// Re-export commonly used types
pub use ahash::{AHashMap, AHashSet};
pub use dashmap::DashMap;
pub use parking_lot::{Mutex, RwLock};
pub use semver::Version;

/// Global allocator using mimalloc for high performance.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;
