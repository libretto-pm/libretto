//! Dump-autoload command implementation.

use crate::output::warning;
use crate::scripts::{ScriptConfig, run_post_autoload_scripts, run_pre_autoload_scripts};
use anyhow::Result;
use clap::Args;
use console::style;
use libretto_autoloader::{AutoloadConfig, AutoloaderGenerator, OptimizationLevel};
use serde::Deserialize;
use sonic_rs::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;
use tracing::{debug, info};

/// Arguments for the dump-autoload command.
#[derive(Args, Debug, Clone)]
pub struct DumpAutoloadArgs {
    /// Optimize autoloader for production
    #[arg(short, long)]
    pub optimize: bool,

    /// Convert PSR-0/PSR-4 to classmap
    #[arg(short, long)]
    pub classmap_authoritative: bool,

    /// `APCu` caching
    #[arg(long)]
    pub apcu: bool,

    /// Skip scripts execution
    #[arg(long)]
    pub no_scripts: bool,
}

/// Composer.json structure for reading autoload config.
#[derive(Debug, Deserialize)]
struct ComposerJson {
    #[serde(default)]
    autoload: AutoloadSection,
    #[serde(default, rename = "autoload-dev")]
    autoload_dev: AutoloadSection,
    #[serde(default)]
    config: ComposerConfig,
}

#[derive(Debug, Default, Deserialize)]
struct AutoloadSection {
    #[serde(default, rename = "psr-4")]
    psr4: HashMap<String, Psr4Value>,
    #[serde(default, rename = "psr-0")]
    psr0: HashMap<String, Psr4Value>,
    #[serde(default)]
    classmap: Vec<String>,
    #[serde(default)]
    files: Vec<String>,
    #[serde(default, rename = "exclude-from-classmap")]
    exclude: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ComposerConfig {
    #[serde(default, rename = "optimize-autoloader")]
    optimize_autoloader: bool,
    #[serde(default, rename = "classmap-authoritative")]
    classmap_authoritative: bool,
}

/// PSR-4 value can be either a string or array of strings.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Psr4Value {
    Single(String),
    Multiple(Vec<String>),
}

impl Psr4Value {
    fn to_vec(&self) -> Vec<String> {
        match self {
            Self::Single(s) => vec![s.clone()],
            Self::Multiple(v) => v.clone(),
        }
    }
}

/// Run the dump-autoload command.
pub async fn run(args: DumpAutoloadArgs) -> Result<()> {
    info!("running dump-autoload command");

    let start_time = Instant::now();
    let root_composer_json = PathBuf::from("composer.json");
    let composer_config = load_composer_config(&root_composer_json).unwrap_or_default();
    let composer_json_value = load_composer_json_value(&root_composer_json);

    let vendor_dir = PathBuf::from("vendor");
    if !vendor_dir.exists() {
        std::fs::create_dir_all(&vendor_dir)?;
    }

    // Determine optimization level
    let classmap_authoritative =
        args.classmap_authoritative || composer_config.classmap_authoritative;
    let optimize = if classmap_authoritative {
        true
    } else {
        args.optimize || composer_config.optimize_autoloader
    };

    let optimization_level = if classmap_authoritative {
        OptimizationLevel::Authoritative
    } else if optimize {
        OptimizationLevel::Optimized
    } else {
        OptimizationLevel::None
    };

    let is_optimized = optimization_level >= OptimizationLevel::Optimized;

    if is_optimized {
        println!(
            "{} Generating {}autoload files",
            style("Libretto").cyan().bold(),
            style("optimized ").yellow()
        );
    } else {
        println!(
            "{} Generating autoload files",
            style("Libretto").cyan().bold()
        );
    }

    let mut generator =
        AutoloaderGenerator::with_optimization(vendor_dir.clone(), optimization_level);

    // Scan vendor directory for installed packages and load their autoload configs
    let mut package_count = 0;
    if let Ok(entries) = std::fs::read_dir(&vendor_dir) {
        for entry in entries.filter_map(Result::ok) {
            let vendor_path = entry.path();
            if vendor_path.is_dir() {
                // Skip composer directory
                if vendor_path.file_name().is_some_and(|n| n == "composer") {
                    continue;
                }

                // Each subdirectory is a vendor namespace (e.g., "monolog", "guzzlehttp")
                if let Ok(package_entries) = std::fs::read_dir(&vendor_path) {
                    for package_entry in package_entries.filter_map(Result::ok) {
                        let package_path = package_entry.path();
                        if package_path.is_dir() {
                            let composer_json_path = package_path.join("composer.json");
                            if composer_json_path.exists()
                                && let Some(config) = load_autoload_config(&composer_json_path)
                            {
                                debug!(
                                    "Loaded autoload config from {:?}: psr4={}, files={}",
                                    composer_json_path,
                                    config.psr4.mappings.len(),
                                    config.files.files.len()
                                );
                                generator.add_package(&package_path, &config);
                                package_count += 1;
                            }
                        }
                    }
                }
            }
        }
    }

    // Also load root project's autoload config if exists (including autoload-dev)
    if root_composer_json.exists()
        && let Some(config) = load_autoload_config_with_dev(&root_composer_json)
    {
        let project_root = PathBuf::from(".");
        generator.add_package(&project_root, &config);
        debug!("Loaded root project autoload config (with dev)");
    }

    info!("Loaded autoload configs from {} packages", package_count);

    // Pre-autoload-dump scripts
    if !args.no_scripts
        && let Some(composer_json) = composer_json_value.as_ref()
    {
        let script_config = ScriptConfig {
            working_dir: std::env::current_dir()?,
            ..Default::default()
        };

        if let Some(result) = run_pre_autoload_scripts(composer_json, &script_config)?
            && !result.success
            && let Some(ref err) = result.error
        {
            warning(&format!("Pre-autoload script warning: {err}"));
        }
    }

    match generator.generate() {
        Ok(()) => {
            for psr_warning in generator.warnings() {
                warning(psr_warning);
            }

            // Post-autoload-dump scripts
            if !args.no_scripts
                && let Some(composer_json) = composer_json_value.as_ref()
            {
                let script_config = ScriptConfig {
                    working_dir: std::env::current_dir()?,
                    ..Default::default()
                };

                if let Some(result) = run_post_autoload_scripts(composer_json, &script_config)?
                    && !result.success
                    && let Some(ref err) = result.error
                {
                    warning(&format!("Post-autoload script warning: {err}"));
                }
            }

            let stats = generator.stats();
            let elapsed = start_time.elapsed();

            // Calculate total classes (PSR-4 classes are lazily loaded, so we show classmap count)
            let total_classes = stats.classmap_entries;

            println!(
                "Generated {}autoload files containing {} classes",
                if is_optimized {
                    style("optimized ").yellow().to_string()
                } else {
                    String::new()
                },
                style(total_classes).green().bold()
            );

            // Show detailed breakdown
            println!();
            println!(
                "   {} PSR-4 namespaces registered",
                style(format!("{:>4}", stats.psr4_namespaces)).cyan()
            );
            println!(
                "   {} classmap entries generated",
                style(format!("{:>4}", stats.classmap_entries)).cyan()
            );
            println!(
                "   {} files to include",
                style(format!("{:>4}", stats.files_count)).cyan()
            );
            println!(
                "   {} packages scanned",
                style(format!("{package_count:>4}")).cyan()
            );

            // Show timing
            let elapsed_ms = elapsed.as_secs_f64() * 1000.0;
            println!();
            println!("   {} {:.1}ms", style("Done in").dim(), elapsed_ms);
        }
        Err(e) => {
            eprintln!();
            eprintln!(
                "  {} {}",
                style("ERROR").red().bold(),
                style("Failed to generate autoloader").red()
            );
            eprintln!();
            eprintln!("  {e}");
            eprintln!();
            std::process::exit(1);
        }
    }

    Ok(())
}

