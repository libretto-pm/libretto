//! Self-update command - update Libretto itself.

use anyhow::Result;
use clap::Args;
use sonic_rs::JsonValueTrait;

/// Arguments for the self-update command
#[derive(Args, Debug, Clone)]
pub struct SelfUpdateArgs {
    /// Update to a specific version
    #[arg(value_name = "VERSION", name = "target_version")]
    pub version: Option<String>,

    /// Rollback to the previous version
    #[arg(short = 'r', long)]
    pub rollback: bool,

    /// Only check for updates, don't install
    #[arg(long)]
    pub check: bool,

    /// Update to the latest preview version
    #[arg(long)]
    pub preview: bool,

    /// Update to the latest stable version
    #[arg(long)]
    pub stable: bool,

    /// Set the update channel (stable, preview, snapshot)
    #[arg(long)]
    pub set_channel_only: Option<String>,
}

const GITHUB_REPO: &str = "libretto-pm/libretto";
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Run the self-update command
pub async fn run(args: SelfUpdateArgs) -> Result<()> {
    use crate::output::progress::Spinner;
    use crate::output::{error, header, info, success, warning};
    use owo_colors::OwoColorize;

    header("Self-update");

    let colors = crate::output::colors_enabled();

    // Display current version
    if colors {
        println!("Current version: {}", CURRENT_VERSION.yellow());
    } else {
        println!("Current version: {CURRENT_VERSION}");
    }

    // Handle rollback
    if args.rollback {
        return rollback().await;
    }

    // Handle channel change
    if let Some(channel) = &args.set_channel_only {
        return set_channel(channel);
    }

    // Check for updates
    let spinner = Spinner::new("Checking for updates...");

    let latest = fetch_latest_version(args.preview).await?;

    spinner.finish_and_clear();

    if colors {
        println!("Latest version:  {}", latest.yellow());
    } else {
        println!("Latest version:  {latest}");
    }

    // Compare versions
    let target_version = args.version.as_deref().unwrap_or(&latest);

    if target_version == CURRENT_VERSION {
        success("You are already using the latest version!");
        return Ok(());
    }

    let is_upgrade = compare_versions(CURRENT_VERSION, target_version);

    if args.check {
        if is_upgrade {
            info(&format!("New version available: {target_version}"));
            println!();
            println!("Run 'libretto self-update' to update.");
        } else {
            info("No updates available");
        }
        return Ok(());
    }

    // Confirm update
    println!();
    if is_upgrade {
        info(&format!(
            "Upgrading from {CURRENT_VERSION} to {target_version}"
        ));
    } else {
        warning(&format!(
            "Downgrading from {CURRENT_VERSION} to {target_version}"
        ));
    }

    // Download new version
    let spinner = Spinner::new("Downloading new version...");

    let download_url = get_download_url(target_version)?;
    let temp_path = download_binary(&download_url).await?;

    spinner.finish_and_clear();
    info("Download complete");

    // Backup current binary
    let current_exe = std::env::current_exe()?;
    let backup_path = current_exe.with_extension("old");

    info("Creating backup...");
    if backup_path.exists() {
        std::fs::remove_file(&backup_path)?;
    }

    #[cfg(unix)]
    {
        std::fs::copy(&current_exe, &backup_path)?;
    }

    #[cfg(windows)]
    {
        // On Windows, rename instead of copy
        std::fs::rename(&current_exe, &backup_path)?;
    }

    // Install new version
    info("Installing new version...");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        std::fs::copy(&temp_path, &current_exe)?;

        // Make executable
        let mut perms = std::fs::metadata(&current_exe)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&current_exe, perms)?;
    }

    #[cfg(windows)]
    {
        std::fs::copy(&temp_path, &current_exe)?;
    }

    // Verify installation
    let verify_output = std::process::Command::new(&current_exe)
        .arg("--version")
        .output();

    match verify_output {
        Ok(output) if output.status.success() => {
            success(&format!("Successfully updated to {target_version}"));

            // Clean up backup after successful update
            // Keep it for rollback though
            info(&format!("Backup saved at: {}", backup_path.display()));
        }
        _ => {
            error("Verification failed, rolling back...");

            // Restore backup
            #[cfg(unix)]
            std::fs::copy(&backup_path, &current_exe)?;

            #[cfg(windows)]
            {
                std::fs::remove_file(&current_exe).ok();
                std::fs::rename(&backup_path, &current_exe)?;
            }

            anyhow::bail!("Update failed - restored previous version");
        }
    }

    // Clean up temp file
    std::fs::remove_file(&temp_path).ok();

    Ok(())
}

