//! Show command implementation - Composer-compatible with beautiful formatting.

use anyhow::Result;
use clap::Args;
use owo_colors::OwoColorize;
use sonic_rs::{JsonContainerTrait, JsonValueTrait, Value};

/// Arguments for the show command.
#[derive(Args, Debug, Clone)]
pub struct ShowArgs {
    /// Package name (vendor/name)
    #[arg(value_name = "PACKAGE")]
    pub package: Option<String>,

    /// Package version constraint
    #[arg(value_name = "VERSION")]
    pub version: Option<String>,

    /// Show installed packages
    #[arg(short, long)]
    pub installed: bool,

    /// Show available versions
    #[arg(long, short = 'a')]
    pub available: bool,

    /// Show all info including dev dependencies
    #[arg(long = "all", short = 'A')]
    pub all: bool,

    /// Output as dependency tree
    #[arg(short, long)]
    pub tree: bool,

    /// Only show package names
    #[arg(short = 'N', long = "name-only")]
    pub name_only: bool,

    /// Only show package paths
    #[arg(short = 'P', long)]
    pub path: bool,

    /// Show self (root package)
    #[arg(short = 's', long)]
    pub self_pkg: bool,
}

/// Run the show command.
pub async fn run(args: ShowArgs) -> Result<()> {
    use crate::output::{header, warning};
    use libretto_core::PackageId;

    // If no package specified, show installed packages
    if args.package.is_none() || args.installed {
        return show_installed(&args).await;
    }

    let package_name = args.package.as_ref().unwrap();

    // Check if it's a wildcard search - filter installed packages
    if package_name.contains('*') {
        return show_wildcard_filter(package_name, &args).await;
    }

    header(&format!("Package: {package_name}"));

    let package_id = PackageId::parse(package_name)
        .ok_or_else(|| anyhow::anyhow!("Invalid package name: {package_name}"))?;

    // Check if package is installed locally first (fast path)
    if let Some(pkg) = get_installed_package_details(&package_id.full_name()) {
        // If version is requested, check if it matches
        let installed_ver = pkg.get("version").and_then(|v| v.as_str()).unwrap_or("");

        if let Some(req_ver) = &args.version {
            if req_ver != installed_ver {
                // Version mismatch, fall back to API
                // proceed to API fetch below
            } else {
                let installed = Some(installed_ver.to_string());
                print_package_details(&pkg, &installed, &args, &[pkg.clone()])?;
                return Ok(());
            }
        } else {
            // No version requested, show installed
            let installed = Some(installed_ver.to_string());
            print_package_details(&pkg, &installed, &args, &[pkg.clone()])?;
            return Ok(());
        }
    }

    // Fetch raw package data from Packagist
    let spinner = crate::output::progress::Spinner::new("Fetching package info...");

    let client = reqwest::Client::new();
    let url = format!(
        "https://repo.packagist.org/p2/{}.json",
        package_id.full_name()
    );

    let response = client.get(&url).send().await?;
    if !response.status().is_success() {
        spinner.finish_and_clear();
        anyhow::bail!(
            "HTTP {} from {}: Not found",
            response.status().as_u16(),
            url
        );
    }

    let json: Value = sonic_rs::from_str(&response.text().await?)?;
    spinner.finish_and_clear();

    // Get the latest version from packages
    let packages = json
        .get("packages")
        .and_then(|p| p.get(&package_id.full_name()))
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow::anyhow!("No versions found"))?;

    if packages.is_empty() {
        warning("No versions found for this package");
        return Ok(());
    }

    // Find the installed version if we're in a project
    let installed_version = get_installed_version(&package_id.full_name());

    // Determine which version to show
    let version_to_show = if let Some(req_version) = &args.version {
        // Try to find exact match first
        let exact = packages
            .iter()
            .find(|v| v.get("version").and_then(|s| s.as_str()) == Some(req_version));

        if let Some(v) = exact {
            v
        } else {
            // Try to match normalized version
            packages
                .iter()
                .find(|v| v.get("version_normalized").and_then(|s| s.as_str()) == Some(req_version))
                .ok_or_else(|| anyhow::anyhow!("Version {} not found", req_version))?
        }
    } else {
        // Get latest stable version (or latest if no stable)
        packages
            .iter()
            .find(|v| {
                let ver = v.get("version").and_then(|v| v.as_str()).unwrap_or("");
                !ver.contains("dev")
                    && !ver.contains("alpha")
                    && !ver.contains("beta")
                    && !ver.contains("RC")
            })
            .or_else(|| packages.first())
            .ok_or_else(|| anyhow::anyhow!("No versions found"))?
    };

    print_package_details(version_to_show, &installed_version, &args, packages)?;

    Ok(())
}

