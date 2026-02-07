//! Validate command implementation.

use anyhow::Result;
use clap::Args;
use console::style;
use libretto_core::from_json;
use libretto_core::is_platform_package_name;
use serde::Deserialize;
use std::path::Path;
use tracing::info;

/// Arguments for the validate command.
#[derive(Args, Debug, Clone)]
pub struct ValidateArgs {
    /// Check composer.lock too
    #[arg(long)]
    pub with_dependencies: bool,

    /// Strict mode (warnings as errors)
    #[arg(long)]
    pub strict: bool,

    /// Don't validate require(-dev) versions
    #[arg(long)]
    pub no_check_version: bool,
}

#[derive(Debug, Deserialize)]
struct ComposerJson {
    name: Option<String>,
    #[serde(default)]
    require: std::collections::HashMap<String, String>,
}

/// Run the validate command.
pub async fn run(args: ValidateArgs) -> Result<()> {
    info!("running validate command");

    println!(
        "{} {}",
        style("Libretto").cyan().bold(),
        style("Validating composer.json...").dim()
    );

    let composer_json = Path::new("composer.json");
    if !composer_json.exists() {
        println!("{} composer.json not found", style("Error:").red().bold());
        return Ok(());
    }

    let content = std::fs::read_to_string(composer_json)?;
    let mut errors: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    // Parse JSON
    let parsed: Result<ComposerJson, _> = from_json(&content);
    match parsed {
        Ok(composer) => {
            // Check required fields
            if composer.name.is_none() {
                warnings.push("'name' is not set".to_string());
            }

            // Validate package names
            for name in composer.require.keys() {
                if !name.contains('/') && !is_platform_requirement(name) {
                    errors.push(format!("Invalid package name: {name}"));
                }
            }
        }
        Err(e) => {
            errors.push(format!("Invalid JSON: {e}"));
        }
    }

    // Check lock file if requested
    if args.with_dependencies {
        let lock_path = Path::new("composer.lock");
        if !lock_path.exists() {
            warnings.push("composer.lock not found".to_string());
        }
    }

    // Report results
    println!();
    if errors.is_empty() && warnings.is_empty() {
        println!(
            "{} composer.json is valid",
            style("Success:").green().bold()
        );
    } else {
        for error in &errors {
            println!("{} {}", style("Error:").red().bold(), error);
        }
        for warning in &warnings {
            println!("{} {}", style("Warning:").yellow().bold(), warning);
        }

        if !errors.is_empty() || (args.strict && !warnings.is_empty()) {
            println!();
            println!("{}", style("Validation failed").red().bold());
        }
    }

    Ok(())
}

fn is_platform_requirement(name: &str) -> bool {
    is_platform_package_name(name)
}

#[cfg(test)]
mod tests {
    use super::is_platform_requirement;

    #[test]
    fn platform_requirement_detection_matches_composer_rules() {
        assert!(is_platform_requirement("php"));
        assert!(is_platform_requirement("php-64bit"));
        assert!(is_platform_requirement("composer-plugin-api"));
        assert!(is_platform_requirement("composer-runtime-api"));
        assert!(is_platform_requirement("ext-json"));
        assert!(is_platform_requirement("lib-icu-uc"));

        assert!(!is_platform_requirement("php-open-source-saver/jwt-auth"));
        assert!(!is_platform_requirement("php-http/discovery"));
    }
}