/// Load full composer.json as dynamic value for scripts execution.
fn load_composer_json_value(path: &PathBuf) -> Option<Value> {
    let content = std::fs::read_to_string(path).ok()?;
    sonic_rs::from_str(&content).ok()
}

/// Load Composer config defaults (e.g., optimize-autoloader).
fn load_composer_config(path: &PathBuf) -> Option<ComposerConfig> {
    let content = std::fs::read_to_string(path).ok()?;
    let composer: ComposerJson = sonic_rs::from_str(&content).ok()?;
    Some(composer.config)
}

/// Load autoload configuration from a composer.json file (production only).
fn load_autoload_config(path: &PathBuf) -> Option<AutoloadConfig> {
    let content = std::fs::read_to_string(path).ok()?;
    let composer: ComposerJson = sonic_rs::from_str(&content).ok()?;

    let mut config = AutoloadConfig::default();

    // Convert PSR-4 mappings (production only, like Composer)
    for (namespace, paths) in composer.autoload.psr4 {
        config.psr4.mappings.insert(namespace, paths.to_vec());
    }

    // Convert PSR-0 mappings (production only)
    for (namespace, paths) in composer.autoload.psr0 {
        config.psr0.mappings.insert(namespace, paths.to_vec());
    }

    // Classmap paths (production only)
    config.classmap.paths = composer.autoload.classmap;

    // Files (production only)
    config.files.files = composer.autoload.files;

    // Exclude patterns
    config.exclude.patterns = composer.autoload.exclude;

    Some(config)
}

/// Load autoload configuration including dev section (for root project only).
fn load_autoload_config_with_dev(path: &PathBuf) -> Option<AutoloadConfig> {
    let content = std::fs::read_to_string(path).ok()?;
    let composer: ComposerJson = sonic_rs::from_str(&content).ok()?;

    let mut config = AutoloadConfig::default();

    // Production autoload
    for (namespace, paths) in composer.autoload.psr4 {
        config.psr4.mappings.insert(namespace, paths.to_vec());
    }
    for (namespace, paths) in composer.autoload.psr0 {
        config.psr0.mappings.insert(namespace, paths.to_vec());
    }
    config.classmap.paths = composer.autoload.classmap;
    config.files.files = composer.autoload.files;
    config.exclude.patterns = composer.autoload.exclude;

    // Dev autoload (only for root project)
    for (namespace, paths) in composer.autoload_dev.psr4 {
        config
            .psr4
            .mappings
            .entry(namespace)
            .or_default()
            .extend(paths.to_vec());
    }
    for (namespace, paths) in composer.autoload_dev.psr0 {
        config
            .psr0
            .mappings
            .entry(namespace)
            .or_default()
            .extend(paths.to_vec());
    }
    config.classmap.paths.extend(composer.autoload_dev.classmap);
    config.files.files.extend(composer.autoload_dev.files);
    config
        .exclude
        .patterns
        .extend(composer.autoload_dev.exclude);

    Some(config)
}