fn get_installed_package_details(package_name: &str) -> Option<Value> {
    let lock_path = std::env::current_dir().ok()?.join("composer.lock");
    if !lock_path.exists() {
        return None;
    }

    let content = std::fs::read_to_string(&lock_path).ok()?;
    let lock: Value = sonic_rs::from_str(&content).ok()?;

    for key in ["packages", "packages-dev"] {
        if let Some(pkgs) = lock.get(key).and_then(|v| v.as_array()) {
            for pkg in pkgs {
                if pkg.get("name").and_then(|v| v.as_str()) == Some(package_name) {
                    return Some(pkg.clone());
                }
            }
        }
    }
    None
}

fn get_installed_version(package_name: &str) -> Option<String> {
    let lock_path = std::env::current_dir().ok()?.join("composer.lock");
    if !lock_path.exists() {
        return None;
    }

    let content = std::fs::read_to_string(&lock_path).ok()?;
    let lock: Value = sonic_rs::from_str(&content).ok()?;

    for key in ["packages", "packages-dev"] {
        if let Some(pkgs) = lock.get(key).and_then(|v| v.as_array()) {
            for pkg in pkgs {
                if pkg.get("name").and_then(|v| v.as_str()) == Some(package_name) {
                    return pkg
                        .get("version")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                }
            }
        }
    }
    None
}

