//! Install command implementation.
//!
//! High-performance package installation using parallel resolution and downloads.

use crate::auth_manager::{
    AuthManager, GitHubRateLimitInfo, is_github_rate_limit_error, parse_rate_limit_headers,
};
use crate::cas_cache;
use crate::fetcher::Fetcher;
use crate::installer_paths::InstallerPaths;
use crate::output::format_bytes;
use crate::output::live::LiveProgress;
use crate::output::table::Table;
use crate::output::{error, header, info, success, warning};
use crate::platform::PlatformValidator;
use crate::scripts::{
    ScriptConfig, run_post_autoload_scripts, run_post_install_scripts, run_pre_autoload_scripts,
    run_pre_install_scripts,
};
use anyhow::{Context, Result, bail};
use clap::Args;
use futures::stream::{FuturesUnordered, StreamExt};
use libretto_audit::Auditor;
use libretto_config::auth::Credential;
use libretto_core::PackageId;
use libretto_resolver::Stability;
use libretto_resolver::turbo::{TurboConfig, TurboResolver};
use libretto_resolver::{ComposerConstraint, Dependency, PackageName, ResolutionMode};
use semver::Version;
use sonic_rs::{JsonContainerTrait, JsonValueTrait, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use tracing::debug;

/// Arguments for the install command.
#[derive(Args, Debug, Clone)]
pub struct InstallArgs {
    /// Skip dev dependencies
    #[arg(long)]
    pub no_dev: bool,

    /// Prefer dist packages (archives)
    #[arg(long)]
    pub prefer_dist: bool,

    /// Prefer source packages (VCS)
    #[arg(long)]
    pub prefer_source: bool,

    /// Dry run (don't install anything)
    #[arg(long)]
    pub dry_run: bool,

    /// Ignore platform requirements
    #[arg(long)]
    pub ignore_platform_reqs: bool,

    /// Ignore specific platform requirements (e.g., php, ext-*)
    #[arg(long, value_name = "REQ")]
    pub ignore_platform_req: Vec<String>,

    /// Optimize autoloader
    #[arg(short = 'o', long)]
    pub optimize_autoloader: bool,

    /// Generate classmap for PSR-0/4 autoloading
    #[arg(short = 'a', long)]
    pub classmap_authoritative: bool,

    /// `APCu` autoloader caching
    #[arg(long)]
    pub apcu_autoloader: bool,

    /// Skip scripts execution
    #[arg(long)]
    pub no_scripts: bool,

    /// Prefer lowest versions (for testing)
    #[arg(long)]
    pub prefer_lowest: bool,

    /// Prefer stable versions
    #[arg(long)]
    pub prefer_stable: bool,

    /// Minimum stability (dev, alpha, beta, RC, stable)
    #[arg(long, value_name = "STABILITY")]
    pub minimum_stability: Option<String>,

    /// Disable progress bar
    #[arg(long)]
    pub no_progress: bool,

    /// Maximum concurrent HTTP requests
    #[arg(long, default_value = "64")]
    pub concurrency: usize,

    /// Run security audit after installation
    #[arg(long)]
    pub audit: bool,

    /// Fail installation if security vulnerabilities are found
    #[arg(long)]
    pub fail_on_audit: bool,

    /// Verify package checksums and fail on mismatch
    #[arg(long)]
    pub verify_checksums: bool,

    /// Specify PHP version to use for this operation
    #[arg(long, value_name = "VERSION")]
    pub php_version: Option<String>,

    /// Skip PHP version requirement check
    #[arg(long)]
    pub no_php_check: bool,
}

/// Run the install command.
pub async fn run(args: InstallArgs) -> Result<()> {
    let start = Instant::now();
    header("Installing dependencies");

    let cwd = std::env::current_dir()?;
    let composer_json_path = cwd.join("composer.json");
    let composer_lock_path = cwd.join("composer.lock");
    let vendor_dir = cwd.join("vendor");

    // Check for composer.json
    if !composer_json_path.exists() {
        bail!("No composer.json found in current directory.\nRun 'libretto init' to create one.");
    }

    // Read composer.json
    let composer_content =
        std::fs::read_to_string(&composer_json_path).context("Failed to read composer.json")?;
    let composer: Value =
        sonic_rs::from_str(&composer_content).context("Failed to parse composer.json")?;

    if args.dry_run {
        warning("Dry run mode - no changes will be made");
    }

    // Script config for lifecycle hooks
    let script_config = ScriptConfig {
        working_dir: cwd.clone(),
        dev_mode: !args.no_dev,
        ..Default::default()
    };

    // Run pre-install scripts
    if !args.no_scripts
        && !args.dry_run
        && let Some(result) = run_pre_install_scripts(&composer, &script_config, false)?
    {
        if result.success {
            debug!(
                "Pre-install script: {} commands in {}ms",
                result.commands_executed,
                result.duration.as_millis()
            );
        } else if let Some(ref err) = result.error {
            warning(&format!("Pre-install script warning: {err}"));
        }
    }

    // Create live progress display
    let progress = if !args.no_progress && !args.dry_run {
        Some(LiveProgress::new())
    } else {
        None
    };

    // Parse installer-paths from composer.json for custom installation locations
    let installer_paths = InstallerPaths::from_composer(&composer);

    // Check for lock file
    let has_lock = composer_lock_path.exists();

    let result = if has_lock && !args.prefer_lowest {
        install_from_lock(
            &composer_lock_path,
            &vendor_dir,
            &cwd,
            &installer_paths,
            &args,
            progress.as_ref(),
        )
        .await
    } else {
        resolve_and_install(
            &composer,
            &composer_lock_path,
            &vendor_dir,
            &cwd,
            &installer_paths,
            &args,
            progress.as_ref(),
        )
        .await
    };

    // Handle result and finish progress
    match result {
        Ok(()) => {
            let elapsed = start.elapsed();
            if let Some(p) = &progress {
                p.finish_success(&format!(
                    "Installed in {}",
                    crate::output::format_duration(elapsed)
                ));
            } else {
                success(&format!(
                    "Installation complete ({})",
                    crate::output::format_duration(elapsed)
                ));
            }
        }
        Err(e) => {
            if let Some(p) = &progress {
                p.finish_error(&e.to_string());
            }
            return Err(e);
        }
    }

    // Generate autoloader
    if !args.dry_run {
        // Pre-autoload-dump scripts
        if !args.no_scripts
            && let Some(result) = run_pre_autoload_scripts(&composer, &script_config)?
            && !result.success
            && let Some(ref err) = result.error
        {
            warning(&format!("Pre-autoload script warning: {err}"));
        }

        generate_autoloader(&vendor_dir, &args)?;

        // Post-autoload-dump scripts
        if !args.no_scripts
            && let Some(result) = run_post_autoload_scripts(&composer, &script_config)?
            && !result.success
            && let Some(ref err) = result.error
        {
            warning(&format!("Post-autoload script warning: {err}"));
        }
    }

    // Run post-install scripts
    if !args.no_scripts
        && !args.dry_run
        && let Some(result) = run_post_install_scripts(&composer, &script_config, false)?
    {
        if result.success {
            debug!(
                "Post-install script: {} commands in {}ms",
                result.commands_executed,
                result.duration.as_millis()
            );
        } else if let Some(ref err) = result.error {
            warning(&format!("Post-install script warning: {err}"));
        }
    }

    // Run security audit if requested
    if args.audit && !args.dry_run {
        run_security_audit(&composer_lock_path, &args).await?;
    }

    Ok(())
}

/// Run security audit on installed packages.
async fn run_security_audit(lock_path: &PathBuf, args: &InstallArgs) -> Result<()> {
    if !lock_path.exists() {
        return Ok(());
    }

    info("Running security audit...");

    let lock_content = std::fs::read_to_string(lock_path)?;
    let lock: Value = sonic_rs::from_str(&lock_content)?;

    let mut packages_to_audit: Vec<(PackageId, Version)> = Vec::new();

    // Collect packages from lock file
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
        success("No security vulnerabilities found");
        return Ok(());
    }

    // Display vulnerabilities
    warning(&format!(
        "Found {} vulnerabilities in {} packages",
        report.vulnerability_count(),
        report.vulnerable_package_count()
    ));

    for (severity, vulns) in report.by_severity() {
        let color = severity.color();
        let reset = "\x1b[0m";

        for vuln in vulns {
            println!(
                "  {color}[{}]{reset} {} ({})",
                severity, vuln.advisory_id, vuln.package
            );
            println!("    {}", vuln.title);
            if let Some(ref fixed) = vuln.fixed_version {
                println!("    Fixed in: {fixed}");
            }
        }
    }

    // Fail if requested and critical/high vulnerabilities found
    if args.fail_on_audit && report.has_critical() {
        bail!("Critical security vulnerabilities found. Use --no-fail to continue anyway.");
    }

    if args.fail_on_audit && !report.passes() {
        bail!("Security vulnerabilities found. Use --no-fail to continue anyway.");
    }

    Ok(())
}

