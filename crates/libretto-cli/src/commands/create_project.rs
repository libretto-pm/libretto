//! Create-project command - bootstrap new projects from packages.

use anyhow::{Context, Result};
use clap::Args;
use std::path::PathBuf;

use crate::scripts::{self, ScriptConfig};

/// Arguments for the create-project command
#[derive(Args, Debug, Clone)]
pub struct CreateProjectArgs {
    /// Package to create project from (vendor/name format)
    #[arg(required = true, value_name = "PACKAGE")]
    pub package: String,

    /// Directory to create project in
    #[arg(value_name = "DIRECTORY")]
    pub directory: Option<String>,

    /// Version constraint to use
    #[arg(value_name = "VERSION", name = "pkg_version")]
    pub version: Option<String>,

    /// Minimum stability to allow (stable, RC, beta, alpha, dev)
    #[arg(short = 's', long, default_value = "stable")]
    pub stability: String,

    /// Prefer source packages (from VCS)
    #[arg(long)]
    pub prefer_source: bool,

    /// Prefer dist packages (archives)
    #[arg(long)]
    pub prefer_dist: bool,

    /// Forces installation from source even for stable versions
    #[arg(long)]
    pub repository: Option<String>,

    /// Add a custom repository URL
    #[arg(long)]
    pub add_repository: Option<String>,

    /// Disables installation of require-dev packages
    #[arg(long)]
    pub no_dev: bool,

    /// Skip vendor directory installation
    #[arg(long)]
    pub no_install: bool,

    /// Run non-interactively
    #[arg(long)]
    pub no_interaction: bool,

    /// Keep the VCS history
    #[arg(long)]
    pub keep_vcs: bool,

    /// Remove the VCS directory
    #[arg(long)]
    pub remove_vcs: bool,

    /// Ignore platform requirements
    #[arg(long)]
    pub ignore_platform_reqs: bool,

    /// Ask before removing old files/directories
    #[arg(long)]
    pub ask: bool,
}

