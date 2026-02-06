//! Core types for dependency resolution.
//!
//! This module contains the fundamental types used throughout the resolver:
//! - `Resolution`: The result of a successful resolution
//! - `ResolvedPackage`: A package selected during resolution with its metadata
//! - `ResolveError`: Errors that can occur during resolution

use crate::package::PackageName;
use crate::version::ComposerVersion;
use ahash::AHashMap;
use petgraph::Direction;
use petgraph::graph::{DiGraph, NodeIndex};
use std::time::Duration;
use thiserror::Error;

/// A resolved package with its version and metadata.
#[derive(Debug, Clone)]
pub struct ResolvedPackage {
    /// Package name.
    pub name: PackageName,
    /// Resolved version.
    pub version: ComposerVersion,
    /// Direct dependencies.
    pub dependencies: Vec<PackageName>,
    /// Is this a dev dependency.
    pub is_dev: bool,
    /// Distribution URL for downloading.
    pub dist_url: Option<String>,
    /// Distribution type (zip, tar, etc.).
    pub dist_type: Option<String>,
    /// Distribution checksum.
    pub dist_shasum: Option<String>,
    /// Source URL (git repository).
    pub source_url: Option<String>,
    /// Source type (git, hg, etc.).
    pub source_type: Option<String>,
    /// Source reference (commit/tag).
    pub source_reference: Option<String>,
    /// Package require dependencies (for lock file).
    pub require: Option<Vec<(String, String)>>,
    /// Package require-dev dependencies (for lock file).
    pub require_dev: Option<Vec<(String, String)>>,
    /// Package suggest (for lock file).
    pub suggest: Option<Vec<(String, String)>>,
    /// Packages this package provides.
    pub provide: Option<Vec<(String, String)>>,
    /// Packages this package replaces.
    pub replace: Option<Vec<(String, String)>>,
    /// Packages this package conflicts with.
    pub conflict: Option<Vec<(String, String)>>,
    /// Package type (library, project, etc.).
    pub package_type: Option<String>,
    /// Package description.
    pub description: Option<String>,
    /// Package homepage URL.
    pub homepage: Option<String>,
    /// Package licenses.
    pub license: Option<Vec<String>>,
    /// Package authors.
    pub authors: Option<sonic_rs::Value>,
    /// Package keywords.
    pub keywords: Option<Vec<String>>,
    /// Release time.
    pub time: Option<String>,
    /// Autoload configuration.
    pub autoload: Option<sonic_rs::Value>,
    /// Autoload-dev configuration.
    pub autoload_dev: Option<sonic_rs::Value>,
    /// Extra metadata.
    pub extra: Option<sonic_rs::Value>,
    /// Support links.
    pub support: Option<sonic_rs::Value>,
    /// Funding links.
    pub funding: Option<sonic_rs::Value>,
    /// Notification URL.
    pub notification_url: Option<String>,
    /// Binary files.
    pub bin: Option<Vec<String>>,
}

/// Result of dependency resolution.
#[derive(Debug)]
pub struct Resolution {
    /// Resolved packages in topological order (dependencies first).
    pub packages: Vec<ResolvedPackage>,
    /// Dependency graph.
    pub graph: DiGraph<PackageName, ()>,
    /// Node indices by package name.
    pub indices: AHashMap<String, NodeIndex>,
    /// Platform packages encountered.
    pub platform_packages: Vec<String>,
    /// Resolution time.
    pub duration: Duration,
}

impl Resolution {
    /// Get the number of resolved packages.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.packages.len()
    }

    /// Check if resolution is empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.packages.is_empty()
    }

    /// Get a resolved package by name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&ResolvedPackage> {
        self.packages.iter().find(|p| p.name.as_str() == name)
    }

    /// Check if a package is in the resolution.
    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.indices.contains_key(name)
    }

    /// Get packages that depend on the given package.
    #[must_use]
    pub fn dependents(&self, name: &str) -> Vec<&ResolvedPackage> {
        let idx = match self.indices.get(name) {
            Some(i) => *i,
            None => return vec![],
        };

        self.graph
            .neighbors_directed(idx, Direction::Outgoing)
            .filter_map(|n| {
                let pkg_name = self.graph.node_weight(n)?;
                self.get(pkg_name.as_str())
            })
            .collect()
    }
}

/// Errors that can occur during dependency resolution.
#[derive(Debug, Error)]
pub enum ResolveError {
    /// Version conflict between packages.
    #[error("dependency conflict:\n{explanation}")]
    Conflict {
        /// Human-readable explanation of the conflict.
        explanation: String,
    },

    /// Package not found in any repository.
    #[error("package not found: {name}")]
    PackageNotFound {
        /// Name of the missing package.
        name: String,
    },

    /// Resolution was cancelled (timeout or user request).
    #[error("resolution cancelled")]
    Cancelled,

    /// Invalid constraint format.
    #[error("invalid constraint: {constraint}")]
    InvalidConstraint {
        /// The invalid constraint string.
        constraint: String,
    },

    /// Network or repository error.
    #[error("repository error: {message}")]
    RepositoryError {
        /// Error message.
        message: String,
    },
}
