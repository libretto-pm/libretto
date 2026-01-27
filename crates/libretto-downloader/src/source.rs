//! Package source types and download strategies.
//!
//! Supports multiple source types: dist (archives), git, svn, hg, and local paths.

use crate::config::ExpectedChecksum;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use url::Url;

/// Type of package source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceType {
    /// Distribution archive (ZIP, tar.gz, etc.).
    Dist,
    /// Git repository.
    Git,
    /// Subversion repository.
    Svn,
    /// Mercurial repository.
    Hg,
    /// Local filesystem path.
    Path,
}

impl SourceType {
    /// Parse source type from string.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "dist" | "zip" | "tar" | "archive" => Some(Self::Dist),
            "git" => Some(Self::Git),
            "svn" | "subversion" => Some(Self::Svn),
            "hg" | "mercurial" => Some(Self::Hg),
            "path" | "local" => Some(Self::Path),
            _ => None,
        }
    }

    /// Check if this is a VCS source.
    #[must_use]
    pub const fn is_vcs(&self) -> bool {
        matches!(self, Self::Git | Self::Svn | Self::Hg)
    }

    /// Check if this requires network access.
    #[must_use]
    pub const fn requires_network(&self) -> bool {
        matches!(self, Self::Dist | Self::Git | Self::Svn | Self::Hg)
    }
}

/// Archive type for dist sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArchiveType {
    /// ZIP archive.
    Zip,
    /// Gzipped tarball.
    TarGz,
    /// Bzip2 tarball.
    TarBz2,
    /// XZ tarball.
    TarXz,
    /// Zstd tarball.
    TarZst,
    /// Plain tarball.
    Tar,
}

impl ArchiveType {
    /// Detect archive type from URL or filename.
    #[must_use]
    pub fn from_url(url: &Url) -> Option<Self> {
        let path = url.path().to_lowercase();
        Self::from_extension(&path)
    }

    /// Detect archive type from file extension.
    #[must_use]
    pub fn from_extension(path: &str) -> Option<Self> {
        let lower = path.to_lowercase();
        if lower.ends_with(".zip") {
            Some(Self::Zip)
        } else if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
            Some(Self::TarGz)
        } else if lower.ends_with(".tar.bz2") || lower.ends_with(".tbz2") {
            Some(Self::TarBz2)
        } else if lower.ends_with(".tar.xz") || lower.ends_with(".txz") {
            Some(Self::TarXz)
        } else if lower.ends_with(".tar.zst") || lower.ends_with(".tzst") {
            Some(Self::TarZst)
        } else if lower.ends_with(".tar") {
            Some(Self::Tar)
        } else {
            None
        }
    }

    /// Get the file extension for this archive type.
    #[must_use]
    pub const fn extension(&self) -> &'static str {
        match self {
            Self::Zip => ".zip",
            Self::TarGz => ".tar.gz",
            Self::TarBz2 => ".tar.bz2",
            Self::TarXz => ".tar.xz",
            Self::TarZst => ".tar.zst",
            Self::Tar => ".tar",
        }
    }

    /// Get the MIME type for this archive.
    #[must_use]
    pub const fn mime_type(&self) -> &'static str {
        match self {
            Self::Zip => "application/zip",
            Self::TarGz => "application/gzip",
            Self::TarBz2 => "application/x-bzip2",
            Self::TarXz => "application/x-xz",
            Self::TarZst => "application/zstd",
            Self::Tar => "application/x-tar",
        }
    }
}

/// VCS reference (branch, tag, or commit).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "type", content = "value")]
pub enum VcsRef {
    /// Branch name.
    Branch(String),
    /// Tag name.
    Tag(String),
    /// Commit hash.
    Commit(String),
}

impl VcsRef {
    /// Parse a reference string.
    ///
    /// Commits are detected by 40-char hex strings.
    /// Tags are detected by 'v' prefix followed by digit.
    /// Everything else is treated as a branch.
    #[must_use]
    pub fn parse(reference: &str) -> Self {
        // 40-char hex is a commit SHA
        if reference.len() == 40 && reference.chars().all(|c| c.is_ascii_hexdigit()) {
            return Self::Commit(reference.to_string());
        }

        // v1.0.0 style is likely a tag
        if reference.starts_with('v')
            && reference.chars().nth(1).is_some_and(|c| c.is_ascii_digit())
        {
            return Self::Tag(reference.to_string());
        }

        // Default to branch
        Self::Branch(reference.to_string())
    }

    /// Get the reference string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Branch(s) | Self::Tag(s) | Self::Commit(s) => s,
        }
    }
}

impl Default for VcsRef {
    fn default() -> Self {
        Self::Branch("main".to_string())
    }
}

/// Package download source specification.
#[derive(Debug, Clone)]
pub struct DownloadSource {
    /// Package name (vendor/name format).
    pub name: String,
    /// Package version.
    pub version: String,
    /// Primary source.
    pub primary: Source,
    /// Fallback sources (tried in order if primary fails).
    pub fallbacks: Vec<Source>,
    /// Expected checksums for verification.
    pub checksums: Vec<ExpectedChecksum>,
    /// Destination path.
    pub dest: PathBuf,
}

