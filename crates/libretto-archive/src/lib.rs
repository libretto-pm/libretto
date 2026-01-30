//! Archive extraction for Libretto.
//!
//! Supports multiple archive formats:
//! - **ZIP** - Native support via `zip` crate
//! - **TAR** - Native support via `tar` crate
//! - **TAR.GZ** - Native support via `flate2` crate
//! - **TAR.BZ2** - Native support via `bzip2` crate
//! - **TAR.XZ** - Native support via `xz2` crate
//! - **7Z** - CLI support (requires `7z` or `7zz` binary)
//! - **RAR** - CLI support (requires `unrar` binary)

#![warn(clippy::all)]
#![allow(clippy::module_name_repetitions)]

use bzip2::read::BzDecoder;
use flate2::read::GzDecoder;
use libretto_core::{Error, Result};
use std::fs::File;
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::{debug, info, warn};
use walkdir::WalkDir;
use xz2::read::XzDecoder;

/// Supported archive types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveType {
    /// ZIP archive.
    Zip,
    /// Gzipped tarball.
    TarGz,
    /// Plain tarball.
    Tar,
    /// Bzip2 compressed tarball.
    TarBz2,
    /// XZ compressed tarball.
    TarXz,
    /// 7-Zip archive (requires CLI tool).
    SevenZip,
    /// RAR archive (requires CLI tool).
    Rar,
}

impl ArchiveType {
    /// Detect archive type from path extension.
    #[must_use]
    pub fn from_path(path: &Path) -> Option<Self> {
        let name = path.file_name()?.to_str()?;
        Self::from_filename(name)
    }

    /// Detect archive type from filename.
    #[must_use]
    #[allow(clippy::case_sensitive_file_extension_comparisons)] // string is already lowercased
    pub fn from_filename(name: &str) -> Option<Self> {
        let lower = name.to_lowercase();
        if lower.ends_with(".zip") {
            Some(Self::Zip)
        } else if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
            Some(Self::TarGz)
        } else if lower.ends_with(".tar.bz2") || lower.ends_with(".tbz2") || lower.ends_with(".tbz")
        {
            Some(Self::TarBz2)
        } else if lower.ends_with(".tar.xz") || lower.ends_with(".txz") {
            Some(Self::TarXz)
        } else if lower.ends_with(".7z") {
            Some(Self::SevenZip)
        } else if lower.ends_with(".rar") {
            Some(Self::Rar)
        } else if lower.ends_with(".tar") {
            Some(Self::Tar)
        } else {
            None
        }
    }

    /// Get file extension.
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Zip => "zip",
            Self::TarGz => "tar.gz",
            Self::Tar => "tar",
            Self::TarBz2 => "tar.bz2",
            Self::TarXz => "tar.xz",
            Self::SevenZip => "7z",
            Self::Rar => "rar",
        }
    }

    /// Check if this archive type requires an external CLI tool.
    #[must_use]
    pub const fn requires_cli(self) -> bool {
        matches!(self, Self::SevenZip | Self::Rar)
    }

    /// Check if the required CLI tool is available.
    #[must_use]
    pub fn is_tool_available(self) -> bool {
        match self {
            Self::SevenZip => is_7z_available(),
            Self::Rar => is_unrar_available(),
            _ => true, // Native support, always available
        }
    }
}

/// Archive extractor.
#[derive(Debug, Default)]
pub struct Extractor {
    strip_prefix: Option<usize>,
}

impl Extractor {
    /// Create new extractor.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Strip N path components from extracted files.
    #[must_use]
    pub const fn strip_prefix(mut self, components: usize) -> Self {
        self.strip_prefix = Some(components);
        self
    }

    /// Extract archive to directory.
    ///
    /// # Errors
    /// Returns error if extraction fails.
    pub fn extract(&self, archive: &Path, dest: &Path) -> Result<ExtractionResult> {
        let archive_type = ArchiveType::from_path(archive).ok_or_else(|| {
            Error::Archive(format!("unknown archive type: {}", archive.display()))
        })?;

        debug!(archive = ?archive, dest = ?dest, archive_type = ?archive_type, "extracting");

        // Check if CLI tool is available for formats that require it
        if archive_type.requires_cli() && !archive_type.is_tool_available() {
            return Err(Error::Archive(format!(
                "required tool for {} extraction is not installed",
                archive_type.extension()
            )));
        }

        std::fs::create_dir_all(dest).map_err(|e| Error::io(dest, e))?;

        let result = match archive_type {
            ArchiveType::Zip => self.extract_zip(archive, dest)?,
            ArchiveType::TarGz => self.extract_tar_gz(archive, dest)?,
            ArchiveType::Tar => self.extract_tar(archive, dest)?,
            ArchiveType::TarBz2 => self.extract_tar_bz2(archive, dest)?,
            ArchiveType::TarXz => self.extract_tar_xz(archive, dest)?,
            ArchiveType::SevenZip => self.extract_7z(archive, dest)?,
            ArchiveType::Rar => self.extract_rar(archive, dest)?,
        };

        info!(
            files = result.files_extracted,
            size = result.total_size,
            "extraction complete"
        );

        Ok(result)
    }

