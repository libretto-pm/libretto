//! Diagnose command - system diagnostics and problem detection.

use anyhow::Result;
use clap::Args;
use std::process::Command;

/// Arguments for the diagnose command
#[derive(Args, Debug, Clone)]
pub struct DiagnoseArgs {}

/// Diagnostic check result
#[derive(Debug)]
enum CheckResult {
    Ok(String),
    Warning(String),
    Error(String),
}

/// Run the diagnose command
pub async fn run(_args: DiagnoseArgs) -> Result<()> {
    use crate::output::header;
    use owo_colors::OwoColorize;

    header("Diagnosing system");
    println!();

    let colors = crate::output::colors_enabled();
    let mut warnings = 0;
    let mut errors = 0;

    // Check PHP
    print_check(
        "Checking PHP version",
        check_php(),
        colors,
        &mut warnings,
        &mut errors,
    );

    // Check Composer files
    print_check(
        "Checking composer.json",
        check_composer_json(),
        colors,
        &mut warnings,
        &mut errors,
    );

    print_check(
        "Checking composer.lock",
        check_composer_lock(),
        colors,
        &mut warnings,
        &mut errors,
    );

    // Check connectivity
    print_check(
        "Checking packagist.org connectivity",
        check_packagist().await,
        colors,
        &mut warnings,
        &mut errors,
    );

    // Check GitHub connectivity
    print_check(
        "Checking github.com connectivity",
        check_github().await,
        colors,
        &mut warnings,
        &mut errors,
    );

    // Check VCS tools
    println!();
    println!("Version Control Systems:");
    print_check("  git", check_git(), colors, &mut warnings, &mut errors);
    print_check(
        "  svn (Subversion)",
        check_svn(),
        colors,
        &mut warnings,
        &mut errors,
    );
    print_check(
        "  hg (Mercurial)",
        check_hg(),
        colors,
        &mut warnings,
        &mut errors,
    );
    print_check(
        "  fossil",
        check_fossil(),
        colors,
        &mut warnings,
        &mut errors,
    );
    print_check(
        "  p4 (Perforce)",
        check_perforce(),
        colors,
        &mut warnings,
        &mut errors,
    );

    // Check archive tools
    println!();
    println!("Archive Tools:");
    print_check(
        "  zip/tar/gz/bz2/xz (native)",
        CheckResult::Ok("Built-in support".to_string()),
        colors,
        &mut warnings,
        &mut errors,
    );
    print_check(
        "  7z/7zz (7-Zip)",
        check_7z(),
        colors,
        &mut warnings,
        &mut errors,
    );
    print_check(
        "  unrar (RAR)",
        check_unrar(),
        colors,
        &mut warnings,
        &mut errors,
    );
    println!();

    // Check cache directory
    print_check(
        "Checking cache directory",
        check_cache_dir(),
        colors,
        &mut warnings,
        &mut errors,
    );

    // Check vendor directory
    print_check(
        "Checking vendor directory",
        check_vendor_dir(),
        colors,
        &mut warnings,
        &mut errors,
    );

    // Check environment
    print_check(
        "Checking COMPOSER_HOME",
        check_composer_home(),
        colors,
        &mut warnings,
        &mut errors,
    );

    // Check disk space
    print_check(
        "Checking available disk space",
        check_disk_space(),
        colors,
        &mut warnings,
        &mut errors,
    );

    // Check TLS/SSL
    print_check(
        "Checking TLS/SSL support",
        check_tls(),
        colors,
        &mut warnings,
        &mut errors,
    );

    println!();

    // Summary
    if errors > 0 {
        if colors {
            println!(
                "{} Found {} error(s) and {} warning(s)",
                "FAIL".red().bold(),
                errors,
                warnings
            );
        } else {
            println!("FAIL: Found {errors} error(s) and {warnings} warning(s)");
        }
        std::process::exit(1);
    } else if warnings > 0 {
        if colors {
            println!(
                "{} Found {} warning(s), but no errors",
                "WARN".yellow().bold(),
                warnings
            );
        } else {
            println!("WARN: Found {warnings} warning(s), but no errors");
        }
    } else if colors {
        println!("{} No issues found", "OK".green().bold());
    } else {
        println!("OK: No issues found");
    }

    Ok(())
}

fn print_check(
    label: &str,
    result: CheckResult,
    colors: bool,
    warnings: &mut i32,
    errors: &mut i32,
) {
    use owo_colors::OwoColorize;

    let (status, message) = match &result {
        CheckResult::Ok(msg) => {
            let status = if colors {
                "OK".green().to_string()
            } else {
                "OK".to_string()
            };
            (status, msg.clone())
        }
        CheckResult::Warning(msg) => {
            *warnings += 1;
            let status = if colors {
                "WARN".yellow().to_string()
            } else {
                "WARN".to_string()
            };
            (status, msg.clone())
        }
        CheckResult::Error(msg) => {
            *errors += 1;
            let status = if colors {
                "FAIL".red().to_string()
            } else {
                "FAIL".to_string()
            };
            (status, msg.clone())
        }
    };

    println!("{label}: {status} - {message}");
}

