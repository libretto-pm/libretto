//! Prohibits command - show what prevents a package from being installed.

use anyhow::Result;
use clap::Args;
use sonic_rs::{JsonContainerTrait, JsonValueTrait};
use std::collections::HashMap;

/// Arguments for the prohibits command
#[derive(Args, Debug, Clone)]
pub struct ProhibitsArgs {
    /// Package to check (vendor/name format)
    #[arg(required = true, value_name = "PACKAGE")]
    pub package: String,

    /// Version to check
    #[arg(value_name = "VERSION", name = "pkg_version")]
    pub version: Option<String>,

    /// Recursively resolve up to the root packages
    #[arg(short = 'r', long)]
    pub recursive: bool,

    /// Show tree view
    #[arg(short = 't', long)]
    pub tree: bool,
}

/// Run the prohibits command
pub async fn run(args: ProhibitsArgs) -> Result<()> {
    use crate::output::table::Table;
    use crate::output::{header, info, warning};

    header("Conflict analysis");

    let lock_path = std::env::current_dir()?.join("composer.lock");
    let composer_path = std::env::current_dir()?.join("composer.json");

    // Build constraint map from all packages
    let mut constraints: HashMap<String, Vec<(String, String, bool)>> = HashMap::new();

    // Check root composer.json
    if composer_path.exists() {
        let composer_content = std::fs::read_to_string(&composer_path)?;
        let composer: sonic_rs::Value = sonic_rs::from_str(&composer_content)?;
        let root_name = composer
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("__root__");

        // Check require
        if let Some(require) = composer.get("require").and_then(|v| v.as_object()) {
            for (dep_name, constraint) in require {
                let constraint_str = constraint.as_str().unwrap_or("*");
                constraints.entry(dep_name.to_string()).or_default().push((
                    root_name.to_string(),
                    constraint_str.to_string(),
                    false,
                ));
            }
        }

        // Check require-dev
        if let Some(require_dev) = composer.get("require-dev").and_then(|v| v.as_object()) {
            for (dep_name, constraint) in require_dev {
                let constraint_str = constraint.as_str().unwrap_or("*");
                constraints.entry(dep_name.to_string()).or_default().push((
                    root_name.to_string(),
                    constraint_str.to_string(),
                    true,
                ));
            }
        }

        // Check conflict section
        if let Some(conflict) = composer.get("conflict").and_then(|v| v.as_object()) {
            for (dep_name, constraint) in conflict {
                let constraint_str = constraint.as_str().unwrap_or("*");
                constraints.entry(dep_name.to_string()).or_default().push((
                    format!("{root_name} (conflict)"),
                    format!("conflicts with {constraint_str}"),
                    false,
                ));
            }
        }
    }

    // Check composer.lock
    if lock_path.exists() {
        let lock_content = std::fs::read_to_string(&lock_path)?;
        let lock: sonic_rs::Value = sonic_rs::from_str(&lock_content)?;

        for (packages_key, is_dev) in [("packages", false), ("packages-dev", true)] {
            if let Some(packages) = lock.get(packages_key).and_then(|v| v.as_array()) {
                for pkg in packages {
                    let name = pkg.get("name").and_then(|v| v.as_str()).unwrap_or("");

                    // Check require
                    if let Some(require) = pkg.get("require").and_then(|v| v.as_object()) {
                        for (dep_name, constraint) in require {
                            let constraint_str = constraint.as_str().unwrap_or("*");
                            constraints.entry(dep_name.to_string()).or_default().push((
                                name.to_string(),
                                constraint_str.to_string(),
                                is_dev,
                            ));
                        }
                    }

                    // Check conflict
                    if let Some(conflict) = pkg.get("conflict").and_then(|v| v.as_object()) {
                        for (dep_name, constraint) in conflict {
                            let constraint_str = constraint.as_str().unwrap_or("*");
                            constraints.entry(dep_name.to_string()).or_default().push((
                                format!("{name} (conflict)"),
                                format!("conflicts with {constraint_str}"),
                                is_dev,
                            ));
                        }
                    }

                    // Check replace
                    if let Some(replace) = pkg.get("replace").and_then(|v| v.as_object()) {
                        for (dep_name, _) in replace {
                            constraints.entry(dep_name.to_string()).or_default().push((
                                format!("{name} (replace)"),
                                "replaced by this package".to_string(),
                                is_dev,
                            ));
                        }
                    }
                }
            }
        }
    }

    // Find constraints that affect the target package
    let target = args.package.to_lowercase();
    let target_constraints = constraints.get(&target);

    if target_constraints.is_none() || target_constraints.is_none_or(std::vec::Vec::is_empty) {
        info(&format!(
            "No constraints found for '{}' - it could be installed freely",
            args.package
        ));
        return Ok(());
    }

    let target_constraints = target_constraints.unwrap();

    // Check which constraints are problematic for the requested version
    let requested_version = args.version.as_deref().unwrap_or("latest");
    info(&format!(
        "Checking what prevents installing {} {}",
        args.package, requested_version
    ));
    println!();

    let colors = crate::output::colors_enabled();

    // Find conflicting constraints
    let mut conflicts: Vec<(&str, &str, bool, bool)> = Vec::new();

    for (source, constraint, is_dev) in target_constraints {
        let is_conflict = constraint.contains("conflict")
            || constraint.contains("replace")
            || !version_satisfies(requested_version, constraint);

        if is_conflict || args.tree {
            conflicts.push((source.as_str(), constraint.as_str(), *is_dev, is_conflict));
        }
    }

    if conflicts.is_empty() {
        info(&format!(
            "No constraints prevent installing {} {}",
            args.package, requested_version
        ));
        return Ok(());
    }

    // Output conflicts
    if args.tree {
        print_tree(&args.package, &conflicts, colors);
    } else {
        let mut table = Table::new();
        table.headers(["Package", "Constraint", "Type", "Blocks?"]);

        for (source, constraint, is_dev, is_conflict) in &conflicts {
            let pkg_type = if *is_dev { "dev" } else { "prod" };
            let blocks = if *is_conflict { "Yes" } else { "No" };

            let blocks_cell = if *is_conflict {
                table.error_cell(blocks)
            } else {
                table.success_cell(blocks)
            };

            table.styled_row(vec![
                comfy_table::Cell::new(source),
                comfy_table::Cell::new(constraint),
                comfy_table::Cell::new(pkg_type),
                blocks_cell,
            ]);
        }

        table.print();
    }

    // Count blocking constraints
    let blocking_count = conflicts.iter().filter(|(_, _, _, c)| *c).count();
    println!();

    if blocking_count > 0 {
        warning(&format!(
            "{} package(s) have constraints that may prevent installing {} {}",
            blocking_count, args.package, requested_version
        ));
    } else {
        info(&format!(
            "All constraints allow installing {} {}",
            args.package, requested_version
        ));
    }

    Ok(())
}

