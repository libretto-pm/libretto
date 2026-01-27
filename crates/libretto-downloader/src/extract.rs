//! Streaming archive extraction with async decompression.
//!
//! Supports ZIP, tar.gz, tar.bz2, tar.xz, and tar.zst formats.

use crate::error::{DownloadError, Result};
use crate::source::ArchiveType;
use async_compression::tokio::bufread::{BzDecoder, GzipDecoder, XzDecoder, ZstdDecoder};
use async_zip::base::read::seek::ZipFileReader;
use std::path::{Path, PathBuf};
use tokio::fs::{self, File, OpenOptions};
use tokio::io::BufReader;
use tokio_tar::Archive as TarArchive;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use tracing::{debug, info, trace};

/// Extraction options.
#[derive(Debug, Clone, Default)]
pub struct ExtractOptions {
    /// Strip N path components from extracted files.
    pub strip_prefix: usize,
    /// Overwrite existing files.
    pub overwrite: bool,
    /// Preserve file permissions (Unix only).
    pub preserve_permissions: bool,
}

impl ExtractOptions {
    /// Create default options.
    #[must_use]
    pub fn new() -> Self {
        Self {
            strip_prefix: 0,
            overwrite: true,
            preserve_permissions: true,
        }
    }

    /// Set strip prefix.
    #[must_use]
    pub const fn with_strip_prefix(mut self, n: usize) -> Self {
        self.strip_prefix = n;
        self
    }
}

/// Extraction result.
#[derive(Debug, Clone)]
pub struct ExtractionResult {
    /// Number of files extracted.
    pub files_extracted: usize,
    /// Total size in bytes.
    pub total_size: u64,
    /// Detected root directory (if single root).
    pub root_dir: Option<PathBuf>,
}

/// Async archive extractor.
pub struct Extractor {
    options: ExtractOptions,
}

impl std::fmt::Debug for Extractor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Extractor")
            .field("options", &self.options)
            .finish()
    }
}

impl Default for Extractor {
    fn default() -> Self {
        Self::new()
    }
}

impl Extractor {
    /// Create a new extractor with default options.
    #[must_use]
    pub fn new() -> Self {
        Self {
            options: ExtractOptions::new(),
        }
    }

    /// Create extractor with options.
    #[must_use]
    pub fn with_options(options: ExtractOptions) -> Self {
        Self { options }
    }

    /// Extract an archive to destination directory.
    ///
    /// # Errors
    /// Returns error if extraction fails.
    pub async fn extract(&self, archive: &Path, dest: &Path) -> Result<ExtractionResult> {
        let archive_type =
            ArchiveType::from_extension(&archive.to_string_lossy()).ok_or_else(|| {
                DownloadError::Archive(format!("unknown archive type: {}", archive.display()))
            })?;

        debug!(archive = ?archive, dest = ?dest, archive_type = ?archive_type, "extracting");

        fs::create_dir_all(dest)
            .await
            .map_err(|e| DownloadError::io(dest, e))?;

        let result = match archive_type {
            ArchiveType::Zip => self.extract_zip(archive, dest).await?,
            ArchiveType::TarGz => self.extract_tar_gz(archive, dest).await?,
            ArchiveType::TarBz2 => self.extract_tar_bz2(archive, dest).await?,
            ArchiveType::TarXz => self.extract_tar_xz(archive, dest).await?,
            ArchiveType::TarZst => self.extract_tar_zst(archive, dest).await?,
            ArchiveType::Tar => self.extract_tar(archive, dest).await?,
        };

        info!(
            files = result.files_extracted,
            size = result.total_size,
            "extraction complete"
        );

        Ok(result)
    }