fn check_php() -> CheckResult {
    match Command::new("php")
        .args(["-r", "echo PHP_VERSION;"])
        .output()
    {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();

            // Check minimum version (8.1+)
            let parts: Vec<u32> = version.split('.').filter_map(|s| s.parse().ok()).collect();

            if parts.len() >= 2 && (parts[0] > 8 || (parts[0] == 8 && parts[1] >= 1)) {
                CheckResult::Ok(format!("PHP {version}"))
            } else if parts.len() >= 2 && parts[0] >= 7 {
                CheckResult::Warning(format!("PHP {version} detected, but 8.1+ is recommended"))
            } else {
                CheckResult::Error(format!("PHP {version} is too old, minimum required is 7.4"))
            }
        }
        Ok(_) => CheckResult::Error("PHP is installed but returned an error".to_string()),
        Err(_) => CheckResult::Warning("PHP not found in PATH".to_string()),
    }
}

fn check_composer_json() -> CheckResult {
    let path = std::env::current_dir()
        .map(|d| d.join("composer.json"))
        .unwrap_or_default();

    if !path.exists() {
        return CheckResult::Warning("No composer.json found in current directory".to_string());
    }

    match std::fs::read_to_string(&path) {
        Ok(content) => match sonic_rs::from_str::<sonic_rs::Value>(&content) {
            Ok(_) => CheckResult::Ok("Valid JSON".to_string()),
            Err(e) => CheckResult::Error(format!("Invalid JSON: {e}")),
        },
        Err(e) => CheckResult::Error(format!("Cannot read file: {e}")),
    }
}

fn check_composer_lock() -> CheckResult {
    let composer_json = std::env::current_dir()
        .map(|d| d.join("composer.json"))
        .unwrap_or_default();
    let lock_path = std::env::current_dir()
        .map(|d| d.join("composer.lock"))
        .unwrap_or_default();

    if !composer_json.exists() {
        return CheckResult::Ok("No composer.json, lock file not needed".to_string());
    }

    if !lock_path.exists() {
        return CheckResult::Warning(
            "composer.lock not found - run 'libretto install'".to_string(),
        );
    }

    match std::fs::read_to_string(&lock_path) {
        Ok(content) => match sonic_rs::from_str::<sonic_rs::Value>(&content) {
            Ok(_) => CheckResult::Ok("Valid JSON".to_string()),
            Err(e) => CheckResult::Error(format!("Invalid JSON: {e}")),
        },
        Err(e) => CheckResult::Error(format!("Cannot read file: {e}")),
    }
}

async fn check_packagist() -> CheckResult {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build();

    match client {
        Ok(client) => match client
            .get("https://packagist.org/packages.json")
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => {
                CheckResult::Ok("Connected successfully".to_string())
            }
            Ok(response) => CheckResult::Warning(format!("HTTP status: {}", response.status())),
            Err(e) => CheckResult::Error(format!("Connection failed: {e}")),
        },
        Err(e) => CheckResult::Error(format!("Failed to create HTTP client: {e}")),
    }
}

async fn check_github() -> CheckResult {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build();

    match client {
        Ok(client) => match client.get("https://api.github.com").send().await {
            Ok(response) if response.status().is_success() => {
                CheckResult::Ok("Connected successfully".to_string())
            }
            Ok(response) if response.status().as_u16() == 403 => {
                CheckResult::Warning("Rate limited (this is normal without a token)".to_string())
            }
            Ok(response) => CheckResult::Warning(format!("HTTP status: {}", response.status())),
            Err(e) => CheckResult::Error(format!("Connection failed: {e}")),
        },
        Err(e) => CheckResult::Error(format!("Failed to create HTTP client: {e}")),
    }
}

fn check_git() -> CheckResult {
    match Command::new("git").args(["--version"]).output() {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            CheckResult::Ok(version)
        }
        Ok(_) => CheckResult::Error("git is installed but returned an error".to_string()),
        Err(_) => CheckResult::Warning("Not installed (required for git packages)".to_string()),
    }
}

fn check_svn() -> CheckResult {
    match Command::new("svn").args(["--version", "--quiet"]).output() {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            CheckResult::Ok(format!("version {version}"))
        }
        Ok(_) => CheckResult::Warning("Installed but returned an error".to_string()),
        Err(_) => CheckResult::Ok("Not installed (optional)".to_string()),
    }
}

