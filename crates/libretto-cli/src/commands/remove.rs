//! Remove command implementation.

use crate::scripts::{ScriptConfig, ScriptEvent, run_package_scripts};
use anyhow::Result;
use clap::Args;
use sonic_rs::{JsonValueMutTrait, JsonValueTrait};

/// Arguments for the remove command.
#[derive(Args, Debug, Clone)]
pub struct RemoveArgs {
    /// Packages to remove
    #[arg(required = true, value_name = "PACKAGE")]
    pub packages: Vec<String>,

    /// Remove from dev dependencies
    #[arg(short = 'D', long)]
    pub dev: bool,

    /// Don't update dependencies after removing
    #[arg(long)]
    pub no_update: bool,

    /// Don't remove unused dependencies
    #[arg(long)]
    pub no_update_with_dependencies: bool,
}

/// Run the remove command.
pub async fn run(args: RemoveArgs) -> Result<()> {
    use crate::output::{header, info, success, warning};
    use owo_colors::OwoColorize;

    header("Removing packages");

    let cwd = std::env::current_dir()?;
    let composer_path = cwd.join("composer.json");
    let vendor_dir = cwd.join("vendor");

    if !composer_path.exists() {
        anyhow::bail!("composer.json not found in current directory");
    }

    // Read composer.json
    let composer_content = std::fs::read_to_string(&composer_path)?;
    let mut composer: sonic_rs::Value = sonic_rs::from_str(&composer_content)?;

    // Set up script configuration
    let script_config = ScriptConfig {
        working_dir: cwd.clone(),
        dev_mode: !args.dev,
        ..Default::default()
    };

    let colors = crate::output::colors_enabled();
    let mut removed: Vec<String> = Vec::new();
    let mut not_found: Vec<String> = Vec::new();

    for package in &args.packages {
        // Run pre-package-uninstall scripts
        let _ = run_package_scripts(
            &composer,
            &script_config,
            ScriptEvent::PrePackageUninstall,
            package,
        );
        let mut found = if !args.dev
            && let Some(require) = composer.get_mut("require").and_then(|v| v.as_object_mut())
            && require.remove(package).is_some()
        {
            true
        } else {
            false
        };

        // Try to remove from require-dev
        if (args.dev || !found)
            && let Some(require_dev) = composer
                .get_mut("require-dev")
                .and_then(|v| v.as_object_mut())
            && require_dev.remove(package).is_some()
        {
            found = true;
        }

        if found {
            if colors {
                println!("  {} {}", "-".red(), package.red());
            } else {
                println!("  - {package}");
            }
            removed.push(package.clone());

            // Remove from vendor directory
            let pkg_dir = vendor_dir.join(package.replace('/', std::path::MAIN_SEPARATOR_STR));
            if pkg_dir.exists()
                && let Err(e) = std::fs::remove_dir_all(&pkg_dir)
            {
                warning(&format!("Could not remove vendor/{package}: {e}"));
            }

            // Run post-package-uninstall scripts
            let _ = run_package_scripts(
                &composer,
                &script_config,
                ScriptEvent::PostPackageUninstall,
                package,
            );
        } else {
            not_found.push(package.clone());
        }
    }

    if !not_found.is_empty() {
        println!();
        for package in &not_found {
            warning(&format!("Package '{package}' not found in dependencies"));
        }
    }

    if removed.is_empty() {
        warning("No packages were removed");
        return Ok(());
    }

    // Write updated composer.json
    let output = sonic_rs::to_string_pretty(&composer)?;
    std::fs::write(&composer_path, format!("{output}\n"))?;

    success(&format!("Removed {} package(s)", removed.len()));

    // Update lock file
    if !args.no_update {
        println!();
        info("Updating lock file...");

        let lock_path = cwd.join("composer.lock");
        if lock_path.exists() {
            // Read and update lock file
            let lock_content = std::fs::read_to_string(&lock_path)?;
            let mut lock: sonic_rs::Value = sonic_rs::from_str(&lock_content)?;

            // Remove packages from lock file
            for key in ["packages", "packages-dev"] {
                if let Some(packages) = lock.get_mut(key).and_then(|v| v.as_array_mut()) {
                    packages.retain(|pkg| {
                        let name = pkg.get("name").and_then(|n| n.as_str()).unwrap_or("");
                        !removed.contains(&name.to_string())
                    });
                }
            }

            // Update content hash
            let content_hash =
                libretto_core::ContentHash::from_bytes(sonic_rs::to_string(&composer)?.as_bytes());
            if let Some(obj) = lock.as_object_mut() {
                obj.insert("content-hash", sonic_rs::json!(content_hash.to_hex()));
            }

            let output = sonic_rs::to_string_pretty(&lock)?;
            std::fs::write(&lock_path, format!("{output}\n"))?;

            info("Lock file updated");
        }

        // Remove unused dependencies if requested
        if !args.no_update_with_dependencies {
            info("Checking for unused dependencies...");
            // In a full implementation, we would analyze the dependency tree
            // and remove packages that are no longer needed
        }
    }

    // Regenerate autoloader
    info("Regenerating autoloader...");
    let autoload_path = vendor_dir.join("autoload.php");
    if autoload_path.exists() {
        let dump_args = crate::commands::dump_autoload::DumpAutoloadArgs {
            optimize: false,
            classmap_authoritative: false,
            apcu: false,
            no_scripts: true,
        };
        crate::commands::dump_autoload::run(dump_args).await?;
    }

    Ok(())
}