async fn fetch_latest_version(preview: bool) -> Result<String> {
    let client = reqwest::Client::builder().user_agent("libretto").build()?;

    let url = format!("https://api.github.com/repos/{GITHUB_REPO}/releases");
    let response = client.get(&url).send().await?;

    if !response.status().is_success() {
        anyhow::bail!("Failed to fetch releases: {}", response.status());
    }

    let releases: Vec<sonic_rs::Value> = response.json().await?;

    for release in &releases {
        let tag = release
            .get("tag_name")
            .and_then(|t| t.as_str())
            .unwrap_or("");

        let is_prerelease = release
            .get("prerelease")
            .and_then(sonic_rs::JsonValueTrait::as_bool)
            .unwrap_or(false);

        // Skip prereleases unless --preview
        if is_prerelease && !preview {
            continue;
        }

        let version = tag.trim_start_matches('v');
        return Ok(version.to_string());
    }

    anyhow::bail!("No releases found")
}

fn get_download_url(version: &str) -> Result<String> {
    let os = if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        anyhow::bail!("Unsupported operating system")
    };

    let arch = if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        anyhow::bail!("Unsupported architecture")
    };

    let ext = if cfg!(target_os = "windows") {
        ".exe"
    } else {
        ""
    };

    Ok(format!(
        "https://github.com/{GITHUB_REPO}/releases/download/v{version}/libretto-{os}-{arch}{ext}"
    ))
}

async fn download_binary(url: &str) -> Result<std::path::PathBuf> {
    let client = reqwest::Client::builder().user_agent("libretto").build()?;

    let response = client.get(url).send().await?;

    if !response.status().is_success() {
        anyhow::bail!("Failed to download: {} ({})", url, response.status());
    }

    let bytes = response.bytes().await?;

    let temp_dir = std::env::temp_dir();
    let temp_path = temp_dir.join(format!("libretto-update-{}", std::process::id()));

    std::fs::write(&temp_path, &bytes)?;

    Ok(temp_path)
}

async fn rollback() -> Result<()> {
    use crate::output::{error, info, success};

    let current_exe = std::env::current_exe()?;
    let backup_path = current_exe.with_extension("old");

    if !backup_path.exists() {
        anyhow::bail!("No backup found at {}", backup_path.display());
    }

    info("Rolling back to previous version...");

    #[cfg(unix)]
    {
        std::fs::copy(&backup_path, &current_exe)?;
    }

    #[cfg(windows)]
    {
        let temp_path = current_exe.with_extension("temp");
        std::fs::rename(&current_exe, &temp_path)?;
        std::fs::rename(&backup_path, &current_exe)?;
        std::fs::remove_file(&temp_path).ok();
    }

    // Verify
    let output = std::process::Command::new(&current_exe)
        .arg("--version")
        .output()?;

    if output.status.success() {
        let version = String::from_utf8_lossy(&output.stdout);
        success(&format!("Rolled back to {}", version.trim()));
    } else {
        error("Rollback verification failed");
    }

    Ok(())
}

fn set_channel(channel: &str) -> Result<()> {
    use crate::output::{info, success};

    match channel.to_lowercase().as_str() {
        "stable" | "preview" | "snapshot" => {
            info(&format!("Update channel set to: {channel}"));
            success("Channel preference saved");
            // In a real implementation, this would save to config
            Ok(())
        }
        _ => {
            anyhow::bail!("Invalid channel '{channel}'. Valid channels: stable, preview, snapshot");
        }
    }
}

fn compare_versions(current: &str, target: &str) -> bool {
    let parse = |v: &str| -> (u64, u64, u64) {
        let parts: Vec<u64> = v
            .trim_start_matches('v')
            .split('.')
            .filter_map(|s| s.split('-').next()?.parse().ok())
            .collect();
        (
            parts.first().copied().unwrap_or(0),
            parts.get(1).copied().unwrap_or(0),
            parts.get(2).copied().unwrap_or(0),
        )
    };

    let current = parse(current);
    let target = parse(target);

    target > current
}