fn print_package_details(
    pkg: &Value,
    installed_version: &Option<String>,
    args: &ShowArgs,
    all_versions: &[Value],
) -> Result<()> {
    let colors = crate::output::colors_enabled();

    let name = pkg
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let version = pkg
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let description = pkg
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let pkg_type = pkg
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("library");
    let homepage = pkg.get("homepage").and_then(|v| v.as_str());
    let time = pkg.get("time").and_then(|v| v.as_str());

    println!();

    // ═══════════════════════════════════════════════════════════════
    // Package Header with beautiful box
    // ═══════════════════════════════════════════════════════════════
    print_section_header(name, colors);

    // Basic Info Section
    println!();
    print_kv("name", name, colors);
    print_kv("descrip.", description, colors);

    // Keywords
    if let Some(keywords) = pkg.get("keywords").and_then(|v| v.as_array()) {
        let kw: Vec<_> = keywords.iter().filter_map(|k| k.as_str()).collect();
        if !kw.is_empty() {
            print_kv("keywords", &kw.join(", "), colors);
        }
    }

    // Version with installed marker
    let version_display = if let Some(installed) = installed_version {
        if installed == version {
            format!("* {version}")
        } else {
            format!("{version} (installed: {installed})")
        }
    } else {
        version.to_string()
    };
    print_kv("versions", &version_display, colors);

    // Release time
    if let Some(t) = time {
        if let Some(formatted) = format_release_time(t) {
            print_kv("released", &formatted, colors);
        }
    }

    print_kv("type", pkg_type, colors);

    // License with SPDX info
    if let Some(licenses) = pkg.get("license").and_then(|v| v.as_array()) {
        let license_str: Vec<_> = licenses
            .iter()
            .filter_map(|l| l.as_str())
            .map(|l| format_license(l))
            .collect();
        if !license_str.is_empty() {
            print_kv("license", &license_str.join(", "), colors);
        }
    }

    // Homepage
    if let Some(hp) = homepage {
        print_kv("homepage", hp, colors);
    }

    // Source
    if let Some(source) = pkg.get("source") {
        let src_type = source.get("type").and_then(|v| v.as_str()).unwrap_or("git");
        let src_url = source.get("url").and_then(|v| v.as_str()).unwrap_or("");
        let src_ref = source
            .get("reference")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        print_kv(
            "source",
            &format!("[{src_type}] {src_url} {src_ref}"),
            colors,
        );
    }

    // Dist
    if let Some(dist) = pkg.get("dist") {
        let dist_type = dist.get("type").and_then(|v| v.as_str()).unwrap_or("zip");
        let dist_url = dist.get("url").and_then(|v| v.as_str()).unwrap_or("");
        let dist_ref = dist.get("reference").and_then(|v| v.as_str()).unwrap_or("");
        print_kv(
            "dist",
            &format!("[{dist_type}] {dist_url} {dist_ref}"),
            colors,
        );
    }

    // Names (aliases/provides/replaces)
    let mut names = vec![name.to_string()];
    if let Some(provides) = pkg.get("provide").and_then(|v| v.as_object()) {
        names.extend(provides.iter().map(|(k, _)| k.to_string()));
    }
    if let Some(replaces) = pkg.get("replace").and_then(|v| v.as_object()) {
        names.extend(replaces.iter().map(|(k, _)| k.to_string()));
    }
    if names.len() > 1 {
        print_kv("names", &names.join(", "), colors);
    }

    // Path (if installed)
    if installed_version.is_some() {
        let vendor_path = std::env::current_dir()
            .ok()
            .map(|p| p.join("vendor").join(name));
        if let Some(path) = vendor_path {
            if path.exists() {
                print_kv("path", &path.display().to_string(), colors);
            }
        }
    }

    // Support section
    println!();
    print_section_title("support", colors);
    if let Some(support) = pkg.get("support") {
        if let Some(issues) = support.get("issues").and_then(|v| v.as_str()) {
            print_kv("issues", issues, colors);
        }
        if let Some(source) = support.get("source").and_then(|v| v.as_str()) {
            print_kv("source", source, colors);
        }
        if let Some(docs) = support.get("docs").and_then(|v| v.as_str()) {
            print_kv("docs", docs, colors);
        }
        if let Some(wiki) = support.get("wiki").and_then(|v| v.as_str()) {
            print_kv("wiki", wiki, colors);
        }
    }

    // Autoload section
    if let Some(autoload) = pkg.get("autoload") {
        println!();
        print_section_title("autoload", colors);
        print_autoload(autoload, colors);
    }

    // Authors
    if let Some(authors) = pkg.get("authors").and_then(|v| v.as_array()) {
        if !authors.is_empty() {
            println!();
            print_section_title("authors", colors);
            for author in authors {
                let author_name = author
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown");
                let email = author.get("email").and_then(|v| v.as_str());
                if let Some(e) = email {
                    if colors {
                        println!("  {} {}", author_name.cyan(), format!("<{e}>").dimmed());
                    } else {
                        println!("  {author_name} <{e}>");
                    }
                } else {
                    println!("  {author_name}");
                }
            }
        }
    }

    // Requires
    print_dependency_section(pkg, "require", "requires", colors);

    // Requires (dev) - only with --all
    if args.all {
        print_dependency_section(pkg, "require-dev", "requires (dev)", colors);
    }

    // Suggests
    if let Some(suggests) = pkg.get("suggest").and_then(|v| v.as_object()) {
        if !suggests.is_empty() {
            println!();
            print_section_title("suggests", colors);
            for (name, reason) in suggests.iter() {
                let reason_str = reason.as_str().unwrap_or("");
                if colors {
                    println!("  {} {}", name.cyan(), reason_str.dimmed());
                } else {
                    println!("  {name} {reason_str}");
                }
            }
        }
    }

    // Provides
    if let Some(provides) = pkg.get("provide").and_then(|v| v.as_object()) {
        if !provides.is_empty() {
            println!();
            print_section_title("provides", colors);
            for (name, version) in provides.iter() {
                let ver = version.as_str().unwrap_or("*");
                if colors {
                    println!("  {} {}", name.green(), ver.yellow());
                } else {
                    println!("  {name} {ver}");
                }
            }
        }
    }

    // Conflicts
    if let Some(conflicts) = pkg.get("conflict").and_then(|v| v.as_object()) {
        if !conflicts.is_empty() {
            println!();
            print_section_title("conflicts", colors);
            for (name, version) in conflicts.iter() {
                let ver = version.as_str().unwrap_or("*");
                if colors {
                    println!("  {} {}", name.red(), ver.yellow());
                } else {
                    println!("  {name} {ver}");
                }
            }
        }
    }

    // Replaces
    if let Some(replaces) = pkg.get("replace").and_then(|v| v.as_object()) {
        if !replaces.is_empty() {
            println!();
            print_section_title("replaces", colors);
            for (name, version) in replaces.iter() {
                let ver = version.as_str().unwrap_or("self.version");
                if colors {
                    println!("  {} {}", name.green(), ver.yellow());
                } else {
                    println!("  {name} {ver}");
                }
            }
        }
    }

    // Available versions (with --available or --all)
    if args.available || args.all {
        println!();
        print_section_title(&format!("versions ({} total)", all_versions.len()), colors);
        let display = if args.all {
            all_versions.len()
        } else {
            15.min(all_versions.len())
        };

        for v in all_versions.iter().take(display) {
            let ver = v.get("version").and_then(|v| v.as_str()).unwrap_or("?");
            let stability = get_stability_tag(ver);
            if colors {
                print!("  {}", ver.yellow());
                if !stability.is_empty() {
                    print!(" {}", stability.dimmed());
                }
                println!();
            } else {
                println!("  {ver} {stability}");
            }
        }

        if all_versions.len() > display {
            println!("  ... and {} more", all_versions.len() - display);
        }
    }

    println!();
    Ok(())
}