    fn extract_zip(&self, archive: &Path, dest: &Path) -> Result<ExtractionResult> {
        let file = File::open(archive).map_err(|e| Error::io(archive, e))?;
        let mut zip = zip::ZipArchive::new(file).map_err(|e| Error::Archive(e.to_string()))?;

        let mut files_extracted = 0;
        let mut total_size = 0u64;

        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).map_err(|e| Error::Archive(e.to_string()))?;

            let path = match entry.enclosed_name() {
                Some(p) => p.clone(),
                None => continue,
            };

            let out_path = self.apply_strip_prefix(&path, dest);
            if out_path == dest {
                continue;
            }

            if entry.is_dir() {
                std::fs::create_dir_all(&out_path).map_err(|e| Error::io(&out_path, e))?;
            } else {
                if let Some(parent) = out_path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
                }

                let mut out_file = File::create(&out_path).map_err(|e| Error::io(&out_path, e))?;
                let size = std::io::copy(&mut entry, &mut out_file)
                    .map_err(|e| Error::io(&out_path, e))?;

                files_extracted += 1;
                total_size += size;

                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Some(mode) = entry.unix_mode() {
                        std::fs::set_permissions(&out_path, std::fs::Permissions::from_mode(mode))
                            .ok();
                    }
                }
            }
        }

        Ok(ExtractionResult {
            files_extracted,
            total_size,
            root_dir: find_root_dir(dest),
        })
    }

    fn extract_tar_gz(&self, archive: &Path, dest: &Path) -> Result<ExtractionResult> {
        let file = File::open(archive).map_err(|e| Error::io(archive, e))?;
        let decoder = GzDecoder::new(file);
        self.extract_tar_reader(decoder, dest)
    }

    fn extract_tar(&self, archive: &Path, dest: &Path) -> Result<ExtractionResult> {
        let file = File::open(archive).map_err(|e| Error::io(archive, e))?;
        self.extract_tar_reader(file, dest)
    }

    fn extract_tar_reader<R: Read>(&self, reader: R, dest: &Path) -> Result<ExtractionResult> {
        let mut archive = tar::Archive::new(reader);

        let mut files_extracted = 0;
        let mut total_size = 0u64;

        for entry in archive
            .entries()
            .map_err(|e| Error::Archive(e.to_string()))?
        {
            let mut entry = entry.map_err(|e| Error::Archive(e.to_string()))?;
            let path = entry
                .path()
                .map_err(|e| Error::Archive(e.to_string()))?
                .into_owned();

            let out_path = self.apply_strip_prefix(&path, dest);
            if out_path == dest {
                continue;
            }

            let entry_type = entry.header().entry_type();

            if entry_type.is_dir() {
                std::fs::create_dir_all(&out_path).map_err(|e| Error::io(&out_path, e))?;
            } else if entry_type.is_file() {
                if let Some(parent) = out_path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
                }

                let mut out_file = File::create(&out_path).map_err(|e| Error::io(&out_path, e))?;
                let size = std::io::copy(&mut entry, &mut out_file)
                    .map_err(|e| Error::io(&out_path, e))?;

                files_extracted += 1;
                total_size += size;

                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Ok(mode) = entry.header().mode() {
                        std::fs::set_permissions(&out_path, std::fs::Permissions::from_mode(mode))
                            .ok();
                    }
                }
            }
        }

        Ok(ExtractionResult {
            files_extracted,
            total_size,
            root_dir: find_root_dir(dest),
        })
    }

    fn extract_tar_bz2(&self, archive: &Path, dest: &Path) -> Result<ExtractionResult> {
        let file = File::open(archive).map_err(|e| Error::io(archive, e))?;
        let decoder = BzDecoder::new(file);
        self.extract_tar_reader(decoder, dest)
    }

    fn extract_tar_xz(&self, archive: &Path, dest: &Path) -> Result<ExtractionResult> {
        let file = File::open(archive).map_err(|e| Error::io(archive, e))?;
        let decoder = XzDecoder::new(file);
        self.extract_tar_reader(decoder, dest)
    }

    fn extract_7z(&self, archive: &Path, dest: &Path) -> Result<ExtractionResult> {
        let cmd = find_7z_command().ok_or_else(|| {
            Error::Archive("7z/7zz not found. Install p7zip or 7-zip.".to_string())
        })?;

        debug!(cmd = %cmd, archive = ?archive, dest = ?dest, "extracting 7z");

        let output = Command::new(&cmd)
            .arg("x") // Extract with full paths
            .arg("-y") // Assume yes on all queries
            .arg(format!("-o{}", dest.display())) // Output directory
            .arg(archive)
            .output()
            .map_err(|e| Error::Archive(format!("failed to run {cmd}: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::Archive(format!("7z extraction failed: {stderr}")));
        }

        // Count extracted files
        let (files_extracted, total_size) = count_files_in_dir(dest);

        Ok(ExtractionResult {
            files_extracted,
            total_size,
            root_dir: find_root_dir(dest),
        })
    }

    fn extract_rar(&self, archive: &Path, dest: &Path) -> Result<ExtractionResult> {
        debug!(archive = ?archive, dest = ?dest, "extracting rar");

        let output = Command::new("unrar")
            .arg("x") // Extract with full paths
            .arg("-y") // Assume yes on all queries
            .arg("-o+") // Overwrite existing files
            .arg(archive)
            .arg(dest)
            .output()
            .map_err(|e| Error::Archive(format!("failed to run unrar: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::Archive(format!("unrar extraction failed: {stderr}")));
        }

        // Count extracted files
        let (files_extracted, total_size) = count_files_in_dir(dest);

        Ok(ExtractionResult {
            files_extracted,
            total_size,
            root_dir: find_root_dir(dest),
        })
    }

    fn apply_strip_prefix(&self, path: &Path, dest: &Path) -> PathBuf {
        if let Some(n) = self.strip_prefix {
            let components: Vec<_> = path.components().skip(n).collect();
            if components.is_empty() {
                return dest.to_path_buf();
            }
            dest.join(components.iter().collect::<PathBuf>())
        } else {
            dest.join(path)
        }
    }
}

/// Extraction result.
#[derive(Debug)]
pub struct ExtractionResult {
    /// Number of files extracted.
    pub files_extracted: usize,
    /// Total size in bytes.
    pub total_size: u64,
    /// Detected root directory (if single root).
    pub root_dir: Option<PathBuf>,
}

fn find_root_dir(dest: &Path) -> Option<PathBuf> {
    let entries: Vec<_> = WalkDir::new(dest)
        .min_depth(1)
        .max_depth(1)
        .into_iter()
        .filter_map(std::result::Result::ok)
        .collect();

    if entries.len() == 1 && entries[0].file_type().is_dir() {
        Some(entries[0].path().to_path_buf())
    } else {
        None
    }
}

/// Count files and total size in a directory.
fn count_files_in_dir(dir: &Path) -> (usize, u64) {
    let mut count = 0;
    let mut size = 0u64;

    for entry in WalkDir::new(dir).min_depth(1) {
        if let Ok(entry) = entry {
            if entry.file_type().is_file() {
                count += 1;
                if let Ok(meta) = entry.metadata() {
                    size += meta.len();
                }
            }
        }
    }

    (count, size)
}

/// Check if 7z or 7zz command is available.
#[must_use]
pub fn is_7z_available() -> bool {
    find_7z_command().is_some()
}

/// Find the 7z command (tries 7z, 7zz, 7za).
fn find_7z_command() -> Option<&'static str> {
    // Try different 7z command names
    for cmd in &["7z", "7zz", "7za"] {
        if Command::new(cmd)
            .arg("--help")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return Some(*cmd);
        }
    }
    None
}

/// Check if unrar command is available.
#[must_use]
pub fn is_unrar_available() -> bool {
    Command::new("unrar")
        .output()
        .map(|o| o.status.success() || o.status.code() == Some(0) || o.status.code() == Some(7))
        .unwrap_or(false)
}

/// Get a list of available archive tools.
#[must_use]
pub fn available_tools() -> Vec<(&'static str, bool)> {
    vec![
        ("zip", true),     // Always available (native)
        ("tar", true),     // Always available (native)
        ("tar.gz", true),  // Always available (native)
        ("tar.bz2", true), // Always available (native)
        ("tar.xz", true),  // Always available (native)
        ("7z", is_7z_available()),
        ("rar", is_unrar_available()),
    ]
}

/// Create a ZIP archive.
///
/// # Errors
/// Returns error if archive creation fails.
pub fn create_zip<W: std::io::Write + Seek>(
    writer: W,
    source: &Path,
    prefix: Option<&str>,
) -> Result<()> {
    let mut zip = zip::ZipWriter::new(writer);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    for entry in WalkDir::new(source).min_depth(1) {
        let entry = entry.map_err(|e| Error::Archive(e.to_string()))?;
        let path = entry.path();
        let relative = path
            .strip_prefix(source)
            .map_err(|e| Error::Archive(e.to_string()))?;

        let name = if let Some(p) = prefix {
            PathBuf::from(p).join(relative)
        } else {
            relative.to_path_buf()
        };

        let name_str = name.to_string_lossy();

        if path.is_dir() {
            zip.add_directory(&*name_str, options)
                .map_err(|e| Error::Archive(e.to_string()))?;
        } else {
            zip.start_file(&*name_str, options)
                .map_err(|e| Error::Archive(e.to_string()))?;

            let mut file = File::open(path).map_err(|e| Error::io(path, e))?;
            std::io::copy(&mut file, &mut zip).map_err(|e| Error::Archive(e.to_string()))?;
        }
    }

    zip.finish().map_err(|e| Error::Archive(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_type_detection() {
        assert_eq!(
            ArchiveType::from_filename("package.zip"),
            Some(ArchiveType::Zip)
        );
        assert_eq!(
            ArchiveType::from_filename("package.tar.gz"),
            Some(ArchiveType::TarGz)
        );
        assert_eq!(
            ArchiveType::from_filename("package.tgz"),
            Some(ArchiveType::TarGz)
        );
        assert_eq!(
            ArchiveType::from_filename("package.tar"),
            Some(ArchiveType::Tar)
        );
        assert_eq!(
            ArchiveType::from_filename("package.tar.bz2"),
            Some(ArchiveType::TarBz2)
        );
        assert_eq!(
            ArchiveType::from_filename("package.tbz2"),
            Some(ArchiveType::TarBz2)
        );
        assert_eq!(
            ArchiveType::from_filename("package.tar.xz"),
            Some(ArchiveType::TarXz)
        );
        assert_eq!(
            ArchiveType::from_filename("package.txz"),
            Some(ArchiveType::TarXz)
        );
        assert_eq!(
            ArchiveType::from_filename("package.7z"),
            Some(ArchiveType::SevenZip)
        );
        assert_eq!(
            ArchiveType::from_filename("package.rar"),
            Some(ArchiveType::Rar)
        );
        assert_eq!(ArchiveType::from_filename("package.unknown"), None);
    }

    #[test]
    fn archive_extension() {
        assert_eq!(ArchiveType::Zip.extension(), "zip");
        assert_eq!(ArchiveType::TarGz.extension(), "tar.gz");
        assert_eq!(ArchiveType::Tar.extension(), "tar");
        assert_eq!(ArchiveType::TarBz2.extension(), "tar.bz2");
        assert_eq!(ArchiveType::TarXz.extension(), "tar.xz");
        assert_eq!(ArchiveType::SevenZip.extension(), "7z");
        assert_eq!(ArchiveType::Rar.extension(), "rar");
    }

    #[test]
    fn archive_requires_cli() {
        assert!(!ArchiveType::Zip.requires_cli());
        assert!(!ArchiveType::TarGz.requires_cli());
        assert!(!ArchiveType::Tar.requires_cli());
        assert!(!ArchiveType::TarBz2.requires_cli());
        assert!(!ArchiveType::TarXz.requires_cli());
        assert!(ArchiveType::SevenZip.requires_cli());
        assert!(ArchiveType::Rar.requires_cli());
    }

    #[test]
    fn available_tools_list() {
        let tools = available_tools();
        assert!(tools.len() >= 7);
        // Native formats should always be available
        assert!(tools.iter().any(|(name, avail)| *name == "zip" && *avail));
        assert!(tools.iter().any(|(name, avail)| *name == "tar" && *avail));
        assert!(
            tools
                .iter()
                .any(|(name, avail)| *name == "tar.gz" && *avail)
        );
        assert!(
            tools
                .iter()
                .any(|(name, avail)| *name == "tar.bz2" && *avail)
        );
        assert!(
            tools
                .iter()
                .any(|(name, avail)| *name == "tar.xz" && *avail)
        );
    }
}