/// Install from an existing lock file.
async fn install_from_lock(
    lock_path: &PathBuf,
    vendor_dir: &PathBuf,
    base_dir: &std::path::Path,
    installer_paths: &InstallerPaths,
    args: &InstallArgs,
    progress: Option<&LiveProgress>,
) -> Result<()> {
    let lock_content = std::fs::read_to_string(lock_path)?;
    let lock: Value = sonic_rs::from_str(&lock_content)?;

    // Collect packages to install
    let mut packages: Vec<PackageInfo> = Vec::new();

    if let Some(pkgs) = lock.get("packages").and_then(|v| v.as_array()) {
        for pkg in pkgs {
            if let Some(info) = parse_lock_package(pkg, false) {
                packages.push(info);
            }
        }
    }

    if !args.no_dev
        && let Some(pkgs) = lock.get("packages-dev").and_then(|v| v.as_array())
    {
        for pkg in pkgs {
            if let Some(info) = parse_lock_package(pkg, true) {
                packages.push(info);
            }
        }
    }

    if packages.is_empty() {
        info("No packages to install");
        return Ok(());
    }

    // Validate platform requirements
    if !args.ignore_platform_reqs {
        validate_platform_from_lock(&lock, args)?;
    }

    if args.dry_run {
        info(&format!("Would install {} package(s)", packages.len()));
        show_packages_table(&packages);
        return Ok(());
    }

    // Create vendor directory
    std::fs::create_dir_all(vendor_dir)?;

    // Install packages
    install_packages(
        &packages,
        vendor_dir,
        base_dir,
        installer_paths,
        args,
        progress,
    )
    .await?;

    Ok(())
}