fn print_section_header(name: &str, colors: bool) {
    let width = 60.min(console::Term::stdout().size().1 as usize);
    let line = "─".repeat(width);
    if colors {
        println!("{}", line.dimmed());
        println!("{}", name.green().bold());
        println!("{}", line.dimmed());
    } else {
        println!("{line}");
        println!("{name}");
        println!("{line}");
    }
}

fn print_section_title(title: &str, colors: bool) {
    if colors {
        println!("{}", title.yellow().bold());
    } else {
        println!("{title}");
    }
}

fn print_kv(key: &str, value: &str, colors: bool) {
    if value.is_empty() {
        return;
    }
    let padded_key = format!("{key:.<9}");
    if colors {
        println!("{} : {}", padded_key.cyan(), value);
    } else {
        println!("{padded_key} : {value}");
    }
}

fn print_dependency_section(pkg: &Value, json_key: &str, title: &str, colors: bool) {
    if let Some(deps) = pkg.get(json_key).and_then(|v| v.as_object()) {
        if !deps.is_empty() {
            println!();
            print_section_title(title, colors);

            // Find max name length for alignment
            let max_len = deps.iter().map(|(name, _)| name.len()).max().unwrap_or(20);

            for (name, constraint) in deps.iter() {
                let c = constraint.as_str().unwrap_or("*");
                if colors {
                    println!("  {:width$} {}", name.green(), c.yellow(), width = max_len);
                } else {
                    println!("  {name:width$} {c}", width = max_len);
                }
            }
        }
    }
}

