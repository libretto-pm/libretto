//! Check platform requirements command.

use anyhow::Result;
use clap::Args;
use libretto_core::is_platform_package_name;
use sonic_rs::{JsonContainerTrait, JsonValueTrait};
use std::collections::HashMap;
use std::process::Command;

/// Arguments for the check-platform-reqs command
#[derive(Args, Debug, Clone)]
pub struct CheckPlatformReqsArgs {
    /// Only check platform requirements from lock file
    #[arg(long)]
    pub lock: bool,

    /// Do not check PHP version
    #[arg(long)]
    pub no_check_php: bool,

    /// Output format (text, json)
    #[arg(short = 'f', long, default_value = "text")]
    pub format: String,
}

/// Platform requirement check result
#[derive(Debug, Clone)]
struct PlatformCheck {
    name: String,
    required: String,
    installed: Option<String>,
    satisfied: bool,
}

/// Run the check-platform-reqs command
pub async fn run(args: CheckPlatformReqsArgs) -> Result<()> {
    use crate::output::table::Table;
    use crate::output::{error, header, info, success};

    header("Checking platform requirements");

    let composer_path = std::env::current_dir()?.join("composer.json");
    let lock_path = std::env::current_dir()?.join("composer.lock");

    // Collect platform requirements
    let mut requirements: HashMap<String, String> = HashMap::new();

    if args.lock {
        if !lock_path.exists() {
            anyhow::bail!("composer.lock not found");
        }
        let lock_content = std::fs::read_to_string(&lock_path)?;
        let lock: sonic_rs::Value = sonic_rs::from_str(&lock_content)?;

        // Get platform requirements from lock
        if let Some(platform) = lock.get("platform").and_then(|v| v.as_object()) {
            for (name, version) in platform {
                if let Some(v) = version.as_str() {
                    requirements.insert(name.to_string(), v.to_string());
                }
            }
        }
    } else {
        if !composer_path.exists() {
            anyhow::bail!("composer.json not found");
        }
        let composer_content = std::fs::read_to_string(&composer_path)?;
        let composer: sonic_rs::Value = sonic_rs::from_str(&composer_content)?;

        // Get requirements from require section
        if let Some(require) = composer.get("require").and_then(|v| v.as_object()) {
            for (name, version) in require {
                if is_platform_package_name(name)
                    && let Some(v) = version.as_str()
                {
                    requirements.insert(name.to_string(), v.to_string());
                }
            }
        }

        // Get platform config
        if let Some(config) = composer.get("config").and_then(|v| v.as_object())
            && let Some(platform) = config
                .get(&"platform".to_string())
                .and_then(|v| v.as_object())
        {
            for (name, version) in platform {
                if let Some(v) = version.as_str() {
                    requirements.insert(name.to_string(), v.to_string());
                }
            }
        }
    }

    if requirements.is_empty() {
        info("No platform requirements found");
        return Ok(());
    }

    // Check each requirement
    let mut checks: Vec<PlatformCheck> = Vec::new();

    for (name, required) in &requirements {
        let installed = get_installed_version(name, args.no_check_php);
        let satisfied = installed
            .as_ref()
            .is_some_and(|v| check_constraint(v, required));

        checks.push(PlatformCheck {
            name: name.clone(),
            required: required.clone(),
            installed,
            satisfied,
        });
    }

    // Sort by name
    checks.sort_by(|a, b| a.name.cmp(&b.name));

    // Output results
    if args.format == "json" {
        let output: Vec<_> = checks
            .iter()
            .map(|c| {
                sonic_rs::json!({
                    "name": c.name,
                    "required": c.required,
                    "installed": c.installed,
                    "satisfied": c.satisfied
                })
            })
            .collect();
        println!("{}", sonic_rs::to_string_pretty(&output)?);
        return Ok(());
    }

    // Table output
    let mut table = Table::new();
    table.headers(["Requirement", "Required", "Installed", "Status"]);

    let mut all_satisfied = true;
    for check in &checks {
        let installed = check.installed.as_deref().unwrap_or("missing");
        let status = if check.satisfied { "OK" } else { "FAIL" };

        if !check.satisfied {
            all_satisfied = false;
        }

        let status_cell = if check.satisfied {
            table.success_cell(status)
        } else {
            table.error_cell(status)
        };

        let installed_cell = if check.installed.is_some() {
            comfy_table::Cell::new(installed)
        } else {
            table.warning_cell(installed)
        };

        table.styled_row(vec![
            comfy_table::Cell::new(&check.name),
            comfy_table::Cell::new(&check.required),
            installed_cell,
            status_cell,
        ]);
    }

    table.print();
    println!();

    if all_satisfied {
        success("All platform requirements are satisfied");
        Ok(())
    } else {
        error("Some platform requirements are not satisfied");
        std::process::exit(2);
    }
}