/// Resolve dependencies and install.
async fn resolve_and_install(
    composer: &Value,
    lock_path: &PathBuf,
    vendor_dir: &PathBuf,
    base_dir: &std::path::Path,
    installer_paths: &InstallerPaths,
    args: &InstallArgs,
    progress: Option<&LiveProgress>,
) -> Result<()> {
    // Collect requirements from composer.json
    let mut require: HashMap<String, String> = HashMap::new();
    let mut require_dev: HashMap<String, String> = HashMap::new();

    if let Some(req) = composer.get("require").and_then(|v| v.as_object()) {
        for (name, constraint) in req {
            if let Some(c) = constraint.as_str() {
                require.insert(name.to_string(), c.to_string());
            }
        }
    }

    if let Some(req) = composer.get("require-dev").and_then(|v| v.as_object()) {
        for (name, constraint) in req {
            if let Some(c) = constraint.as_str() {
                require_dev.insert(name.to_string(), c.to_string());
            }
        }
    }

    if require.is_empty() && require_dev.is_empty() {
        info("No dependencies to install");
        return Ok(());
    }

    info(&format!(
        "Found {} production and {} dev dependencies",
        require.len(),
        require_dev.len()
    ));

    // Parse minimum stability
    let min_stability = args
        .minimum_stability
        .as_deref()
        .and_then(parse_stability)
        .or_else(|| {
            composer
                .get("minimum-stability")
                .and_then(|v| v.as_str())
                .and_then(parse_stability)
        })
        .unwrap_or(Stability::Stable);

    // Create fetcher
    let fetcher =
        Arc::new(Fetcher::new().map_err(|e| anyhow::anyhow!("Failed to create fetcher: {e}"))?);

    // Configure resolver
    let config = TurboConfig {
        max_concurrent: args.concurrency.max(32),
        request_timeout: std::time::Duration::from_secs(10),
        mode: if args.prefer_lowest {
            ResolutionMode::PreferLowest
        } else {
            // Default to PreferStable like Composer does
            ResolutionMode::PreferStable
        },
        min_stability,
        include_dev: !args.no_dev,
    };

    // Parse dependencies
    let mut root_deps = Vec::new();
    let mut dev_deps = Vec::new();

    for (name, constraint) in &require {
        if is_platform_package(name) {
            continue;
        }
        if let (Some(n), Some(c)) = (
            PackageName::parse(name),
            ComposerConstraint::parse(constraint),
        ) {
            root_deps.push(Dependency::new(n, c));
        }
    }

    for (name, constraint) in &require_dev {
        if is_platform_package(name) {
            continue;
        }
        if let (Some(n), Some(c)) = (
            PackageName::parse(name),
            ComposerConstraint::parse(constraint),
        ) {
            dev_deps.push(Dependency::new(n, c));
        }
    }

    // Resolve dependencies
    if let Some(p) = progress {
        p.set_resolving();
    }

    let resolver = TurboResolver::new(fetcher.clone(), config);
    let resolution = resolver
        .resolve(&root_deps, &dev_deps)
        .await
        .map_err(|e| anyhow::anyhow!("Resolution failed: {e}"))?;

    // Log fetcher statistics
    let stats = fetcher.stats();
    tracing::debug!(
        requests = stats.requests,
        bytes = stats.bytes_downloaded,
        cache_hits = stats.cache_hits,
        cache_hit_rate = format!("{:.1}%", stats.cache_hit_rate()),
        "resolution fetch statistics"
    );

    // Convert to package info
    let packages: Vec<PackageInfo> = resolution
        .packages
        .iter()
        .map(|p| PackageInfo {
            name: p.name.as_str().to_string(),
            version: p.version.to_string(),
            is_dev: p.is_dev,
            dist_url: p.dist_url.clone(),
            dist_shasum: p.dist_shasum.clone(),
            package_type: p.package_type.clone(),
        })
        .collect();

    if args.dry_run {
        info(&format!("Would install {} package(s)", packages.len()));
        show_packages_table(&packages);
        return Ok(());
    }

    // Create vendor directory
    std::fs::create_dir_all(vendor_dir)?;

    // Install packages
    install_packages(
        &packages,
        vendor_dir,
        base_dir,
        installer_paths,
        args,
        progress,
    )
    .await?;

    // Generate lock file
    generate_lock_file(lock_path, &resolution, composer)?;

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
        || name == "composer"
        || name == "composer-plugin-api"
        || name == "composer-runtime-api"
}

/// Package information for installation.
#[derive(Debug, Clone)]
struct PackageInfo {
    name: String,
    version: String,
    is_dev: bool,
    dist_url: Option<String>,
    dist_shasum: Option<String>,
    /// Package type (e.g., "library", "wordpress-plugin", "drupal-module")
    package_type: Option<String>,
}