fn print_autoload(autoload: &Value, colors: bool) {
    // PSR-4
    if let Some(psr4) = autoload.get("psr-4").and_then(|v| v.as_object()) {
        if colors {
            println!("  {}", "psr-4".cyan());
        } else {
            println!("  psr-4");
        }
        for (ns, path) in psr4.iter() {
            let path_str = if let Some(arr) = path.as_array() {
                arr.iter()
                    .filter_map(|p| p.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            } else {
                path.as_str().unwrap_or("").to_string()
            };
            if colors {
                println!("    {} => {}", ns.green(), path_str);
            } else {
                println!("    {ns} => {path_str}");
            }
        }
    }

    // PSR-0
    if let Some(psr0) = autoload.get("psr-0").and_then(|v| v.as_object()) {
        if colors {
            println!("  {}", "psr-0".cyan());
        } else {
            println!("  psr-0");
        }
        for (ns, path) in psr0.iter() {
            let path_str = path.as_str().unwrap_or("");
            if colors {
                println!("    {} => {}", ns.green(), path_str);
            } else {
                println!("    {ns} => {path_str}");
            }
        }
    }

    // Classmap
    if let Some(classmap) = autoload.get("classmap").and_then(|v| v.as_array()) {
        if !classmap.is_empty() {
            if colors {
                println!("  {}", "classmap".cyan());
            } else {
                println!("  classmap");
            }
            for path in classmap.iter().filter_map(|p| p.as_str()) {
                println!("    {path}");
            }
        }
    }

    // Files
    if let Some(files) = autoload.get("files").and_then(|v| v.as_array()) {
        if !files.is_empty() {
            if colors {
                println!("  {}", "files".cyan());
            } else {
                println!("  files");
            }
            for file in files.iter().filter_map(|f| f.as_str()) {
                println!("    {file}");
            }
        }
    }
}

fn format_license(license: &str) -> String {
    // Map common licenses to full names
    let full_name = match license {
        "MIT" => "MIT License (MIT)",
        "Apache-2.0" => "Apache License 2.0",
        "GPL-2.0" | "GPL-2.0-only" => "GNU General Public License v2.0",
        "GPL-3.0" | "GPL-3.0-only" => "GNU General Public License v3.0",
        "BSD-2-Clause" => "BSD 2-Clause License",
        "BSD-3-Clause" => "BSD 3-Clause License",
        "LGPL-2.1" => "GNU Lesser General Public License v2.1",
        "LGPL-3.0" => "GNU Lesser General Public License v3.0",
        "ISC" => "ISC License",
        "Unlicense" => "The Unlicense",
        "proprietary" => "Proprietary",
        _ => license,
    };
    full_name.to_string()
}

fn format_release_time(time: &str) -> Option<String> {
    use chrono::{DateTime, Utc};

    let dt: DateTime<Utc> = time.parse().ok()?;
    let now = Utc::now();
    let diff = now.signed_duration_since(dt);

    let relative = if diff.num_days() == 0 {
        "today".to_string()
    } else if diff.num_days() == 1 {
        "yesterday".to_string()
    } else if diff.num_days() < 7 {
        format!("{} days ago", diff.num_days())
    } else if diff.num_days() < 14 {
        "last week".to_string()
    } else if diff.num_days() < 30 {
        format!("{} weeks ago", diff.num_days() / 7)
    } else if diff.num_days() < 60 {
        "last month".to_string()
    } else if diff.num_days() < 365 {
        format!("{} months ago", diff.num_days() / 30)
    } else if diff.num_days() < 730 {
        "last year".to_string()
    } else {
        format!("{} years ago", diff.num_days() / 365)
    };

    Some(format!("{}, {}", dt.format("%Y-%m-%d"), relative))
}

fn get_stability_tag(version: &str) -> &'static str {
    let v = version.to_lowercase();
    if v.contains("dev") {
        "(dev)"
    } else if v.contains("alpha") {
        "(alpha)"
    } else if v.contains("beta") {
        "(beta)"
    } else if v.contains("rc") {
        "(RC)"
    } else {
        ""
    }
}

/// Unicode-safe string truncation
fn truncate_str(s: &str, max_len: usize) -> String {
    if max_len == 0 {
        return String::new();
    }
    if s.chars().count() <= max_len {
        return s.to_string();
    }
    if max_len <= 3 {
        return s.chars().take(max_len).collect();
    }
    let truncated: String = s.chars().take(max_len - 3).collect();
    format!("{truncated}...")
}