impl DownloadSource {
    /// Create a new download source.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        version: impl Into<String>,
        primary: Source,
        dest: PathBuf,
    ) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            primary,
            fallbacks: Vec::new(),
            checksums: Vec::new(),
            dest,
        }
    }

    /// Add a fallback source.
    #[must_use]
    pub fn with_fallback(mut self, source: Source) -> Self {
        self.fallbacks.push(source);
        self
    }

    /// Add expected checksums.
    #[must_use]
    pub fn with_checksums(mut self, checksums: Vec<ExpectedChecksum>) -> Self {
        self.checksums = checksums;
        self
    }

    /// Get all sources (primary + fallbacks) in order.
    pub fn all_sources(&self) -> impl Iterator<Item = &Source> {
        std::iter::once(&self.primary).chain(self.fallbacks.iter())
    }

    /// Get unique identifier for this download.
    #[must_use]
    pub fn id(&self) -> String {
        format!("{}@{}", self.name, self.version)
    }
}

/// A single source location.
#[derive(Debug, Clone)]
pub enum Source {
    /// Distribution archive.
    Dist {
        /// Download URL.
        url: Url,
        /// Archive type.
        archive_type: ArchiveType,
    },
    /// Git repository.
    Git {
        /// Repository URL.
        url: Url,
        /// Reference to checkout.
        reference: VcsRef,
    },
    /// Subversion repository.
    Svn {
        /// Repository URL.
        url: Url,
        /// Revision or branch.
        reference: Option<String>,
    },
    /// Mercurial repository.
    Hg {
        /// Repository URL.
        url: Url,
        /// Revision or branch.
        reference: Option<String>,
    },
    /// Local filesystem path.
    Path {
        /// Path to source.
        path: PathBuf,
        /// Whether to symlink instead of copy.
        symlink: bool,
    },
}

impl Source {
    /// Create a dist source from URL.
    #[must_use]
    pub fn dist(url: Url) -> Option<Self> {
        let archive_type = ArchiveType::from_url(&url)?;
        Some(Self::Dist { url, archive_type })
    }

    /// Create a dist source with explicit archive type.
    #[must_use]
    pub fn dist_with_type(url: Url, archive_type: ArchiveType) -> Self {
        Self::Dist { url, archive_type }
    }

    /// Create a git source.
    #[must_use]
    pub fn git(url: Url, reference: impl Into<String>) -> Self {
        Self::Git {
            url,
            reference: VcsRef::parse(&reference.into()),
        }
    }

    /// Create a local path source.
    #[must_use]
    pub fn path(path: PathBuf, symlink: bool) -> Self {
        Self::Path { path, symlink }
    }

    /// Get the source type.
    #[must_use]
    pub const fn source_type(&self) -> SourceType {
        match self {
            Self::Dist { .. } => SourceType::Dist,
            Self::Git { .. } => SourceType::Git,
            Self::Svn { .. } => SourceType::Svn,
            Self::Hg { .. } => SourceType::Hg,
            Self::Path { .. } => SourceType::Path,
        }
    }

    /// Get the URL if this is a network source.
    #[must_use]
    pub fn url(&self) -> Option<&Url> {
        match self {
            Self::Dist { url, .. }
            | Self::Git { url, .. }
            | Self::Svn { url, .. }
            | Self::Hg { url, .. } => Some(url),
            Self::Path { .. } => None,
        }
    }

    /// Check if this source requires network access.
    #[must_use]
    pub const fn requires_network(&self) -> bool {
        self.source_type().requires_network()
    }
}

/// Result of downloading a package.
#[derive(Debug, Clone)]
pub struct DownloadResult {
    /// Package name.
    pub name: String,
    /// Package version.
    pub version: String,
    /// Destination path.
    pub path: PathBuf,
    /// Downloaded size in bytes.
    pub size: u64,
    /// Computed checksums.
    pub checksums: crate::checksum::ComputedChecksums,
    /// Source type used.
    pub source_type: SourceType,
    /// Whether download was resumed.
    pub resumed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_type_detection() {
        let url = Url::parse("https://example.com/pkg.zip").unwrap();
        assert_eq!(ArchiveType::from_url(&url), Some(ArchiveType::Zip));

        let url = Url::parse("https://example.com/pkg.tar.gz").unwrap();
        assert_eq!(ArchiveType::from_url(&url), Some(ArchiveType::TarGz));

        let url = Url::parse("https://example.com/pkg.tar.xz").unwrap();
        assert_eq!(ArchiveType::from_url(&url), Some(ArchiveType::TarXz));
    }

    #[test]
    fn vcs_ref_parsing() {
        assert!(matches!(VcsRef::parse("main"), VcsRef::Branch(_)));
        assert!(matches!(VcsRef::parse("develop"), VcsRef::Branch(_)));
        assert!(matches!(VcsRef::parse("v1.0.0"), VcsRef::Tag(_)));
        assert!(matches!(VcsRef::parse("v2.3.4-beta"), VcsRef::Tag(_)));
        assert!(matches!(
            VcsRef::parse("abc123def456abc123def456abc123def456abcd"),
            VcsRef::Commit(_)
        ));
    }

    #[test]
    fn source_type_properties() {
        assert!(SourceType::Git.is_vcs());
        assert!(SourceType::Svn.is_vcs());
        assert!(SourceType::Hg.is_vcs());
        assert!(!SourceType::Dist.is_vcs());
        assert!(!SourceType::Path.is_vcs());

        assert!(SourceType::Dist.requires_network());
        assert!(!SourceType::Path.requires_network());
    }

    #[test]
    fn download_source_builder() {
        let url = Url::parse("https://example.com/pkg.zip").unwrap();
        let source = DownloadSource::new(
            "vendor/pkg",
            "1.0.0",
            Source::dist(url).unwrap(),
            PathBuf::from("/tmp/pkg"),
        );

        assert_eq!(source.name, "vendor/pkg");
        assert_eq!(source.version, "1.0.0");
        assert_eq!(source.id(), "vendor/pkg@1.0.0");
    }
}