fn parse_lock_package(pkg: &Value, is_dev: bool) -> Option<PackageInfo> {
    let name = pkg.get("name").and_then(|v| v.as_str())?;
    let version = pkg.get("version").and_then(|v| v.as_str())?;
    let dist_url = pkg
        .get("dist")
        .and_then(|d| d.get("url"))
        .and_then(|u| u.as_str())
        .map(String::from);
    let dist_shasum = pkg
        .get("dist")
        .and_then(|d| d.get("shasum"))
        .and_then(|u| u.as_str())
        .map(String::from);
    let package_type = pkg.get("type").and_then(|t| t.as_str()).map(String::from);

    Some(PackageInfo {
        name: name.to_string(),
        version: version.to_string(),
        is_dev,
        dist_url,
        dist_shasum,
        package_type,
    })
}

fn validate_platform_from_lock(lock: &Value, args: &InstallArgs) -> Result<()> {
    let mut requirements: Vec<(&str, &str, Vec<String>)> = Vec::new();

    if let Some(platform) = lock.get("platform").and_then(|v| v.as_object()) {
        for (name, constraint) in platform {
            if let Some(c) = constraint.as_str() {
                if args
                    .ignore_platform_req
                    .iter()
                    .any(|r| r == name || r == "*")
                {
                    continue;
                }
                requirements.push((name, c, vec!["lock file".to_string()]));
            }
        }
    }

    if requirements.is_empty() {
        return Ok(());
    }

    let mut validator = PlatformValidator::new();
    validator.detect()?;

    let result = validator.validate(&requirements)?;

    if !result.is_satisfied() {
        warning("Platform requirements not satisfied:");
        for err in &result.errors {
            error(&format!(
                "  {} {} required, {} installed",
                err.name,
                err.constraint,
                err.installed.as_deref().unwrap_or("not found")
            ));
        }
        if !args.ignore_platform_reqs {
            bail!("Platform requirements check failed. Use --ignore-platform-reqs to skip.");
        }
    }

    Ok(())
}

fn show_packages_table(packages: &[PackageInfo]) {
    let mut table = Table::new();
    table.headers(["Package", "Version", "Type"]);

    for pkg in packages {
        let pkg_type = if pkg.is_dev { "dev" } else { "prod" };
        table.row([pkg.name.as_str(), pkg.version.as_str(), pkg_type]);
    }

    table.print();
}