/// Filter and display installed packages matching a wildcard pattern
async fn show_wildcard_filter(pattern: &str, args: &ShowArgs) -> Result<()> {
    use crate::output::{header, info, warning};

    header(&format!("Packages matching: {pattern}"));

    let lock_path = std::env::current_dir()?.join("composer.lock");

    if !lock_path.exists() {
        warning("No composer.lock found. Run 'libretto install' first.");
        return Ok(());
    }

    let lock_content = std::fs::read_to_string(&lock_path)?;
    let lock: Value = sonic_rs::from_str(&lock_content)?;

    // Convert glob pattern to regex
    let regex_pattern = pattern
        .replace('.', "\\.")
        .replace('*', ".*")
        .replace('?', ".");
    let regex = regex::Regex::new(&format!("^{regex_pattern}$"))?;

    // Collect matching packages
    let mut matches: Vec<(String, String, String)> = Vec::new();

    for key in ["packages", "packages-dev"] {
        if let Some(pkgs) = lock.get(key).and_then(|v| v.as_array()) {
            for pkg in pkgs {
                let name = pkg.get("name").and_then(|v| v.as_str()).unwrap_or("");
                if regex.is_match(name) {
                    let version = pkg.get("version").and_then(|v| v.as_str()).unwrap_or("");
                    let description = pkg
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    matches.push((
                        name.to_string(),
                        version.to_string(),
                        description.to_string(),
                    ));
                }
            }
        }
    }

    if matches.is_empty() {
        warning(&format!("No packages found matching '{pattern}'"));
        return Ok(());
    }

    matches.sort_by(|a, b| a.0.cmp(&b.0));

    if args.name_only {
        for (name, _, _) in &matches {
            println!("{name}");
        }
        return Ok(());
    }

    // Calculate column widths
    let name_width = matches.iter().map(|(n, _, _)| n.len()).max().unwrap_or(30);
    let ver_width = matches.iter().map(|(_, v, _)| v.len()).max().unwrap_or(10);
    let term_width = console::Term::stdout().size().1 as usize;
    let desc_width = term_width.saturating_sub(name_width + ver_width + 4);

    let colors = crate::output::colors_enabled();

    for (name, version, description) in &matches {
        let desc_truncated = truncate_str(description, desc_width);

        if colors {
            println!(
                "{:name_width$} {:ver_width$} {}",
                name.green(),
                version.yellow(),
                desc_truncated.dimmed()
            );
        } else {
            println!("{name:name_width$} {version:ver_width$} {desc_truncated}");
        }
    }

    println!();
    info(&format!("Found {} package(s)", matches.len()));

    Ok(())
}

async fn show_installed(args: &ShowArgs) -> Result<()> {
    use crate::output::table::Table;
    use crate::output::{header, info, warning};

    header("Installed packages");

    let lock_path = std::env::current_dir()?.join("composer.lock");

    if !lock_path.exists() {
        warning("No composer.lock found. Run 'libretto install' first.");
        return Ok(());
    }

    let lock_content = std::fs::read_to_string(&lock_path)?;
    let lock: Value = sonic_rs::from_str(&lock_content)?;

    if args.tree {
        return show_tree(&lock).await;
    }

    // Collect packages
    let mut packages: Vec<(String, String, bool, Option<String>)> = Vec::new();

    if let Some(pkgs) = lock.get("packages").and_then(|v| v.as_array()) {
        for pkg in pkgs {
            let name = pkg.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let version = pkg.get("version").and_then(|v| v.as_str()).unwrap_or("");
            let description = pkg
                .get("description")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            packages.push((name.to_string(), version.to_string(), false, description));
        }
    }

    if let Some(pkgs) = lock.get("packages-dev").and_then(|v| v.as_array()) {
        for pkg in pkgs {
            let name = pkg.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let version = pkg.get("version").and_then(|v| v.as_str()).unwrap_or("");
            let description = pkg
                .get("description")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            packages.push((name.to_string(), version.to_string(), true, description));
        }
    }

    if packages.is_empty() {
        info("No packages installed");
        return Ok(());
    }

    // Sort by name
    packages.sort_by(|a, b| a.0.cmp(&b.0));

    // Filter by package name if specified
    if let Some(filter) = &args.package {
        packages.retain(|(name, _, _, _)| name.contains(filter));
    }

    if args.name_only {
        for (name, _, _, _) in &packages {
            println!("{name}");
        }
        return Ok(());
    }

    if args.path {
        let vendor = std::env::current_dir()?.join("vendor");
        for (name, _, _, _) in &packages {
            println!("{}", vendor.join(name).display());
        }
        return Ok(());
    }

    // Display with beautiful table
    let mut table = Table::new();
    table.headers(["Package", "Version", "Type"]);

    for (name, version, is_dev, _) in &packages {
        let pkg_type = if *is_dev { "dev" } else { "prod" };

        let type_cell = if *is_dev {
            table.dim_cell(pkg_type)
        } else {
            comfy_table::Cell::new(pkg_type)
        };

        table.styled_row(vec![
            comfy_table::Cell::new(name),
            comfy_table::Cell::new(version),
            type_cell,
        ]);
    }

    table.print();

    println!();
    info(&format!("{} package(s) installed", packages.len()));

    Ok(())
}