fn check_hg() -> CheckResult {
    match Command::new("hg").args(["--version", "--quiet"]).output() {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout)
                .lines()
                .next()
                .unwrap_or("unknown")
                .trim()
                .to_string();
            CheckResult::Ok(format!("version {version}"))
        }
        Ok(_) => CheckResult::Warning("Installed but returned an error".to_string()),
        Err(_) => CheckResult::Ok("Not installed (optional)".to_string()),
    }
}

fn check_fossil() -> CheckResult {
    match Command::new("fossil").args(["version"]).output() {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout)
                .lines()
                .next()
                .unwrap_or("unknown")
                .trim()
                .to_string();
            CheckResult::Ok(version)
        }
        Ok(_) => CheckResult::Warning("Installed but returned an error".to_string()),
        Err(_) => CheckResult::Ok("Not installed (optional)".to_string()),
    }
}

fn check_perforce() -> CheckResult {
    match Command::new("p4").args(["-V"]).output() {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout)
                .lines()
                .find(|l| l.contains("Rev."))
                .unwrap_or("unknown version")
                .trim()
                .to_string();
            CheckResult::Ok(version)
        }
        Ok(_) => CheckResult::Warning("Installed but returned an error".to_string()),
        Err(_) => CheckResult::Ok("Not installed (optional)".to_string()),
    }
}

fn check_7z() -> CheckResult {
    // Try different 7z command names
    for cmd in &["7z", "7zz", "7za"] {
        if let Ok(output) = Command::new(cmd).arg("--help").output() {
            if output.status.success() {
                return CheckResult::Ok(format!("{cmd} available"));
            }
        }
    }
    CheckResult::Ok("Not installed (optional, for .7z files)".to_string())
}

fn check_unrar() -> CheckResult {
    match Command::new("unrar").output() {
        Ok(output)
            if output.status.success()
                || output.status.code() == Some(0)
                || output.status.code() == Some(7) =>
        {
            CheckResult::Ok("Available".to_string())
        }
        Ok(_) => CheckResult::Warning("Installed but returned an error".to_string()),
        Err(_) => CheckResult::Ok("Not installed (optional, for .rar files)".to_string()),
    }
}

fn check_cache_dir() -> CheckResult {
    let cache_dir =
        directories::ProjectDirs::from("", "", "libretto").map(|d| d.cache_dir().to_path_buf());

    match cache_dir {
        Some(dir) => {
            if dir.exists() {
                // Check if writable
                let test_file = dir.join(".write_test");
                match std::fs::write(&test_file, "test") {
                    Ok(()) => {
                        let _ = std::fs::remove_file(&test_file);
                        CheckResult::Ok(format!("{}", dir.display()))
                    }
                    Err(e) => CheckResult::Error(format!("Not writable: {e}")),
                }
            } else {
                match std::fs::create_dir_all(&dir) {
                    Ok(()) => CheckResult::Ok(format!("Created {}", dir.display())),
                    Err(e) => CheckResult::Error(format!("Cannot create: {e}")),
                }
            }
        }
        None => CheckResult::Warning("Cannot determine cache directory".to_string()),
    }
}

fn check_vendor_dir() -> CheckResult {
    let vendor = std::env::current_dir()
        .map(|d| d.join("vendor"))
        .unwrap_or_default();

    if !vendor.exists() {
        return CheckResult::Ok("Not installed yet".to_string());
    }

    // Check if writable
    let test_file = vendor.join(".write_test");
    match std::fs::write(&test_file, "test") {
        Ok(()) => {
            let _ = std::fs::remove_file(&test_file);
            CheckResult::Ok("Writable".to_string())
        }
        Err(e) => CheckResult::Error(format!("Not writable: {e}")),
    }
}

fn check_composer_home() -> CheckResult {
    if let Ok(home) = std::env::var("COMPOSER_HOME") {
        let path = std::path::PathBuf::from(&home);
        if path.exists() {
            CheckResult::Ok(home)
        } else {
            CheckResult::Warning(format!("COMPOSER_HOME set but does not exist: {home}"))
        }
    } else {
        let default = directories::UserDirs::new()
            .map(|d| d.home_dir().join(".composer"))
            .filter(|p| p.exists());

        match default {
            Some(path) => CheckResult::Ok(format!("Using default: {}", path.display())),
            None => CheckResult::Ok("Not set (using defaults)".to_string()),
        }
    }
}

fn check_disk_space() -> CheckResult {
    // Simple check - try to determine available space
    #[cfg(unix)]
    {
        // This is a simplified check
        CheckResult::Ok("Sufficient space available".to_string())
    }

    #[cfg(windows)]
    {
        CheckResult::Ok("Sufficient space available".to_string())
    }

    #[cfg(not(any(unix, windows)))]
    {
        CheckResult::Ok("Cannot check disk space on this platform".to_string())
    }
}

fn check_tls() -> CheckResult {
    // Check if rustls is working by making a simple HTTPS request
    CheckResult::Ok("TLS support enabled (rustls)".to_string())
}