/// Install packages with parallel downloads and CAS cache.
async fn install_packages(
    packages: &[PackageInfo],
    vendor_dir: &PathBuf,
    base_dir: &std::path::Path,
    installer_paths: &InstallerPaths,
    args: &InstallArgs,
    progress: Option<&LiveProgress>,
) -> Result<()> {
    let start = Instant::now();

    // Initialize auth manager and check for existing GitHub token
    let mut auth_manager = AuthManager::with_project_root(Some(base_dir));
    let github_token = auth_manager
        .get_credential("github.com")
        .and_then(|c| match c {
            Credential::GitHubOAuth(token) => Some(token),
            _ => None,
        });

    if github_token.is_some() {
        debug!("Using existing GitHub OAuth token");
    }

    // Build HTTP client with optimized settings
    let client = reqwest::Client::builder()
        .pool_max_idle_per_host(100)
        .pool_idle_timeout(std::time::Duration::from_secs(90))
        .http2_adaptive_window(true)
        .http2_initial_stream_window_size(Some(4 * 1024 * 1024))
        .http2_initial_connection_window_size(Some(8 * 1024 * 1024))
        .http2_keep_alive_interval(Some(std::time::Duration::from_secs(15)))
        .http2_keep_alive_timeout(std::time::Duration::from_secs(20))
        .http2_keep_alive_while_idle(true)
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(60))
        .tcp_nodelay(true)
        .tcp_keepalive(std::time::Duration::from_secs(30))
        .gzip(true)
        .brotli(true)
        .deflate(true)
        .build()
        .context("Failed to create HTTP client")?;

    // Separate cached vs need-download
    let mut to_download: Vec<(String, String, String, PathBuf, Option<String>)> = Vec::new();
    let mut from_cache: Vec<(String, PathBuf, PathBuf)> = Vec::new();
    let mut skipped = 0;

    for pkg in packages {
        // Check if this package has a custom install path
        let dest = installer_paths
            .get_path(base_dir, &pkg.name, pkg.package_type.as_deref())
            .unwrap_or_else(|| {
                vendor_dir.join(pkg.name.replace('/', std::path::MAIN_SEPARATOR_STR))
            });

        if let Some(ref url_str) = pkg.dist_url {
            let url = convert_github_api_url(url_str);

            // Use is_cached for quick check, then get_cached_path for the actual path
            if cas_cache::is_cached(&url) {
                if let Some(cache_path) = cas_cache::get_cached_path(&url) {
                    from_cache.push((pkg.name.clone(), cache_path, dest));
                } else {
                    // Cache marker exists but path retrieval failed, download
                    to_download.push((
                        pkg.name.clone(),
                        pkg.version.clone(),
                        url,
                        dest,
                        pkg.dist_shasum.clone(),
                    ));
                }
            } else {
                to_download.push((
                    pkg.name.clone(),
                    pkg.version.clone(),
                    url,
                    dest,
                    pkg.dist_shasum.clone(),
                ));
            }
        } else {
            skipped += 1;
        }
    }

    let cached_count = from_cache.len();
    let download_count = to_download.len();
    let total = cached_count + download_count;

    if total == 0 {
        if skipped > 0 {
            warning(&format!("No download URLs for {skipped} packages"));
        }
        return Ok(());
    }

    // Set up progress for all packages (downloads + cache links)
    if let Some(p) = progress {
        if download_count > 0 {
            p.set_downloading(total, cached_count);
        } else {
            p.set_linking(total);
        }
    }

    // Link cached packages first (instant)
    for (name, cache_path, dest) in &from_cache {
        if let Some(p) = progress {
            p.set_current(name);
        }
        if let Err(e) = cas_cache::link_from_cache(cache_path, dest) {
            warning(&format!("Cache link failed for {name}: {e}"));
        }
        if let Some(p) = progress {
            p.inc_completed();
        }
    }

    if to_download.is_empty() {
        return Ok(());
    }

    // Adaptive concurrency based on CPU cores
    let cpu_cores = std::thread::available_parallelism()
        .map(std::num::NonZero::get)
        .unwrap_or(4);
    let max_concurrent = if args.dry_run {
        1
    } else {
        (cpu_cores * 8).clamp(32, 128)
    };

    let completed = Arc::new(AtomicU64::new(0));
    let failed_count = Arc::new(AtomicU64::new(0));
    let total_bytes = Arc::new(AtomicU64::new(0));

    let mut pending: Vec<_> = to_download.into_iter().collect();
    let mut in_flight = FuturesUnordered::new();
    let mut errors: Vec<String> = Vec::new();
    let verify_checksums = args.verify_checksums;

    // Track credentials for retry - convert GitHub token to Credential if present
    let mut credential_for_retry: Option<Credential> = github_token.map(Credential::GitHubOAuth);
    let mut rate_limited_packages: Vec<(String, String, String, PathBuf, Option<String>)> =
        Vec::new();

    while !pending.is_empty() || !in_flight.is_empty() {
        while in_flight.len() < max_concurrent && !pending.is_empty() {
            let (name, version, url, dest, shasum) = pending.pop().unwrap();
            let client = client.clone();
            let total_bytes = Arc::clone(&total_bytes);

            // Get credential for this URL's domain
            let credential = credential_for_retry.clone().or_else(|| {
                extract_domain_from_url(&url)
                    .and_then(|domain| auth_manager.get_credential(&domain))
            });

            // Update progress with current package
            if let Some(p) = progress {
                p.set_current(&name);
            }

            in_flight.push(async move {
                let result = download_and_extract_with_credential(
                    &client,
                    &name,
                    &version,
                    &url,
                    &dest,
                    &total_bytes,
                    shasum.as_deref(),
                    verify_checksums,
                    credential.as_ref(),
                )
                .await;
                (name, version, url, dest, shasum, result)
            });
        }

        if let Some((name, version, url, dest, shasum, result)) = in_flight.next().await {
            match result {
                Ok(dest_path) => {
                    completed.fetch_add(1, Ordering::Relaxed);
                    if let Some(p) = progress {
                        p.inc_completed();
                        p.add_bytes(total_bytes.load(Ordering::Relaxed));
                    }
                    let _ = cas_cache::store_in_cache(&url, &dest_path);
                }
                Err(e) => {
                    let error_str = e.to_string();
                    // Check if this is an authentication/rate limit error that can be retried
                    let is_github_rate_limit = error_str.contains("rate limit")
                        && (url.contains("github.com") || url.contains("codeload.github.com"));
                    let is_auth_failure = error_str.contains("HTTP 401")
                        || error_str.contains("HTTP 403")
                        || error_str.contains("Unauthorized")
                        || error_str.contains("Forbidden");

                    if is_github_rate_limit || is_auth_failure {
                        // Queue for retry after getting credentials
                        rate_limited_packages.push((name, version, url, dest, shasum));
                    } else {
                        failed_count.fetch_add(1, Ordering::Relaxed);
                        errors.push(format!("{name}: {e}"));
                    }
                }
            }
        }
    }

    // Handle packages that failed due to authentication/rate limit issues
    if !rate_limited_packages.is_empty() && credential_for_retry.is_none() {
        // Detect auth type based on first URL's domain
        let first_url = &rate_limited_packages[0].2;
        let (domain, auth_type) = detect_auth_type_for_url(first_url);

        let reason = if first_url.contains("github.com") {
            let rate_limit_info = GitHubRateLimitInfo {
                url: first_url.clone(),
                limit: 60,
                reset: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() + 3600)
                    .unwrap_or(0),
            };
            format!(
                "GitHub API limit (60 calls/hr) is exhausted, could not fetch {}.\n\
                 You can also wait until {} for the rate limit to reset.",
                rate_limit_info.url,
                rate_limit_info.reset_time_string()
            )
        } else {
            format!(
                "Authentication required for {}.\n\
                 Please provide credentials to access this repository.",
                domain
            )
        };

        match auth_manager.prompt_for_auth_type(&domain, auth_type, &reason) {
            Ok(Some(cred)) => {
                let _ = credential_for_retry.insert(cred.clone());
                info(&format!(
                    "Retrying {} packages with credentials...",
                    rate_limited_packages.len()
                ));

                // Retry downloads with the new credential
                for (name, version, url, dest, shasum) in rate_limited_packages {
                    if let Some(p) = progress {
                        p.set_current(&name);
                    }

                    match download_and_extract_with_credential(
                        &client,
                        &name,
                        &version,
                        &url,
                        &dest,
                        &total_bytes,
                        shasum.as_deref(),
                        verify_checksums,
                        Some(&cred),
                    )
                    .await
                    {
                        Ok(dest_path) => {
                            completed.fetch_add(1, Ordering::Relaxed);
                            if let Some(p) = progress {
                                p.inc_completed();
                                p.add_bytes(total_bytes.load(Ordering::Relaxed));
                            }
                            let _ = cas_cache::store_in_cache(&url, &dest_path);
                        }
                        Err(e) => {
                            failed_count.fetch_add(1, Ordering::Relaxed);
                            errors.push(format!("{name}: {e}"));
                        }
                    }
                }
            }
            Ok(None) => {
                // User declined to provide credentials, mark all as failed
                for (name, _, url, _, _) in rate_limited_packages {
                    failed_count.fetch_add(1, Ordering::Relaxed);
                    errors.push(format!("{name}: Authentication required for {url}"));
                }
            }
            Err(e) => {
                warning(&format!("Failed to prompt for credentials: {e}"));
                for (name, _, url, _, _) in rate_limited_packages {
                    failed_count.fetch_add(1, Ordering::Relaxed);
                    errors.push(format!("{name}: Authentication required for {url}"));
                }
            }
        }
    } else if !rate_limited_packages.is_empty() {
        // We had credentials but still failed - credentials may be invalid
        warning(
            "Authentication failed despite having credentials. They may be invalid or expired.",
        );
        for (name, _, url, _, _) in rate_limited_packages {
            failed_count.fetch_add(1, Ordering::Relaxed);
            errors.push(format!("{name}: Authentication failed for {url}"));
        }
    }

    let elapsed = start.elapsed();
    let installed = completed.load(Ordering::Relaxed) + cached_count as u64;
    let failed = failed_count.load(Ordering::Relaxed);
    let bytes = total_bytes.load(Ordering::Relaxed);

    for err in &errors {
        warning(&format!("Failed: {err}"));
    }

    if failed > 0 {
        bail!("Failed to install {failed} of {total} packages. See warnings above.");
    }

    // Print installation summary
    if installed > 0 {
        let speed = if elapsed.as_secs() > 0 {
            format!(" ({}/s)", format_bytes(bytes / elapsed.as_secs().max(1)))
        } else {
            String::new()
        };

        tracing::info!(
            installed = installed,
            cached = cached_count,
            downloaded = download_count,
            bytes = bytes,
            elapsed_ms = elapsed.as_millis(),
            "installation complete"
        );

        if !args.no_progress {
            println!(
                "  Installed {} packages in {:.2}s ({} downloaded{}, {} from cache)",
                installed,
                elapsed.as_secs_f64(),
                format_bytes(bytes),
                speed,
                cached_count
            );
        }
    }

    Ok(())
}

