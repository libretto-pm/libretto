//! Global command - run commands in global composer directory.

use anyhow::Result;
use clap::Args;
use std::path::PathBuf;

/// Arguments for the global command
#[derive(Args, Debug, Clone)]
pub struct GlobalArgs {
    /// Command to run (require, remove, update, etc.)
    #[arg(required = true, value_name = "COMMAND")]
    pub command: String,

    /// Arguments for the command
    #[arg(trailing_var_arg = true, value_name = "ARGS")]
    pub args: Vec<String>,
}

/// Run the global command
pub async fn run(args: GlobalArgs) -> Result<()> {
    use crate::output::header;

    // Get global composer directory
    let global_dir = get_global_dir()?;

    header(&format!("Global ({})", global_dir.display()));

    // Ensure global directory exists
    std::fs::create_dir_all(&global_dir)?;

    // Ensure composer.json exists
    let composer_json = global_dir.join("composer.json");
    if !composer_json.exists() {
        std::fs::write(
            &composer_json,
            r#"{
    "name": "libretto/global",
    "description": "Global packages managed by Libretto",
    "type": "project",
    "license": "MIT",
    "require": {}
}
"#,
        )?;
    }

    // Change to global directory
    let original_dir = std::env::current_dir()?;
    std::env::set_current_dir(&global_dir)?;

    // Run the sub-command
    let result = run_subcommand(&args.command, &args.args).await;

    // Restore original directory
    std::env::set_current_dir(&original_dir)?;

    result
}

fn get_global_dir() -> Result<PathBuf> {
    // Check COMPOSER_HOME first
    if let Ok(home) = std::env::var("COMPOSER_HOME") {
        return Ok(PathBuf::from(home));
    }

    // Fall back to default
    let home = directories::UserDirs::new()
        .map(|d| d.home_dir().to_path_buf())
        .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;

    Ok(home.join(".composer"))
}

async fn run_subcommand(command: &str, args: &[String]) -> Result<()> {
    use crate::commands;

    match command {
        "require" | "r" => {
            let mut require_args = commands::require::RequireArgs {
                packages: args.to_vec(),
                dev: false,
                no_update: false,
                dry_run: false,
                prefer_stable: true,
                sort_packages: true,
            };

            // Parse flags from args
            let mut packages = Vec::new();
            let iter = args.iter();
            for arg in iter {
                match arg.as_str() {
                    "--dev" | "-D" => require_args.dev = true,
                    "--no-update" => require_args.no_update = true,
                    "--dry-run" => require_args.dry_run = true,
                    _ if !arg.starts_with('-') => packages.push(arg.clone()),
                    _ => {}
                }
            }
            require_args.packages = packages;

            commands::require::run(require_args).await
        }

        "remove" | "rm" => {
            let mut remove_args = commands::remove::RemoveArgs {
                packages: args.to_vec(),
                dev: false,
                no_update: false,
                no_update_with_dependencies: false,
            };

            let mut packages = Vec::new();
            for arg in args {
                match arg.as_str() {
                    "--dev" | "-D" => remove_args.dev = true,
                    "--no-update" => remove_args.no_update = true,
                    _ if !arg.starts_with('-') => packages.push(arg.clone()),
                    _ => {}
                }
            }
            remove_args.packages = packages;

            commands::remove::run(remove_args).await
        }

        "update" | "u" => {
            let update_args = commands::update::UpdateArgs {
                packages: args
                    .iter()
                    .filter(|a| !a.starts_with('-'))
                    .cloned()
                    .collect(),
                no_dev: args.contains(&"--no-dev".to_string()),
                prefer_lowest: args.contains(&"--prefer-lowest".to_string()),
                prefer_stable: args.contains(&"--prefer-stable".to_string()),
                dry_run: args.contains(&"--dry-run".to_string()),
                root_reqs: args.contains(&"--root-reqs".to_string()),
                lock: args.contains(&"--lock".to_string()),
                audit: args.contains(&"--audit".to_string()),
                fail_on_audit: args.contains(&"--fail-on-audit".to_string()),
            };

            commands::update::run(update_args).await
        }

        "install" | "i" => {
            let install_args = commands::install::InstallArgs {
                no_dev: args.contains(&"--no-dev".to_string()),
                prefer_dist: args.contains(&"--prefer-dist".to_string()),
                prefer_source: args.contains(&"--prefer-source".to_string()),
                dry_run: args.contains(&"--dry-run".to_string()),
                ignore_platform_reqs: args.contains(&"--ignore-platform-reqs".to_string()),
                ignore_platform_req: vec![],
                optimize_autoloader: args.contains(&"-o".to_string())
                    || args.contains(&"--optimize-autoloader".to_string()),
                classmap_authoritative: args.contains(&"-a".to_string())
                    || args.contains(&"--classmap-authoritative".to_string()),
                apcu_autoloader: args.contains(&"--apcu-autoloader".to_string()),
                no_scripts: args.contains(&"--no-scripts".to_string()),
                prefer_lowest: args.contains(&"--prefer-lowest".to_string()),
                prefer_stable: args.contains(&"--prefer-stable".to_string()),
                minimum_stability: None,
                no_progress: args.contains(&"--no-progress".to_string()),
                concurrency: 64,
                audit: args.contains(&"--audit".to_string()),
                fail_on_audit: args.contains(&"--fail-on-audit".to_string()),
                verify_checksums: args.contains(&"--verify-checksums".to_string()),
                php_version: None,
                no_php_check: false,
            };

            commands::install::run(install_args).await
        }

        "show" => {
            let package = args.first().cloned();
            let show_args = commands::show::ShowArgs {
                package,
                installed: args.contains(&"--installed".to_string()),
                available: args.contains(&"--available".to_string()),
                all: args.contains(&"--all".to_string()) || args.contains(&"-A".to_string()),
                tree: args.contains(&"--tree".to_string()) || args.contains(&"-t".to_string()),
                name_only: args.contains(&"--name-only".to_string())
                    || args.contains(&"-N".to_string()),
                path: args.contains(&"--path".to_string()) || args.contains(&"-P".to_string()),
                self_pkg: args.contains(&"--self".to_string()) || args.contains(&"-s".to_string()),
                version: None,
            };

            commands::show::run(show_args).await
        }

        "outdated" => {
            let outdated_args = commands::outdated::OutdatedArgs {
                packages: args
                    .iter()
                    .filter(|a| !a.starts_with('-'))
                    .cloned()
                    .collect(),
                all: args.contains(&"--all".to_string()) || args.contains(&"-a".to_string()),
                direct: args.contains(&"--direct".to_string()) || args.contains(&"-D".to_string()),
                minor_only: args.contains(&"--minor-only".to_string())
                    || args.contains(&"-m".to_string()),
                format: "text".to_string(),
                strict: args.contains(&"--strict".to_string()),
            };

            commands::outdated::run(outdated_args).await
        }

        "dump-autoload" | "dumpautoload" => {
            let dump_args = commands::dump_autoload::DumpAutoloadArgs {
                optimize: args.contains(&"-o".to_string())
                    || args.contains(&"--optimize".to_string()),
                classmap_authoritative: args.contains(&"-c".to_string())
                    || args.contains(&"--classmap-authoritative".to_string()),
                apcu: args.contains(&"--apcu".to_string()),
                no_scripts: args.contains(&"--no-scripts".to_string()),
            };

            commands::dump_autoload::run(dump_args).await
        }

        _ => {
            anyhow::bail!(
                "Unknown global command '{command}'. Available commands: require, remove, update, install, show, outdated, dump-autoload"
            );
        }
    }
}