/// Run the create-project command
pub async fn run(args: CreateProjectArgs) -> Result<()> {
    use crate::output::progress::Spinner;
    use crate::output::{header, info, success, warning};
    use libretto_repository::Repository;

    header("Creating project");

    // Parse package name
    let package_id =
        libretto_core::PackageId::parse(&args.package).context("Invalid package name")?;

    info(&format!("Package: {}", args.package));

    // Determine project directory
    let project_name = args
        .directory
        .unwrap_or_else(|| package_id.name().to_string());
    let project_dir = PathBuf::from(&project_name);

    info(&format!("Directory: {}", project_dir.display()));

    // Check if directory exists
    if project_dir.exists() && project_dir.read_dir()?.next().is_some() {
        if args.ask {
            use crate::output::prompt::Confirm;
            let confirm = Confirm::new(format!(
                "Directory '{}' already exists and is not empty. Remove existing files?",
                project_dir.display()
            ))
            .default(false)
            .prompt()?;

            if !confirm {
                anyhow::bail!("Aborted");
            }
        } else {
            anyhow::bail!(
                "Directory '{}' already exists and is not empty",
                project_dir.display()
            );
        }
    }

    // Create project directory
    std::fs::create_dir_all(&project_dir)?;

    // Fetch package info
    let spinner = Spinner::new("Fetching package information...");
    let repo = Repository::packagist()?;
    repo.init_packagist().await?;
    let package_versions = repo.get_package(&package_id).await?;
    spinner.finish_and_clear();

    // Find the best matching version
    let version_constraint = args.version.as_deref().unwrap_or("*");
    info(&format!("Version constraint: {version_constraint}"));

    let constraint = libretto_core::VersionConstraint::new(version_constraint);
    let versions: Vec<_> = package_versions.into_iter().collect();

    let selected_version = versions
        .iter()
        .find(|v| constraint.matches(&v.version))
        .or_else(|| versions.first())
        .context("No matching version found")?;

    info(&format!("Selected version: {}", selected_version.version));

    // Download the package
    let spinner = Spinner::new("Downloading package...");

    // Get the dist URL
    let dist_url = selected_version
        .dist
        .as_ref()
        .map(|d| match d {
            libretto_core::PackageSource::Dist { url, .. } => url.to_string(),
            libretto_core::PackageSource::Git { url, .. } => url.to_string(),
        })
        .context("No distribution URL found for package")?;

    // Download to temp file
    let temp_dir = tempfile::tempdir()?;
    let archive_path = temp_dir.path().join("package.zip");

    let client = reqwest::Client::builder()
        .user_agent("libretto/0.1.0")
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()?;
    let response = client.get(&dist_url).send().await?;

    // Check for successful response
    if !response.status().is_success() {
        anyhow::bail!(
            "Failed to download package: HTTP {} from {}",
            response.status(),
            dist_url
        );
    }

    let bytes = response.bytes().await?;
    std::fs::write(&archive_path, &bytes)?;

    spinner.finish_and_clear();
    info(&format!(
        "Downloaded: {}",
        crate::output::format_bytes(bytes.len() as u64)
    ));

    // Extract the archive
    let spinner = Spinner::new("Extracting package...");

    let archive_file = std::fs::File::open(&archive_path)?;
    let mut archive = zip::ZipArchive::new(archive_file)?;

    // Find the common prefix (most packages have a top-level directory)
    let prefix = archive
        .file_names()
        .next()
        .and_then(|name| name.split('/').next())
        .map(String::from);

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let name = file.name().to_string();

        // Strip the prefix if present
        let relative_path = if let Some(ref prefix) = prefix {
            name.strip_prefix(prefix)
                .and_then(|s| s.strip_prefix('/'))
                .unwrap_or(&name)
        } else {
            &name
        };

        if relative_path.is_empty() {
            continue;
        }

        let target_path = project_dir.join(relative_path);

        if file.is_dir() {
            std::fs::create_dir_all(&target_path)?;
        } else {
            if let Some(parent) = target_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut outfile = std::fs::File::create(&target_path)?;
            std::io::copy(&mut file, &mut outfile)?;
        }
    }

    spinner.finish_and_clear();

    // Remove .git directory if requested
    let git_dir = project_dir.join(".git");
    if git_dir.exists() && (args.remove_vcs || !args.keep_vcs) {
        info("Removing .git directory...");
        std::fs::remove_dir_all(&git_dir)?;
    }

    // Install dependencies unless --no-install
    if !args.no_install {
        info("Installing dependencies...");

        let composer_json = project_dir.join("composer.json");
        if composer_json.exists() {
            // Change to project directory and run install
            std::env::set_current_dir(&project_dir)?;

            let install_args = crate::commands::install::InstallArgs {
                no_dev: args.no_dev,
                prefer_dist: args.prefer_dist,
                prefer_source: args.prefer_source,
                dry_run: false,
                ignore_platform_reqs: args.ignore_platform_reqs,
                ignore_platform_req: vec![],
                optimize_autoloader: false,
                classmap_authoritative: false,
                apcu_autoloader: false,
                no_scripts: false,
                prefer_lowest: false,
                prefer_stable: true,
                minimum_stability: Some(args.stability.clone()),
                no_progress: false,
                concurrency: 64,
                audit: false,
                fail_on_audit: false,
                verify_checksums: false,
            };

            crate::commands::install::run(install_args).await?;
        } else {
            warning("No composer.json found in project, skipping dependency installation");
        }
    }

    // Run post-create-project scripts
    // Note: We're already in the project directory after install, so use current_dir
    let current_dir = std::env::current_dir()?;
    let composer_json_path = current_dir.join("composer.json");
    if composer_json_path.exists() {
        let composer_content = std::fs::read_to_string(&composer_json_path)?;
        let composer_json: sonic_rs::Value = sonic_rs::from_str(&composer_content)?;

        let script_config = ScriptConfig {
            working_dir: current_dir.clone(),
            dev_mode: !args.no_dev,
            ..Default::default()
        };

        // Run post-root-package-install scripts
        if let Some(result) =
            scripts::run_root_package_install_scripts(&composer_json, &script_config)?
        {
            if !result.success {
                warning(&format!(
                    "Post-root-package-install script warning: {}",
                    result.error.unwrap_or_default()
                ));
            }
        }

        // Run post-create-project-cmd scripts
        if let Some(result) = scripts::run_create_project_scripts(&composer_json, &script_config)? {
            if !result.success {
                warning(&format!(
                    "Post-create-project script warning: {}",
                    result.error.unwrap_or_default()
                ));
            }
        }
    }

    success(&format!("Project '{project_name}' created successfully!"));

    // Print next steps
    println!();
    info("Next steps:");
    println!("  cd {project_name}");
    if args.no_install {
        println!("  libretto install");
    }

    Ok(())
}