/// Get installed version of a platform package
fn get_installed_version(name: &str, no_check_php: bool) -> Option<String> {
    if name == "php" {
        if no_check_php {
            return None;
        }
        return get_php_version();
    }

    if let Some(ext) = name.strip_prefix("ext-") {
        return get_extension_version(ext);
    }

    if let Some(lib) = name.strip_prefix("lib-") {
        return get_library_version(lib);
    }

    None
}

/// Get PHP version
fn get_php_version() -> Option<String> {
    Command::new("php")
        .args(["-r", "echo PHP_VERSION;"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        })
}

/// Get PHP extension version
fn get_extension_version(ext: &str) -> Option<String> {
    let code = format!("echo phpversion('{ext}') ?: (extension_loaded('{ext}') ? '1.0.0' : '');");
    Command::new("php")
        .args(["-r", &code])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                let version = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if version.is_empty() {
                    None
                } else {
                    Some(version)
                }
            } else {
                None
            }
        })
}

/// Get library version (simplified)
fn get_library_version(lib: &str) -> Option<String> {
    // This is a simplified implementation
    // Real implementation would check pkg-config, etc.
    match lib {
        "openssl" => Command::new("openssl")
            .args(["version"])
            .output()
            .ok()
            .and_then(|o| {
                if o.status.success() {
                    let output = String::from_utf8_lossy(&o.stdout);
                    output.split_whitespace().nth(1).map(String::from)
                } else {
                    None
                }
            }),
        "curl" => Command::new("curl")
            .args(["--version"])
            .output()
            .ok()
            .and_then(|o| {
                if o.status.success() {
                    let output = String::from_utf8_lossy(&o.stdout);
                    output.split_whitespace().nth(1).map(String::from)
                } else {
                    None
                }
            }),
        _ => None,
    }
}

/// Check if installed version satisfies constraint
fn check_constraint(installed: &str, constraint: &str) -> bool {
    // Parse installed version
    let installed_ver = match semver::Version::parse(installed)
        .or_else(|_| semver::Version::parse(&format!("{installed}.0.0")))
        .or_else(|_| semver::Version::parse(&format!("{installed}.0")))
    {
        Ok(v) => v,
        Err(_) => return true, // Be lenient
    };

    // Parse constraint
    let constraint = constraint.replace("||", " || ").replace(',', " ");

    // Simple constraint checking
    for part in constraint.split("||").map(str::trim) {
        if check_single_constraint(&installed_ver, part) {
            return true;
        }
    }

    false
}

fn check_single_constraint(version: &semver::Version, constraint: &str) -> bool {
    let constraint = constraint.trim();

    if constraint == "*" {
        return true;
    }

    if let Some(rest) = constraint.strip_prefix(">=") {
        let req_ver = parse_version(rest.trim());
        return req_ver.is_none_or(|r| version >= &r);
    }

    if let Some(rest) = constraint.strip_prefix("<=") {
        let req_ver = parse_version(rest.trim());
        return req_ver.is_none_or(|r| version <= &r);
    }

    if let Some(rest) = constraint.strip_prefix('>') {
        let req_ver = parse_version(rest.trim());
        return req_ver.is_none_or(|r| version > &r);
    }

    if let Some(rest) = constraint.strip_prefix('<') {
        let req_ver = parse_version(rest.trim());
        return req_ver.is_none_or(|r| version < &r);
    }

    if let Some(rest) = constraint.strip_prefix('^') {
        let req_ver = parse_version(rest.trim());
        return req_ver.is_none_or(|r| version.major == r.major && version >= &r);
    }

    if let Some(rest) = constraint.strip_prefix('~') {
        let req_ver = parse_version(rest.trim());
        return req_ver
            .is_none_or(|r| version.major == r.major && version.minor == r.minor && version >= &r);
    }

    // Exact match
    let req_ver = parse_version(constraint);
    req_ver.is_none_or(|r| version == &r)
}

fn parse_version(s: &str) -> Option<semver::Version> {
    let s = s.trim_start_matches('v');
    semver::Version::parse(s)
        .or_else(|_| semver::Version::parse(&format!("{s}.0.0")))
        .or_else(|_| semver::Version::parse(&format!("{s}.0")))
        .ok()
}