/// Download and extract a package archive with credential-based authentication.
///
/// Supports all Composer authentication types:
/// - GitHub OAuth tokens
/// - GitLab OAuth and private tokens
/// - Bitbucket OAuth
/// - HTTP Basic authentication
/// - Bearer tokens
/// - Forgejo/Gitea tokens
/// - Custom HTTP headers
async fn download_and_extract_with_credential(
    client: &reqwest::Client,
    name: &str,
    _version: &str,
    url: &str,
    dest: &std::path::Path,
    total_bytes: &AtomicU64,
    expected_shasum: Option<&str>,
    verify_checksums: bool,
    credential: Option<&Credential>,
) -> Result<PathBuf> {
    let mut request = client.get(url);

    // Apply credential-based authentication
    if let Some(cred) = credential {
        request = apply_credential_to_request(request, cred, url);
    }

    let response = request
        .send()
        .await
        .with_context(|| format!("Failed to fetch {name}"))?;

    let status = response.status();

    // Check for GitHub rate limit
    if is_github_rate_limit_error(url, status.as_u16()) {
        let rate_limit =
            parse_rate_limit_headers(url, response.headers()).unwrap_or(GitHubRateLimitInfo {
                url: url.to_string(),
                limit: 60,
                reset: 0,
            });

        anyhow::bail!(
            "GitHub rate limit exceeded for {}: {} calls/hr limit, resets at {}",
            name,
            rate_limit.limit,
            rate_limit.reset_time_string()
        );
    }

    if !status.is_success() {
        anyhow::bail!("HTTP {}", status);
    }

    let bytes = response
        .bytes()
        .await
        .with_context(|| format!("Failed to read response for {name}"))?;

    total_bytes.fetch_add(bytes.len() as u64, Ordering::Relaxed);

    // Verify checksum if provided and verification is enabled
    if verify_checksums
        && let Some(expected) = expected_shasum
        && !expected.is_empty()
    {
        let actual = compute_sha1(&bytes);
        if !constant_time_eq(&actual, expected) {
            anyhow::bail!("Checksum mismatch: expected {expected}, got {actual}");
        }
    }

    // Extract in blocking task to not block async runtime
    let dest = dest.to_path_buf();
    let name = name.to_string();
    tokio::task::spawn_blocking(move || {
        use std::io::Write;

        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let temp_path = dest.with_extension("download.zip");
        {
            let mut file = std::fs::File::create(&temp_path)?;
            file.write_all(&bytes)?;
        }

        extract_zip(&temp_path, &dest).with_context(|| format!("Failed to extract {name}"))?;
        let _ = std::fs::remove_file(&temp_path);

        Ok(dest)
    })
    .await
    .context("Extraction task failed")?
}