fn print_tree(package: &str, conflicts: &[(&str, &str, bool, bool)], colors: bool) {
    use owo_colors::OwoColorize;

    let unicode = crate::output::unicode_enabled();

    if colors {
        println!("{}", package.cyan().bold());
    } else {
        println!("{package}");
    }

    for (i, (source, constraint, is_dev, is_conflict)) in conflicts.iter().enumerate() {
        let is_last = i == conflicts.len() - 1;
        let prefix = if unicode {
            if is_last {
                "\u{2514}\u{2500}\u{2500}"
            } else {
                "\u{251C}\u{2500}\u{2500}"
            }
        } else if is_last {
            "`--"
        } else {
            "|--"
        };

        let dev_marker = if *is_dev { " (dev)" } else { "" };
        let block_marker = if *is_conflict { " [BLOCKS]" } else { "" };

        if colors {
            let constraint_colored = if *is_conflict {
                constraint.red().to_string()
            } else {
                constraint.green().to_string()
            };
            println!(
                " {} {} {} {}{}",
                prefix,
                source.green(),
                constraint_colored,
                dev_marker.dimmed(),
                if *is_conflict {
                    block_marker.red().bold().to_string()
                } else {
                    String::new()
                }
            );
        } else {
            println!(" {prefix} {source} {constraint} {dev_marker}{block_marker}");
        }
    }
}

fn version_satisfies(version: &str, constraint: &str) -> bool {
    // Simplified version matching
    if constraint == "*" || version == "latest" {
        return true;
    }

    if constraint.contains("conflict") || constraint.contains("replace") {
        return false;
    }

    // Try to parse and compare
    let constraint = libretto_core::VersionConstraint::new(constraint);
    if let Ok(version) = semver::Version::parse(version.trim_start_matches('v')) {
        constraint.matches(&version)
    } else {
        // If we can't parse the version, assume it might match
        true
    }
}
