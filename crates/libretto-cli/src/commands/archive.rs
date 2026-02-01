//! Archive command - create distributable archives.

use anyhow::{Context, Result};
use clap::Args;
use sonic_rs::JsonValueTrait;
use std::path::PathBuf;

/// Arguments for the archive command
#[derive(Args, Debug, Clone)]
pub struct ArchiveArgs {
    /// The package to archive (vendor/name format)
    #[arg(value_name = "PACKAGE")]
    pub package: Option<String>,

    /// Version to archive
    #[arg(value_name = "VERSION", name = "pkg_version")]
    pub version: Option<String>,

    /// Write the archive to this file
    #[arg(short = 'f', long, value_name = "FILE")]
    pub file: Option<PathBuf>,

    /// Write the archive to this directory
    #[arg(long, value_name = "DIR")]
    pub dir: Option<PathBuf>,

    /// Format of the archive (zip, tar, tar.gz, tar.bz2)
    #[arg(long, default_value = "tar")]
    pub format: String,

    /// Dump the archive to stdout instead of writing to file
    #[arg(long)]
    pub stdout: bool,

    /// Do not include files/folders from the project
    #[arg(long)]
    pub ignore_filters: bool,
}

/// Run the archive command
pub async fn run(args: ArchiveArgs) -> Result<()> {
    use crate::output::{header, info, success, warning};

    header("Creating archive");

    // Determine package to archive
    let package_name = if let Some(name) = &args.package {
        name.clone()
    } else {
        // Read from composer.json
        let composer_path = std::env::current_dir()?.join("composer.json");
        if !composer_path.exists() {
            anyhow::bail!("No composer.json found in current directory");
        }
        let content = std::fs::read_to_string(&composer_path)?;
        let json: sonic_rs::Value = sonic_rs::from_str(&content)?;
        json.get("name")
            .and_then(|v| v.as_str())
            .map(String::from)
            .context("No package name found in composer.json")?
    };

    let version = args.version.as_deref().unwrap_or("dev-main");

    info(&format!("Packaging {package_name} ({version})"));

    // Validate format
    let valid_formats = ["zip", "tar", "tar.gz", "tar.bz2", "tar.xz"];
    if !valid_formats.contains(&args.format.as_str()) {
        anyhow::bail!(
            "Invalid format '{}'. Valid formats: {}",
            args.format,
            valid_formats.join(", ")
        );
    }

    // Determine output path
    let output_dir = args.dir.unwrap_or_else(|| std::env::current_dir().unwrap());
    let filename = args.file.unwrap_or_else(|| {
        let safe_name = package_name.replace('/', "-");
        let ext = &args.format;
        PathBuf::from(format!("{safe_name}-{version}.{ext}"))
    });
    let output_path = output_dir.join(&filename);

    if args.stdout {
        warning("Writing to stdout is not yet implemented");
        return Ok(());
    }

    // Collect files to archive
    let cwd = std::env::current_dir()?;
    let mut files: Vec<PathBuf> = Vec::new();

    // Walk directory and collect files
    for entry in walkdir::WalkDir::new(&cwd).into_iter().filter_entry(|e| {
        let name = e.file_name().to_string_lossy();
        // Skip common excludes
        !name.starts_with('.') && name != "vendor" && name != "node_modules" && name != "target"
    }) {
        let entry = entry?;
        if entry.file_type().is_file() {
            files.push(entry.path().to_path_buf());
        }
    }

    info(&format!("Found {} files to archive", files.len()));

    // Create archive based on format
    match args.format.as_str() {
        "zip" => create_zip_archive(&output_path, &cwd, &files)?,
        "tar" | "tar.gz" | "tar.bz2" | "tar.xz" => {
            create_tar_archive(&output_path, &cwd, &files, &args.format)?;
        }
        _ => unreachable!(),
    }

    success(&format!("Created archive: {}", output_path.display()));

    // Print archive size
    let metadata = std::fs::metadata(&output_path)?;
    info(&format!(
        "Archive size: {}",
        crate::output::format_bytes(metadata.len())
    ));

    Ok(())
}

fn create_zip_archive(output_path: &PathBuf, base_dir: &PathBuf, files: &[PathBuf]) -> Result<()> {
    use std::io::Write;

    let file = std::fs::File::create(output_path)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    for path in files {
        let relative = path.strip_prefix(base_dir)?;
        let name = relative.to_string_lossy();

        zip.start_file(name.as_ref(), options)?;
        let content = std::fs::read(path)?;
        zip.write_all(&content)?;
    }

    zip.finish()?;
    Ok(())
}

fn create_tar_archive(
    output_path: &PathBuf,
    base_dir: &PathBuf,
    files: &[PathBuf],
    format: &str,
) -> Result<()> {
    use std::fs::File;

    let file = File::create(output_path)?;

    // Create the appropriate encoder
    let encoder: Box<dyn std::io::Write> = match format {
        "tar" => Box::new(file),
        "tar.gz" => Box::new(flate2::write::GzEncoder::new(
            file,
            flate2::Compression::default(),
        )),
        "tar.bz2" => {
            // bzip2 not directly available, use gzip as fallback
            crate::output::warning("bzip2 compression falling back to gzip");
            Box::new(flate2::write::GzEncoder::new(
                file,
                flate2::Compression::default(),
            ))
        }
        "tar.xz" => {
            // xz not directly available, use gzip as fallback
            crate::output::warning("xz compression falling back to gzip");
            Box::new(flate2::write::GzEncoder::new(
                file,
                flate2::Compression::default(),
            ))
        }
        _ => unreachable!(),
    };

    let mut tar = tar::Builder::new(encoder);

    for path in files {
        let relative = path.strip_prefix(base_dir)?;
        tar.append_path_with_name(path, relative)?;
    }

    tar.finish()?;
    Ok(())
}