async fn show_tree(lock: &Value) -> Result<()> {
    use std::collections::HashMap;

    let colors = crate::output::colors_enabled();
    let unicode = crate::output::unicode_enabled();

    // Build dependency map
    let mut deps: HashMap<String, Vec<(String, String)>> = HashMap::new();
    let mut versions: HashMap<String, String> = HashMap::new();

    for key in ["packages", "packages-dev"] {
        if let Some(packages) = lock.get(key).and_then(|v| v.as_array()) {
            for pkg in packages {
                let name = pkg.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let version = pkg.get("version").and_then(|v| v.as_str()).unwrap_or("");

                versions.insert(name.to_string(), version.to_string());

                if let Some(require) = pkg.get("require").and_then(|v| v.as_object()) {
                    let pkg_deps: Vec<(String, String)> = require
                        .iter()
                        .filter(|(n, _)| !n.starts_with("php") && !n.starts_with("ext-"))
                        .map(|(n, c)| (n.to_string(), c.as_str().unwrap_or("*").to_string()))
                        .collect();
                    deps.insert(name.to_string(), pkg_deps);
                }
            }
        }
    }

    // Get root dependencies
    let composer_path = std::env::current_dir()?.join("composer.json");
    let mut root_deps: Vec<String> = Vec::new();

    if composer_path.exists() {
        let composer_content = std::fs::read_to_string(&composer_path)?;
        let composer: Value = sonic_rs::from_str(&composer_content)?;

        if let Some(require) = composer.get("require").and_then(|v| v.as_object()) {
            for (name, _) in require {
                if !name.starts_with("php") && !name.starts_with("ext-") {
                    root_deps.push(name.to_string());
                }
            }
        }

        if let Some(require_dev) = composer.get("require-dev").and_then(|v| v.as_object()) {
            for (name, _) in require_dev {
                if !name.starts_with("php") && !name.starts_with("ext-") {
                    root_deps.push(name.to_string());
                }
            }
        }
    }

    root_deps.sort();

    // Print tree
    fn print_tree_node(
        name: &str,
        versions: &HashMap<String, String>,
        deps: &HashMap<String, Vec<(String, String)>>,
        depth: usize,
        is_last: bool,
        prefix: &str,
        colors: bool,
        unicode: bool,
        visited: &mut Vec<String>,
    ) {
        let connector = if depth == 0 {
            ""
        } else if unicode {
            if is_last { "└── " } else { "├── " }
        } else if is_last {
            "`-- "
        } else {
            "|-- "
        };

        let version = versions
            .get(name)
            .cloned()
            .unwrap_or_else(|| "?".to_string());

        if colors {
            println!(
                "{}{}{}{}",
                prefix,
                connector,
                name.green(),
                format!(" {version}").yellow()
            );
        } else {
            println!("{prefix}{connector}{name} {version}");
        }

        // Avoid cycles
        if visited.contains(&name.to_string()) {
            return;
        }
        visited.push(name.to_string());

        if let Some(pkg_deps) = deps.get(name) {
            let new_prefix = if depth == 0 {
                String::new()
            } else {
                format!(
                    "{}{}",
                    prefix,
                    if unicode {
                        if is_last { "    " } else { "│   " }
                    } else if is_last {
                        "    "
                    } else {
                        "|   "
                    }
                )
            };

            for (i, (dep_name, _)) in pkg_deps.iter().enumerate() {
                let dep_is_last = i == pkg_deps.len() - 1;
                print_tree_node(
                    dep_name,
                    versions,
                    deps,
                    depth + 1,
                    dep_is_last,
                    &new_prefix,
                    colors,
                    unicode,
                    visited,
                );
            }
        }

        visited.pop();
    }

    for (i, root) in root_deps.iter().enumerate() {
        let is_last = i == root_deps.len() - 1;
        let mut visited = Vec::new();
        print_tree_node(
            root,
            &versions,
            &deps,
            0,
            is_last,
            "",
            colors,
            unicode,
            &mut visited,
        );
    }

    Ok(())
}