/// Compute SHA-1 hash of bytes and return as hex string.
fn compute_sha1(data: &[u8]) -> String {
    use sha1::{Digest, Sha1};
    let mut hasher = Sha1::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

/// Constant-time string comparison to prevent timing attacks.
fn constant_time_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut result = 0u8;
    for (x, y) in a.bytes().zip(b.bytes()) {
        result |= x ^ y;
    }
    result == 0
}

fn extract_zip(zip_path: &std::path::Path, dest: &std::path::Path) -> Result<()> {
    use std::io::Read;

    let file = std::fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file)?;

    // Find common prefix (GitHub zips have vendor-repo-hash/ prefix)
    let mut common_prefix: Option<String> = None;
    for i in 0..archive.len() {
        let entry = archive.by_index(i)?;
        let path = entry.name();
        if let Some(first_component) = path.split('/').next()
            && !first_component.is_empty()
        {
            match &common_prefix {
                None => common_prefix = Some(format!("{first_component}/")),
                Some(p) if !path.starts_with(p) => {
                    common_prefix = None;
                    break;
                }
                _ => {}
            }
        }
    }

    let prefix_len = common_prefix.as_ref().map_or(0, std::string::String::len);

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let entry_path = entry.name();

        if entry_path.len() <= prefix_len {
            continue;
        }

        let relative_path = &entry_path[prefix_len..];
        if relative_path.is_empty() {
            continue;
        }

        let out_path = dest.join(relative_path);

        if entry.is_dir() {
            std::fs::create_dir_all(&out_path)?;
        } else {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut outfile = std::fs::File::create(&out_path)?;
            let mut buffer = Vec::new();
            entry.read_to_end(&mut buffer)?;
            std::io::Write::write_all(&mut outfile, &buffer)?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Some(mode) = entry.unix_mode() {
                    let _ =
                        std::fs::set_permissions(&out_path, std::fs::Permissions::from_mode(mode));
                }
            }
        }
    }

    Ok(())
}

/// Apply authentication credentials to an HTTP request.
///
/// Handles all Composer-supported authentication types:
/// - GitHub OAuth: Bearer token for github.com URLs
/// - GitLab OAuth/Token: Bearer token for gitlab URLs
/// - Bitbucket OAuth: Basic auth with consumer key/secret
/// - HTTP Basic: Basic auth header
/// - Bearer: Generic bearer token
/// - Forgejo: Token-based auth
/// - Custom Headers: Raw header injection
fn apply_credential_to_request(
    request: reqwest::RequestBuilder,
    credential: &Credential,
    url: &str,
) -> reqwest::RequestBuilder {
    match credential {
        Credential::GitHubOAuth(token) => {
            // Only apply to GitHub URLs
            if url.contains("github.com") || url.contains("api.github.com") {
                request.header("Authorization", format!("Bearer {token}"))
            } else {
                request
            }
        }
        Credential::GitLabOAuth(token) | Credential::GitLabToken(token) => {
            // Apply to GitLab URLs
            if url.contains("gitlab") {
                request.header("Authorization", format!("Bearer {token}"))
            } else {
                request
            }
        }
        Credential::BitbucketOAuth {
            consumer_key,
            consumer_secret,
        } => {
            // Bitbucket uses Basic auth with OAuth credentials
            request.basic_auth(consumer_key, Some(consumer_secret))
        }
        Credential::HttpBasic { username, password } => {
            request.basic_auth(username, Some(password))
        }
        Credential::Bearer(token) => request.header("Authorization", format!("Bearer {token}")),
        Credential::ForgejoToken { token, .. } => {
            request.header("Authorization", format!("token {token}"))
        }
        Credential::CustomHeaders(headers) => {
            let mut req = request;
            for header in headers {
                // Parse "Header-Name: value" format
                if let Some((name, value)) = header.split_once(':') {
                    req = req.header(name.trim(), value.trim());
                }
            }
            req
        }
    }
}

/// Extract domain from URL for credential lookup.
fn extract_domain_from_url(url: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(String::from))
}

/// Detect the appropriate authentication type based on URL domain.
fn detect_auth_type_for_url(url: &str) -> (String, crate::auth_manager::PromptableAuthType) {
    use crate::auth_manager::PromptableAuthType;

    let domain = extract_domain_from_url(url).unwrap_or_else(|| "unknown".to_string());
    let domain_lower = domain.to_lowercase();

    let auth_type = if domain_lower.contains("github.com") {
        PromptableAuthType::GitHubOAuth
    } else if domain_lower == "gitlab.com" {
        // GitLab.com prefers OAuth tokens
        PromptableAuthType::GitLabOAuth
    } else if domain_lower.contains("gitlab") {
        // Self-hosted GitLab instances typically use private tokens
        PromptableAuthType::GitLabToken
    } else if domain_lower.contains("bitbucket") {
        PromptableAuthType::BitbucketOAuth
    } else if domain_lower.contains("codeberg.org")
        || domain_lower.contains("forgejo")
        || domain_lower.contains("gitea")
    {
        PromptableAuthType::ForgejoToken
    } else {
        // Default to HTTP Basic for unknown domains (private Packagist, Satis, etc.)
        PromptableAuthType::HttpBasic
    };

    (domain, auth_type)
}

