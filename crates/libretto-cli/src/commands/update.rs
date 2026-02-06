//! Update command implementation.

use crate::fetcher::Fetcher;
use crate::output::{info, success};
use crate::scripts::{ScriptConfig, run_post_install_scripts, run_pre_install_scripts};
use anyhow::Result;
use clap::Args;
use libretto_resolver::turbo::{TurboConfig, TurboResolver};
use libretto_resolver::{ComposerConstraint, Dependency, PackageName, ResolutionMode, Stability};
use owo_colors::OwoColorize;
use sonic_rs::{JsonContainerTrait, JsonValueTrait};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Arguments for the update command.
#[derive(Args, Debug, Clone)]
pub struct UpdateArgs {
    /// Packages to update (all if empty)
    #[arg(value_name = "PACKAGE")]
    pub packages: Vec<String>,

    /// Skip dev dependencies
    #[arg(long)]
    pub no_dev: bool,

    /// Prefer lowest versions
    #[arg(long)]
    pub prefer_lowest: bool,

    /// Prefer stable versions
    #[arg(long)]
    pub prefer_stable: bool,

    /// Dry run (don't update anything)
    #[arg(long)]
    pub dry_run: bool,

    /// Only update root dependencies
    #[arg(long)]
    pub root_reqs: bool,

    /// Lock file only (don't install)
    #[arg(long)]
    pub lock: bool,

    /// Run security audit after update
    #[arg(long)]
    pub audit: bool,

    /// Fail update if security vulnerabilities are found
    #[arg(long)]
    pub fail_on_audit: bool,
}

/// A categorized lock file change.
enum LockOp {
    Install {
        name: String,
        version: String,
    },
    Upgrade {
        name: String,
        from: String,
        to: String,
    },
    Downgrade {
        name: String,
        from: String,
        to: String,
    },
    Remove {
        name: String,
        version: String,
    },
}

impl LockOp {
    /// Print a single operation line with rich formatting.
    fn print(&self) {
        let colors = crate::output::colors_enabled();
        match self {
            Self::Install { name, version } => {
                if colors {
                    println!(
                        "    {} {} {}",
                        "+".green().bold(),
                        name.green(),
                        format!("({version})").dimmed(),
                    );
                } else {
                    println!("    + {name} ({version})");
                }
            }
            Self::Upgrade { name, from, to } => {
                if colors {
                    println!(
                        "    {} {} {} {} {}",
                        "^".cyan().bold(),
                        name.cyan(),
                        from.dimmed(),
                        "->".dimmed(),
                        to.green(),
                    );
                } else {
                    println!("    ^ {name} {from} -> {to}");
                }
            }
            Self::Downgrade { name, from, to } => {
                if colors {
                    println!(
                        "    {} {} {} {} {}",
                        "v".yellow().bold(),
                        name.yellow(),
                        from.dimmed(),
                        "->".dimmed(),
                        to.yellow(),
                    );
                } else {
                    println!("    v {name} {from} -> {to}");
                }
            }
            Self::Remove { name, version } => {
                if colors {
                    println!(
                        "    {} {} {}",
                        "-".red().bold(),
                        name.red(),
                        format!("({version})").dimmed(),
                    );
                } else {
                    println!("    - {name} ({version})");
                }
            }
        }
    }
}

