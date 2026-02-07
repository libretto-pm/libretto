//! Exec command - execute vendored binaries/scripts.

use anyhow::{Context, Result};
use clap::Args;
use std::path::{Path, PathBuf};

/// Arguments for the exec command
#[derive(Args, Debug, Clone)]
pub struct ExecArgs {
    /// Binary/script name to execute
    #[arg(required = true, value_name = "BINARY")]
    pub binary: String,

    /// Arguments to pass to the binary
    #[arg(trailing_var_arg = true, value_name = "ARGS")]
    pub args: Vec<String>,

    /// List available binaries
    #[arg(short = 'l', long)]
    pub list: bool,
}

/// Run the exec command
pub async fn run(args: ExecArgs) -> Result<()> {
    // Handle list mode
    if args.list {
        return list_binaries();
    }

    let vendor_bin = std::env::current_dir()?.join("vendor").join("bin");

    if !vendor_bin.exists() {
        anyhow::bail!("No vendor/bin directory found. Run 'libretto install' first.");
    }

    // Find the binary
    let binary_path = find_binary(&vendor_bin, &args.binary)?;

    // Execute the binary
    let status = std::process::Command::new(&binary_path)
        .args(&args.args)
        .status()
        .context(format!("Failed to execute '{}'", binary_path.display()))?;

    if !status.success() {
        let code = status.code().unwrap_or(1);
        std::process::exit(code);
    }

    Ok(())
}

fn find_binary(vendor_bin: &PathBuf, name: &str) -> Result<PathBuf> {
    // Try exact name first
    let exact = vendor_bin.join(name);
    if exact.exists() {
        return Ok(exact);
    }

    // On Windows, try with extensions
    #[cfg(windows)]
    {
        for ext in &["", ".bat", ".cmd", ".exe", ".phar"] {
            let with_ext = vendor_bin.join(format!("{name}{ext}"));
            if with_ext.exists() {
                return Ok(with_ext);
            }
        }
    }

    // On Unix, try with .phar extension
    #[cfg(unix)]
    {
        let phar = vendor_bin.join(format!("{name}.phar"));
        if phar.exists() {
            return Ok(phar);
        }
    }

    // List available binaries in error message
    #[cfg(unix)]
    let available: Vec<String> = std::fs::read_dir(vendor_bin)?
        .filter_map(std::result::Result::ok)
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            if name.ends_with(".bat") {
                None
            } else {
                Some(name)
            }
        })
        .collect();
    #[cfg(not(unix))]
    let available: Vec<String> = std::fs::read_dir(vendor_bin)?
        .filter_map(std::result::Result::ok)
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();

    anyhow::bail!(
        "Binary '{}' not found in vendor/bin.\nAvailable binaries: {}",
        name,
        if available.is_empty() {
            "(none)".to_string()
        } else {
            available.join(", ")
        }
    );
}

fn list_binaries() -> Result<()> {
    use crate::output::table::Table;
    use crate::output::{header, info};

    header("Available binaries");

    let vendor_bin = std::env::current_dir()?.join("vendor").join("bin");

    if !vendor_bin.exists() {
        info("No vendor/bin directory found");
        return Ok(());
    }

    let mut entries: Vec<(String, String)> = Vec::new();

    for entry in std::fs::read_dir(&vendor_bin)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();

        // Skip batch files on Unix
        #[cfg(unix)]
        if name.ends_with(".bat") {
            continue;
        }

        // Determine the source package
        let source = detect_binary_source(&vendor_bin, &name);

        entries.push((name, source));
    }

    if entries.is_empty() {
        info("No binaries installed");
        return Ok(());
    }

    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let mut table = Table::new();
    table.headers(["Binary", "Package"]);

    for (name, source) in &entries {
        table.row([name.as_str(), source.as_str()]);
    }

    table.print();

    println!();
    info("Run with: libretto exec <binary> [args...]");

    Ok(())
}

fn detect_binary_source(vendor_bin: &Path, binary_name: &str) -> String {
    let binary_path = vendor_bin.join(binary_name);

    // Try to read the binary and find package info
    if let Ok(content) = std::fs::read_to_string(&binary_path) {
        // Look for common patterns in shim files
        if let Some(line) = content.lines().find(|l| l.contains("vendor/")) {
            // Extract package name from path like '../vendor/package/name/bin/...'
            let parts: Vec<&str> = line.split("vendor/").collect();
            if parts.len() > 1 {
                let after_vendor = parts[1];
                let path_parts: Vec<&str> = after_vendor.split('/').collect();
                if path_parts.len() >= 2 {
                    return format!("{}/{}", path_parts[0], path_parts[1]);
                }
            }
        }
    }

    // Check if it's a symlink
    #[cfg(unix)]
    {
        if let Ok(target) = std::fs::read_link(&binary_path) {
            let target_str = target.to_string_lossy();
            if target_str.contains("vendor/") {
                let parts: Vec<&str> = target_str.split("vendor/").collect();
                if parts.len() > 1 {
                    let path_parts: Vec<&str> = parts[1].split('/').collect();
                    if path_parts.len() >= 2 {
                        return format!("{}/{}", path_parts[0], path_parts[1]);
                    }
                }
            }
        }
    }

    "(unknown)".to_string()
}
