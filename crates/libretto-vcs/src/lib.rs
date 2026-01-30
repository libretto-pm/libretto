//! High-performance Version Control System operations for Libretto.
//!
//! This crate provides VCS operations optimized for package management:
//!
//! - **Git**: Primary VCS with full feature support
//!   - Shallow clones by default for performance
//!   - Full clones for source installations
//!   - Submodule support
//!   - Sparse checkout for monorepos
//!   - LFS support
//!   - SSH and HTTPS authentication
//!
//! - **SVN, Mercurial, Fossil**: Secondary VCS support via CLI
//!
//! - **Parallel Operations**: Clone 100+ repositories concurrently
//!
//! - **Reference Caching**: Bare repository cache for object sharing
//!
//! # Quick Start
//!
//! ```no_run
//! use libretto_vcs::{VcsManager, VcsRef, CloneOptions};
//!
//! # fn main() -> libretto_vcs::error::Result<()> {
//! // Create a VCS manager
//! let manager = VcsManager::new();
//!
//! // Clone a repository
//! let result = manager.clone(
//!     "https://github.com/symfony/console",
//!     std::path::Path::new("/tmp/console"),
//!     Some(&VcsRef::Tag("v6.0.0".to_string())),
//! )?;
//!
//! println!("Cloned to {:?} at commit {}", result.path, result.commit);
//! # Ok(())
//! # }
//! ```
//!
//! # Parallel Cloning
//!
//! ```no_run
//! use libretto_vcs::{VcsManager, VcsUrl, CloneRequest, BatchCloneBuilder};
//! use std::path::PathBuf;
//!
//! # fn main() -> libretto_vcs::error::Result<()> {
//! let manager = VcsManager::new();
//!
//! // Clone multiple repositories in parallel
//! let result = manager.batch_clone()
//!     .add(VcsUrl::parse("symfony/console")?, PathBuf::from("/tmp/console"))
//!     .add(VcsUrl::parse("symfony/http-kernel")?, PathBuf::from("/tmp/http-kernel"))
//!     .add(VcsUrl::parse("symfony/routing")?, PathBuf::from("/tmp/routing"))
//!     .max_parallel(8)
//!     .execute();
//!
//! println!(
//!     "Cloned {} repositories ({} failed) in {:?}",
//!     result.successful.len(),
//!     result.failed.len(),
//!     result.duration
//! );
//! # Ok(())
//! # }
//! ```
//!
//! # Reference Caching
//!
//! ```no_run
//! use libretto_vcs::VcsManager;
//! use std::path::PathBuf;
//!
//! # fn main() -> libretto_vcs::error::Result<()> {
//! // Create manager with reference cache
//! let manager = VcsManager::with_cache(PathBuf::from("/tmp/vcs-cache"))?;
//!
//! // First clone creates a reference repository
//! manager.clone(
//!     "https://github.com/symfony/console",
//!     std::path::Path::new("/tmp/console1"),
//!     None,
//! )?;
//!
//! // Subsequent clones use object sharing (faster)
//! manager.clone(
//!     "https://github.com/symfony/console",
//!     std::path::Path::new("/tmp/console2"),
//!     None,
//! )?;
//!
//! // Check cache statistics
//! if let Some(stats) = manager.cache_stats() {
//!     println!("Cache: {} repos, {}", stats.count, stats.size_human());
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # Authentication
//!
//! ```no_run
//! use libretto_vcs::{VcsManager, CredentialManager, VcsCredentials};
//! use std::sync::Arc;
//! use std::path::PathBuf;
//!
//! # fn main() -> libretto_vcs::error::Result<()> {
//! let mut creds = CredentialManager::new();
//!
//! // Load from Composer's auth.json
//! creds.load_auth_json(std::path::Path::new("auth.json"))?;
//!
//! // Or add SSH key
//! creds.add_ssh_key(PathBuf::from("~/.ssh/id_ed25519"));
//!
//! let manager = VcsManager::new()
//!     .with_credentials(Arc::new(creds));
//!
//! // Clone private repository
//! manager.clone(
//!     "git@github.com:private/repo.git",
//!     std::path::Path::new("/tmp/private"),
//!     None,
//! )?;
//! # Ok(())
//! # }
//! ```
//!
//! # Module Structure
//!
//! - [`error`]: Error types for VCS operations
//! - [`types`]: Core types (`VcsType`, `VcsRef`, `CloneOptions`)
//! - [`url`]: URL parsing with protocol detection
//! - [`credentials`]: Authentication handling
//! - [`git`]: Git repository operations
//! - [`svn`]: SVN operations
//! - [`hg`]: Mercurial operations
//! - [`fossil`]: Fossil operations
//! - [`parallel`]: Parallel clone execution
//! - [`cache`]: Reference repository caching
//! - [`manager`]: Unified VCS manager

#![warn(clippy::all)]
#![allow(clippy::module_name_repetitions)]

pub mod cache;
pub mod credentials;
pub mod error;
pub mod fossil;
pub mod git;
pub mod hg;
pub mod manager;
pub mod parallel;
pub mod perforce;
pub mod svn;
pub mod types;
pub mod url;

// Re-export main types at crate root
pub use cache::{AlternatesManager, ReferenceCache};
pub use credentials::{CredentialManager, KnownHost};
pub use error::{Result, VcsError};
pub use git::{GitCloneBuilder, GitRepository, SubmoduleInfo};
pub use manager::{CacheStats, Repository, VcsManager};
pub use parallel::{BatchCloneBuilder, CloneRequest, ParallelCloneResult, ParallelCloner};
pub use types::{CloneOptions, CloneResult, RepoStatus, VcsCredentials, VcsRef, VcsType};
pub use url::{GitHosting, GitProtocol, VcsUrl};

// Re-export other VCS types
pub use fossil::FossilRepository;
pub use hg::HgRepository;
pub use perforce::PerforceRepository;
pub use svn::SvnRepository;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_exports() {
        // Ensure main types are accessible
        let _: VcsType = VcsType::Git;
        let _: VcsRef = VcsRef::Default;
        let _ = CloneOptions::default();
        let _ = VcsManager::new();
    }

    #[test]
    fn vcs_ref_parse() {
        assert!(matches!(VcsRef::parse("main"), VcsRef::Branch(_)));
        assert!(matches!(VcsRef::parse("v1.0.0"), VcsRef::Tag(_)));
        assert!(matches!(
            VcsRef::parse("abc123def456abc123def456abc123def456abcd"),
            VcsRef::Commit(_)
        ));
    }

    #[test]
    fn vcs_url_parse() {
        let url = VcsUrl::parse("https://github.com/owner/repo.git").unwrap();
        assert_eq!(url.protocol, GitProtocol::Https);
        assert_eq!(url.owner, Some("owner".to_string()));
        assert_eq!(url.repo, Some("repo".to_string()));
    }

    #[test]
    fn vcs_type_detect() {
        let temp = tempfile::tempdir().unwrap();
        assert!(VcsType::detect(temp.path()).is_none());

        std::fs::create_dir(temp.path().join(".git")).unwrap();
        assert_eq!(VcsType::detect(temp.path()), Some(VcsType::Git));
    }
}