/// Run the update command.
pub async fn run(args: UpdateArgs) -> Result<()> {
    use crate::output::progress::Spinner;
    use crate::output::{header, success, warning};

    header("Updating dependencies");

    let cwd = std::env::current_dir()?;
    let composer_path = cwd.join("composer.json");
    let lock_path = cwd.join("composer.lock");

    if !composer_path.exists() {
        anyhow::bail!("composer.json not found in current directory");
    }

    let composer_content = std::fs::read_to_string(&composer_path)?;
    let composer: sonic_rs::Value = sonic_rs::from_str(&composer_content)?;

    if args.dry_run {
        warning("Dry run mode - no changes will be made");
    }

    // Set up script configuration
    let script_config = ScriptConfig {
        working_dir: cwd.clone(),
        dev_mode: !args.no_dev,
        ..Default::default()
    };

    // Run pre-update-cmd scripts
    if !args.dry_run {
        if let Some(result) = run_pre_install_scripts(&composer, &script_config, true)? {
            if !result.success {
                warning(&format!(
                    "Pre-update script warning: {}",
                    result.error.unwrap_or_default()
                ));
            }
        }
    }

    // Collect current locked versions
    let mut current_versions: HashMap<String, String> = HashMap::new();
    if lock_path.exists() {
        let lock_content = std::fs::read_to_string(&lock_path)?;
        let lock: sonic_rs::Value = sonic_rs::from_str(&lock_content)?;

        for key in ["packages", "packages-dev"] {
            if let Some(packages) = lock.get(key).and_then(|v| v.as_array()) {
                for pkg in packages {
                    let name = pkg.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    let version = pkg.get("version").and_then(|v| v.as_str()).unwrap_or("");
                    current_versions.insert(name.to_string(), version.to_string());
                }
            }
        }
    }

    // Configure resolver
    let min_stability = composer
        .get("minimum-stability")
        .and_then(|v| v.as_str())
        .and_then(parse_stability)
        .unwrap_or(Stability::Stable);

    let config = TurboConfig {
        max_concurrent: 64,
        request_timeout: std::time::Duration::from_secs(10),
        mode: if args.prefer_lowest {
            ResolutionMode::PreferLowest
        } else {
            ResolutionMode::PreferStable
        },
        min_stability,
        include_dev: !args.no_dev,
    };

    // Parse dependencies
    let mut root_deps = Vec::new();
    let mut dev_deps = Vec::new();

    if let Some(require) = composer.get("require").and_then(|v| v.as_object()) {
        for (name, constraint) in require {
            if is_platform_package(name) {
                continue;
            }
            if let (Some(n), Some(c)) = (
                PackageName::parse(name),
                ComposerConstraint::parse(constraint.as_str().unwrap_or("*")),
            ) {
                root_deps.push(Dependency::new(n, c));
            }
        }
    }

    if !args.no_dev {
        if let Some(require_dev) = composer.get("require-dev").and_then(|v| v.as_object()) {
            for (name, constraint) in require_dev {
                if is_platform_package(name) {
                    continue;
                }
                if let (Some(n), Some(c)) = (
                    PackageName::parse(name),
                    ComposerConstraint::parse(constraint.as_str().unwrap_or("*")),
                ) {
                    dev_deps.push(Dependency::new(n, c));
                }
            }
        }
    }

    // Resolve dependencies
    let fetcher =
        Arc::new(Fetcher::new().map_err(|e| anyhow::anyhow!("Failed to create fetcher: {e}"))?);
    let resolver = TurboResolver::new(fetcher.clone(), config);

    let spinner = Spinner::new("Resolving dependencies...");
    let resolution = resolver
        .resolve(&root_deps, &dev_deps)
        .await
        .map_err(|e| anyhow::anyhow!("Resolution failed: {e}"))?;
    spinner.finish_and_clear();

    // Build set of resolved package names for removal detection
    let resolved_names: HashSet<String> = resolution
        .packages
        .iter()
        .map(|p| p.name.as_str().to_string())
        .collect();

    // Categorize all changes
    let mut ops: Vec<LockOp> = Vec::new();

    for pkg in &resolution.packages {
        let name = pkg.name.as_str();
        let new_ver = pkg.version.to_string();

        match current_versions.get(name) {
            None => {
                ops.push(LockOp::Install {
                    name: name.to_string(),
                    version: new_ver,
                });
            }
            Some(old_ver) => {
                let old_norm = old_ver.trim_start_matches('v');
                let new_norm = new_ver.trim_start_matches('v');
                if old_norm != new_norm {
                    // Compare versions to determine upgrade vs downgrade
                    let is_upgrade = new_norm > old_norm;
                    if is_upgrade {
                        ops.push(LockOp::Upgrade {
                            name: name.to_string(),
                            from: old_ver.clone(),
                            to: new_ver,
                        });
                    } else {
                        ops.push(LockOp::Downgrade {
                            name: name.to_string(),
                            from: old_ver.clone(),
                            to: new_ver,
                        });
                    }
                }
            }
        }
    }

    // Detect removals: packages in old lock but not in new resolution
    for (name, version) in &current_versions {
        if !resolved_names.contains(name) {
            ops.push(LockOp::Remove {
                name: name.clone(),
                version: version.clone(),
            });
        }
    }

    // Sort operations: installs first, then upgrades, downgrades, removals
    ops.sort_by_key(|op| match op {
        LockOp::Install { name, .. } => (0, name.clone()),
        LockOp::Upgrade { name, .. } => (1, name.clone()),
        LockOp::Downgrade { name, .. } => (2, name.clone()),
        LockOp::Remove { name, .. } => (3, name.clone()),
    });

    // Count by category
    let n_install = ops
        .iter()
        .filter(|o| matches!(o, LockOp::Install { .. }))
        .count();
    let n_upgrade = ops
        .iter()
        .filter(|o| matches!(o, LockOp::Upgrade { .. }))
        .count();
    let n_downgrade = ops
        .iter()
        .filter(|o| matches!(o, LockOp::Downgrade { .. }))
        .count();
    let n_remove = ops
        .iter()
        .filter(|o| matches!(o, LockOp::Remove { .. }))
        .count();

    if ops.is_empty() {
        println!();
        success("Nothing to modify in lock file");
    } else {
        // Summary line
        let mut parts: Vec<String> = Vec::new();
        if n_install > 0 {
            parts.push(format!(
                "{n_install} install{}",
                if n_install == 1 { "" } else { "s" }
            ));
        }
        if n_upgrade > 0 {
            parts.push(format!(
                "{n_upgrade} upgrade{}",
                if n_upgrade == 1 { "" } else { "s" }
            ));
        }
        if n_downgrade > 0 {
            parts.push(format!(
                "{n_downgrade} downgrade{}",
                if n_downgrade == 1 { "" } else { "s" }
            ));
        }
        if n_remove > 0 {
            parts.push(format!(
                "{n_remove} removal{}",
                if n_remove == 1 { "" } else { "s" }
            ));
        }

        println!();
        if crate::output::colors_enabled() {
            println!(
                "  {} {}",
                "Lock file operations:".white().bold(),
                parts.join(", ").dimmed(),
            );
        } else {
            println!("  Lock file operations: {}", parts.join(", "));
        }
        println!();

        for op in &ops {
            op.print();
        }
        println!();
    }

    if args.dry_run {
        warning("Dry run - no changes made");
        return Ok(());
    }

    // Write lock file
    crate::commands::lock_generator::generate_lock_file(&lock_path, &resolution, &composer)?;
    if !ops.is_empty() {
        success("Writing lock file");
    }

    // Install packages
    if !args.lock {
        let install_args = crate::commands::install::InstallArgs {
            no_dev: args.no_dev,
            prefer_dist: true,
            prefer_source: false,
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

        crate::commands::install::run(install_args).await?;
    }

    // Run post-update-cmd scripts
    if let Some(result) = run_post_install_scripts(&composer, &script_config, true)? {
        if !result.success {
            warning(&format!(
                "Post-update script warning: {}",
                result.error.unwrap_or_default()
            ));
        }
    }

    // Show funding info (like Composer)
    let funded_count = resolution
        .packages
        .iter()
        .filter(|p| {
            p.funding
                .as_ref()
                .is_some_and(|f| f.is_array() && !f.as_array().unwrap().is_empty())
        })
        .count();
    if funded_count > 0 {
        if crate::output::colors_enabled() {
            println!(
                "{} {}",
                format!(
                    "{funded_count} package{} you are using {} looking for funding.",
                    if funded_count == 1 { "" } else { "s" },
                    if funded_count == 1 { "is" } else { "are" },
                )
                .dimmed(),
                "Use the `libretto fund` command to find out more.".dimmed(),
            );
        } else {
            println!(
                "{funded_count} package{} you are using {} looking for funding.\n\
                 Use the `libretto fund` command to find out more.",
                if funded_count == 1 { "" } else { "s" },
                if funded_count == 1 { "is" } else { "are" },
            );
        }
    }

    // Always run security audit (like Composer does by default)
    run_security_audit(&lock_path, args.fail_on_audit).await?;

    Ok(())
}

/// Run security audit on packages in lock file.
async fn run_security_audit(lock_path: &std::path::Path, fail_on_audit: bool) -> Result<()> {
    use libretto_audit::Auditor;
    use libretto_core::PackageId;
    use semver::Version;

    if !lock_path.exists() {
        return Ok(());
    }

    info("Running security audit...");

    let lock_content = std::fs::read_to_string(lock_path)?;
    let lock: sonic_rs::Value = sonic_rs::from_str(&lock_content)?;

    let mut packages_to_audit: Vec<(PackageId, Version)> = Vec::new();

    for key in ["packages", "packages-dev"] {
        if let Some(pkgs) = lock.get(key).and_then(|v| v.as_array()) {
            for pkg in pkgs {
                let name = pkg.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let version_str = pkg
                    .get("version")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim_start_matches('v');

                if let Some(id) = PackageId::parse(name)
                    && let Ok(ver) = Version::parse(version_str)
                {
                    packages_to_audit.push((id, ver));
                }
            }
        }
    }

    if packages_to_audit.is_empty() {
        return Ok(());
    }

    let auditor = Auditor::new().map_err(|e| anyhow::anyhow!("Failed to create auditor: {e}"))?;
    let report = auditor
        .audit(&packages_to_audit)
        .await
        .map_err(|e| anyhow::anyhow!("Audit failed: {e}"))?;

    if report.vulnerability_count() == 0 {
        success("No security vulnerability advisories found");
        return Ok(());
    }

    // Compact severity summary: e.g. "Found 3 advisories: 1 critical, 2 high"
    let by_sev = report.by_severity();
    let mut sev_parts: Vec<String> = Vec::new();
    for (severity, vulns) in &by_sev {
        sev_parts.push(format!(
            "{} {}",
            vulns.len(),
            severity.as_str().to_lowercase()
        ));
    }

    let colors = crate::output::colors_enabled();
    let vuln_count = report.vulnerability_count();
    let pkg_count = report.vulnerable_package_count();

    if colors {
        println!(
            "  {} Found {} {} affecting {} {}: {}",
            "!".yellow().bold(),
            vuln_count,
            if vuln_count == 1 {
                "advisory"
            } else {
                "advisories"
            },
            pkg_count,
            if pkg_count == 1 {
                "package"
            } else {
                "packages"
            },
            sev_parts.join(", "),
        );
    } else {
        println!(
            "  ! Found {} {} affecting {} {}: {}",
            vuln_count,
            if vuln_count == 1 {
                "advisory"
            } else {
                "advisories"
            },
            pkg_count,
            if pkg_count == 1 {
                "package"
            } else {
                "packages"
            },
            sev_parts.join(", "),
        );
    }

    if colors {
        println!("    {}", "Run `libretto audit` for details.".dimmed(),);
    } else {
        println!("    Run `libretto audit` for details.");
    }

    if fail_on_audit && report.has_critical() {
        anyhow::bail!("Critical security vulnerabilities found.");
    }

    if fail_on_audit && !report.passes() {
        anyhow::bail!("Security vulnerabilities found.");
    }

    Ok(())
}

fn parse_stability(s: &str) -> Option<Stability> {
    match s.to_lowercase().as_str() {
        "dev" => Some(Stability::Dev),
        "alpha" => Some(Stability::Alpha),
        "beta" => Some(Stability::Beta),
        "rc" => Some(Stability::RC),
        "stable" => Some(Stability::Stable),
        _ => None,
    }
}

fn is_platform_package(name: &str) -> bool {
    name == "php"
        || name.starts_with("php-")
        || name.starts_with("ext-")
        || name.starts_with("lib-")
        || name.starts_with("api-")
}