    /// Extract ZIP archive.
    async fn extract_zip(&self, archive: &Path, dest: &Path) -> Result<ExtractionResult> {
        let file = File::open(archive)
            .await
            .map_err(|e| DownloadError::io(archive, e))?;

        let reader = BufReader::new(file).compat();

        let mut zip = ZipFileReader::new(reader)
            .await
            .map_err(|e| DownloadError::Archive(e.to_string()))?;

        let mut files_extracted = 0;
        let mut total_size = 0u64;

        let entry_count = zip.file().entries().len();

        for i in 0..entry_count {
            let entry = zip
                .file()
                .entries()
                .get(i)
                .ok_or_else(|| DownloadError::Archive(format!("failed to get entry {i}")))?;

            let filename = entry
                .filename()
                .as_str()
                .map_err(|e| DownloadError::Archive(format!("invalid filename: {e}")))?;

            // Sanitize the path to prevent directory traversal
            let sanitized: PathBuf = filename
                .replace('\\', "/")
                .split('/')
                .filter(|s| !s.is_empty() && *s != "." && *s != "..")
                .collect();

            let path = self.strip_path_prefix(&sanitized);
            if path.as_os_str().is_empty() {
                continue;
            }

            let out_path = dest.join(&path);

            // Validate path doesn't escape dest
            if !out_path.starts_with(dest) {
                return Err(DownloadError::Archive(format!(
                    "path escape attempt: {}",
                    filename
                )));
            }

            let is_dir = entry
                .dir()
                .map_err(|e| DownloadError::Archive(e.to_string()))?;

            #[cfg(unix)]
            let unix_mode = entry.unix_permissions();

            if is_dir {
                fs::create_dir_all(&out_path)
                    .await
                    .map_err(|e| DownloadError::io(&out_path, e))?;
            } else {
                if let Some(parent) = out_path.parent() {
                    fs::create_dir_all(parent)
                        .await
                        .map_err(|e| DownloadError::io(parent, e))?;
                }

                let mut entry_reader = zip
                    .reader_without_entry(i)
                    .await
                    .map_err(|e| DownloadError::Archive(e.to_string()))?;

                let writer = OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .open(&out_path)
                    .await
                    .map_err(|e| DownloadError::io(&out_path, e))?;

                let size = futures_lite::io::copy(&mut entry_reader, &mut writer.compat_write())
                    .await
                    .map_err(|e| DownloadError::Archive(e.to_string()))?;

                total_size += size;
                files_extracted += 1;

                #[cfg(unix)]
                if self.options.preserve_permissions {
                    if let Some(mode) = unix_mode {
                        use std::os::unix::fs::PermissionsExt;
                        let _ = std::fs::set_permissions(
                            &out_path,
                            std::fs::Permissions::from_mode(mode.into()),
                        );
                    }
                }

                trace!(file = ?out_path, size, "extracted file");
            }
        }

        Ok(ExtractionResult {
            files_extracted,
            total_size,
            root_dir: self.find_root_dir(dest).await,
        })
    }

    /// Extract gzipped tarball.
    async fn extract_tar_gz(&self, archive: &Path, dest: &Path) -> Result<ExtractionResult> {
        let file = File::open(archive)
            .await
            .map_err(|e| DownloadError::io(archive, e))?;

        let reader = BufReader::new(file);
        let decoder = GzipDecoder::new(reader);
        self.extract_tar_reader(decoder, dest).await
    }

    /// Extract bzip2 tarball.
    async fn extract_tar_bz2(&self, archive: &Path, dest: &Path) -> Result<ExtractionResult> {
        let file = File::open(archive)
            .await
            .map_err(|e| DownloadError::io(archive, e))?;

        let reader = BufReader::new(file);
        let decoder = BzDecoder::new(reader);
        self.extract_tar_reader(decoder, dest).await
    }

    /// Extract xz tarball.
    async fn extract_tar_xz(&self, archive: &Path, dest: &Path) -> Result<ExtractionResult> {
        let file = File::open(archive)
            .await
            .map_err(|e| DownloadError::io(archive, e))?;

        let reader = BufReader::new(file);
        let decoder = XzDecoder::new(reader);
        self.extract_tar_reader(decoder, dest).await
    }

    /// Extract zstd tarball.
    async fn extract_tar_zst(&self, archive: &Path, dest: &Path) -> Result<ExtractionResult> {
        let file = File::open(archive)
            .await
            .map_err(|e| DownloadError::io(archive, e))?;

        let reader = BufReader::new(file);
        let decoder = ZstdDecoder::new(reader);
        self.extract_tar_reader(decoder, dest).await
    }

    /// Extract plain tarball.
    async fn extract_tar(&self, archive: &Path, dest: &Path) -> Result<ExtractionResult> {
        let file = File::open(archive)
            .await
            .map_err(|e| DownloadError::io(archive, e))?;

        let reader = BufReader::new(file);
        self.extract_tar_reader(reader, dest).await
    }

