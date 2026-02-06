//! Reinstall command - uninstall and reinstall packages.

use anyhow::Result;
use clap::Args;
use sonic_rs::{JsonContainerTrait, JsonValueTrait};

/// Arguments for the reinstall command
#[derive(Args, Debug, Clone)]
pub struct ReinstallArgs {
    /// Packages to reinstall (all if omitted)
    #[arg(value_name = "PACKAGE")]
    pub packages: Vec<String>,

    /// Prefer source packages (from VCS)
    #[arg(long)]
    pub prefer_source: bool,

    /// Prefer dist packages (archives)
    #[arg(long)]
    pub prefer_dist: bool,

    /// Skip dev dependencies
    #[arg(long)]
    pub no_dev: bool,

    /// Skip autoloader generation
    #[arg(long)]
    pub no_autoloader: bool,

    /// Skip script execution
    #[arg(long)]
    pub no_scripts: bool,
}

/// Run the reinstall command
pub async fn run(args: ReinstallArgs) -> Result<()> {
    use crate::output::progress::Spinner;
    use crate::output::{header, info, success, warning};

    header("Reinstalling packages");

    let lock_path = std::env::current_dir()?.join("composer.lock");
    let vendor_dir = std::env::current_dir()?.join("vendor");

    if !lock_path.exists() {
        anyhow::bail!("composer.lock not found - run 'libretto install' first");
    }

    let lock_content = std::fs::read_to_string(&lock_path)?;
    let lock: sonic_rs::Value = sonic_rs::from_str(&lock_content)?;

    // Collect packages to reinstall
    let mut packages_to_reinstall: Vec<(String, String, bool)> = Vec::new();

    for (key, is_dev) in [("packages", false), ("packages-dev", true)] {
        if args.no_dev && is_dev {
            continue;
        }

        if let Some(packages) = lock.get(key).and_then(|v| v.as_array()) {
            for pkg in packages {
                let name = pkg.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let version = pkg.get("version").and_then(|v| v.as_str()).unwrap_or("");

                if !name.is_empty() {
                    // Filter by package names if specified
                    if args.packages.is_empty()
                        || args.packages.iter().any(|p| {
                            name == p
                                || name.contains(p)
                                || p.contains('*') && matches_glob(name, p)
                        })
                    {
                        packages_to_reinstall.push((name.to_string(), version.to_string(), is_dev));
                    }
                }
            }
        }
    }

    if packages_to_reinstall.is_empty() {
        warning("No packages to reinstall");
        return Ok(());
    }

    info(&format!(
        "Reinstalling {} package(s)...",
        packages_to_reinstall.len()
    ));

    // Remove packages
    let spinner = Spinner::new("Removing packages...");

    for (name, _, _) in &packages_to_reinstall {
        let package_dir = vendor_dir.join(name.replace('/', std::path::MAIN_SEPARATOR_STR));
        if package_dir.exists() {
            std::fs::remove_dir_all(&package_dir)?;
        }
    }

    spinner.finish_with_message("Packages removed");

    // Reinstall packages
    let spinner = Spinner::new("Installing packages...");

    // Re-run install command
    let install_args = crate::commands::install::InstallArgs {
        no_dev: args.no_dev,
        prefer_dist: args.prefer_dist,
        prefer_source: args.prefer_source,
        dry_run: false,
        ignore_platform_reqs: false,
        ignore_platform_req: vec![],
        optimize_autoloader: false,
        classmap_authoritative: false,
        apcu_autoloader: false,
        no_scripts: false,
        prefer_lowest: false,
        prefer_stable: true,
        minimum_stability: None,
        no_progress: false,
        concurrency: 64,
        audit: false,
        fail_on_audit: false,
        verify_checksums: false,
        php_version: None,
        no_php_check: false,
    };

    spinner.finish_and_clear();

    // Run install
    crate::commands::install::run(install_args).await?;

    success(&format!(
        "Reinstalled {} package(s)",
        packages_to_reinstall.len()
    ));

    Ok(())
}

fn matches_glob(name: &str, pattern: &str) -> bool {
    let regex_pattern = pattern
        .replace('.', "\\.")
        .replace('*', ".*")
        .replace('?', ".");
    regex::Regex::new(&format!("^{regex_pattern}$"))
        .map(|r| r.is_match(name))
        .unwrap_or(false)
}