fn generate_lock_file(
    lock_path: &PathBuf,
    resolution: &libretto_resolver::Resolution,
    composer: &Value,
) -> Result<()> {
    super::lock_generator::generate_lock_file(lock_path, resolution, composer)
}

fn generate_autoloader(vendor_dir: &PathBuf, args: &InstallArgs) -> Result<()> {
    use libretto_autoloader::{AutoloadConfig, AutoloaderGenerator, OptimizationLevel};
    use serde::Deserialize;
    use std::collections::HashMap;

    #[derive(Debug, Deserialize)]
    struct ComposerJson {
        #[serde(default)]
        autoload: AutoloadSection,
        #[serde(default, rename = "autoload-dev")]
        autoload_dev: AutoloadSection,
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

    fn load_autoload_config(path: &std::path::Path) -> Option<AutoloadConfig> {
        let content = std::fs::read_to_string(path).ok()?;
        let composer: ComposerJson = sonic_rs::from_str(&content).ok()?;

        let mut config = AutoloadConfig::default();

        for (namespace, paths) in composer.autoload.psr4 {
            config.psr4.mappings.insert(namespace, paths.to_vec());
        }
        for (namespace, paths) in composer.autoload.psr0 {
            config.psr0.mappings.insert(namespace, paths.to_vec());
        }
        config.classmap.paths = composer.autoload.classmap;
        config.files.files = composer.autoload.files;
        config.exclude.patterns = composer.autoload.exclude;

        Some(config)
    }

    fn load_autoload_config_with_dev(path: &std::path::Path) -> Option<AutoloadConfig> {
        let content = std::fs::read_to_string(path).ok()?;
        let composer: ComposerJson = sonic_rs::from_str(&content).ok()?;

        let mut config = AutoloadConfig::default();

        for (namespace, paths) in composer.autoload.psr4 {
            config.psr4.mappings.insert(namespace, paths.to_vec());
        }
        for (namespace, paths) in composer.autoload.psr0 {
            config.psr0.mappings.insert(namespace, paths.to_vec());
        }
        config.classmap.paths = composer.autoload.classmap;
        config.files.files = composer.autoload.files;
        config.exclude.patterns = composer.autoload.exclude;

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

    let level = if args.classmap_authoritative {
        OptimizationLevel::Authoritative
    } else if args.optimize_autoloader {
        OptimizationLevel::Optimized
    } else {
        OptimizationLevel::None
    };

    let mut generator = AutoloaderGenerator::with_optimization(vendor_dir.clone(), level);

    // Scan vendor directory for installed packages
    if let Ok(entries) = std::fs::read_dir(vendor_dir) {
        for entry in entries.filter_map(Result::ok) {
            let vendor_path = entry.path();
            if vendor_path.is_dir() {
                if vendor_path.file_name().is_some_and(|n| n == "composer") {
                    continue;
                }

                if let Ok(package_entries) = std::fs::read_dir(&vendor_path) {
                    for package_entry in package_entries.filter_map(Result::ok) {
                        let package_path = package_entry.path();
                        if package_path.is_dir() {
                            let composer_json_path = package_path.join("composer.json");
                            if composer_json_path.exists()
                                && let Some(config) = load_autoload_config(&composer_json_path)
                            {
                                generator.add_package(&package_path, &config);
                            }
                        }
                    }
                }
            }
        }
    }

    // Load root project's autoload config (including dev if not --no-dev)
    let root_composer_json = std::path::PathBuf::from("composer.json");
    if root_composer_json.exists() {
        let config = if args.no_dev {
            load_autoload_config(&root_composer_json)
        } else {
            load_autoload_config_with_dev(&root_composer_json)
        };
        if let Some(config) = config {
            let project_root = std::path::PathBuf::from(".");
            generator.add_package(&project_root, &config);
        }
    }

    generator.generate()?;

    Ok(())
}

/// Convert GitHub API URLs to codeload URLs to avoid rate limits.
fn convert_github_api_url(url: &str) -> String {
    if !url.starts_with("https://api.github.com/repos/") {
        return url.to_string();
    }

    let path = url.trim_start_matches("https://api.github.com/repos/");
    let parts: Vec<&str> = path.split('/').collect();

    if parts.len() >= 4 {
        let owner = parts[0];
        let repo = parts[1];
        let archive_type = parts[2];
        let reference = parts[3..].join("/");

        let ext = if archive_type == "tarball" {
            "legacy.tar.gz"
        } else {
            "legacy.zip"
        };

        format!("https://codeload.github.com/{owner}/{repo}/{ext}/{reference}")
    } else {
        url.to_string()
    }
}