    /// Extract tar from any async reader.
    async fn extract_tar_reader<R: tokio::io::AsyncRead + Unpin>(
        &self,
        reader: R,
        dest: &Path,
    ) -> Result<ExtractionResult> {
        let mut archive = TarArchive::new(reader);
        let mut entries = archive
            .entries()
            .map_err(|e| DownloadError::Archive(e.to_string()))?;

        let mut files_extracted = 0;
        let mut total_size = 0u64;

        use futures_util::TryStreamExt;
        while let Some(entry) = entries
            .try_next()
            .await
            .map_err(|e| DownloadError::Archive(e.to_string()))?
        {
            let path = entry
                .path()
                .map_err(|e| DownloadError::Archive(e.to_string()))?;
            let path = self.strip_path_prefix(&path);

            if path.as_os_str().is_empty() {
                continue;
            }

            let out_path = dest.join(&path);

            // Validate path doesn't escape dest
            if !out_path.starts_with(dest) {
                return Err(DownloadError::Archive(format!(
                    "path escape attempt: {}",
                    path.display()
                )));
            }

            let entry_type = entry.header().entry_type();

            if entry_type.is_dir() {
                fs::create_dir_all(&out_path)
                    .await
                    .map_err(|e| DownloadError::io(&out_path, e))?;
            } else if entry_type.is_file() {
                if let Some(parent) = out_path.parent() {
                    fs::create_dir_all(parent)
                        .await
                        .map_err(|e| DownloadError::io(parent, e))?;
                }

                // Use unpack_in for the entry
                let size = entry.header().size().unwrap_or(0);

                let mut entry = entry;
                entry
                    .unpack(&out_path)
                    .await
                    .map_err(|e| DownloadError::Archive(e.to_string()))?;

                total_size += size;
                files_extracted += 1;

                trace!(file = ?out_path, size, "extracted file");
            }
        }

        Ok(ExtractionResult {
            files_extracted,
            total_size,
            root_dir: self.find_root_dir(dest).await,
        })
    }

    /// Strip prefix components from path.
    fn strip_path_prefix(&self, path: &Path) -> PathBuf {
        if self.options.strip_prefix == 0 {
            return path.to_path_buf();
        }

        let components: Vec<_> = path.components().skip(self.options.strip_prefix).collect();
        if components.is_empty() {
            PathBuf::new()
        } else {
            components.iter().collect()
        }
    }

    /// Find single root directory in destination.
    async fn find_root_dir(&self, dest: &Path) -> Option<PathBuf> {
        let mut read_dir = fs::read_dir(dest).await.ok()?;
        let mut entries = Vec::new();

        while let Ok(Some(entry)) = read_dir.next_entry().await {
            entries.push(entry);
            if entries.len() > 1 {
                return None;
            }
        }

        if entries.len() == 1 && entries[0].file_type().await.ok()?.is_dir() {
            Some(entries[0].path())
        } else {
            None
        }
    }
}

/// Extract archive to directory (convenience function).
///
/// # Errors
/// Returns error if extraction fails.
pub async fn extract(archive: &Path, dest: &Path) -> Result<ExtractionResult> {
    Extractor::new().extract(archive, dest).await
}

/// Extract archive with strip prefix.
///
/// # Errors
/// Returns error if extraction fails.
pub async fn extract_with_strip(
    archive: &Path,
    dest: &Path,
    strip: usize,
) -> Result<ExtractionResult> {
    let options = ExtractOptions::new().with_strip_prefix(strip);
    Extractor::with_options(options)
        .extract(archive, dest)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_options_default() {
        let options = ExtractOptions::new();
        assert_eq!(options.strip_prefix, 0);
        assert!(options.overwrite);
        assert!(options.preserve_permissions);
    }

    #[test]
    fn strip_path_prefix() {
        let extractor = Extractor::with_options(ExtractOptions::new().with_strip_prefix(1));
        let path = Path::new("prefix/subdir/file.txt");
        let stripped = extractor.strip_path_prefix(path);
        assert_eq!(stripped, Path::new("subdir/file.txt"));
    }

    #[test]
    fn strip_path_prefix_zero() {
        let extractor = Extractor::new();
        let path = Path::new("dir/file.txt");
        let stripped = extractor.strip_path_prefix(path);
        assert_eq!(stripped, path);
    }

    #[test]
    fn strip_path_prefix_all() {
        let extractor = Extractor::with_options(ExtractOptions::new().with_strip_prefix(3));
        let path = Path::new("a/b");
        let stripped = extractor.strip_path_prefix(path);
        assert_eq!(stripped, PathBuf::new());
    }
}
