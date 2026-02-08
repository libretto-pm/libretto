//! CLI commands for Libretto.

// Core commands
pub mod audit;
pub mod dump_autoload;
pub mod init;
pub mod install;
pub mod lock_generator;
pub mod remove;
pub mod require;
pub mod search;
pub mod show;
pub mod update;
pub mod validate;

// Additional commands
pub mod about;
pub mod archive;
pub mod browse;
pub mod bump;
pub mod cache;
pub mod check_platform_reqs;
pub mod completion;
pub mod config;
pub mod create_project;
pub mod depends;
pub mod diagnose;
pub mod exec;
pub mod fund;
pub mod global;
pub mod licenses;
pub mod outdated;
pub mod prohibits;
pub mod reinstall;
pub mod repository;
pub mod run_script;
pub mod self_update;
pub mod status;
pub mod suggests;

use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

/// Libretto - A high-performance Composer-compatible package manager
///
/// Libretto is a drop-in replacement for Composer written in Rust,
/// offering significantly improved performance through parallel operations,
/// SIMD optimizations, and efficient caching.
#[derive(Parser, Debug)]
#[command(name = "libretto")]
#[command(author = "Libretto Contributors")]
#[command(version)]
#[command(about = "A high-performance Composer-compatible package manager", long_about = None)]
#[command(propagate_version = true)]
#[command(arg_required_else_help = true)]
#[command(styles = get_styles())]
pub struct Cli {
    /// Do not output any message
    #[arg(short = 'q', long, global = true)]
    pub quiet: bool,

    /// Force ANSI output (colors and formatting)
    #[arg(long, global = true, conflicts_with = "no_ansi")]
    pub ansi: bool,

    /// Disable ANSI output (colors and formatting)
    #[arg(long, global = true)]
    pub no_ansi: bool,

    /// Do not ask any interactive question
    #[arg(short = 'n', long, global = true)]
    pub no_interaction: bool,

    /// Display timing and memory usage information
    #[arg(long, global = true)]
    pub profile: bool,

    /// Disables all plugins
    #[arg(long, global = true)]
    pub no_plugins: bool,

    /// Skips execution of scripts defined in composer.json
    #[arg(long, global = true)]
    pub no_scripts: bool,

    /// Use the specified directory as working directory
    #[arg(short = 'd', long = "working-dir", global = true, value_name = "DIR")]
    pub working_dir: Option<PathBuf>,

    /// Prevent use of the cache
    #[arg(long, global = true)]
    pub no_cache: bool,

    /// Output format (text, json, or table)
    #[arg(long, global = true, value_enum, default_value = "text")]
    pub format: OutputFormat,

    /// Increase the verbosity of messages: -v for verbose, -vv for very verbose, -vvv for debug
    #[arg(short = 'v', long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,

    #[command(subcommand)]
    pub command: Commands,
}

impl Cli {
    /// Convert to context args
    pub fn to_context_args(&self) -> crate::context::ContextArgs {
        crate::context::ContextArgs {
            working_dir: self.working_dir.clone(),
            verbosity: self.verbose,
            quiet: self.quiet,
            ansi: self.ansi,
            no_ansi: self.no_ansi,
            no_plugins: self.no_plugins,
            no_scripts: self.no_scripts,
            no_cache: self.no_cache,
            no_interaction: self.no_interaction,
            profile: self.profile,
        }
    }
}

/// Available commands
#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Short information about Libretto
    About(about::AboutArgs),

    /// Creates an archive of this composer package
    Archive(archive::ArchiveArgs),

    /// Checks for security vulnerability advisories for installed packages
    Audit(audit::AuditArgs),

    /// Opens the package's repository URL or homepage in your browser
    #[command(alias = "home")]
    Browse(browse::BrowseArgs),

    /// Increases the lower limit of your composer.json requirements to the currently installed versions
    Bump(bump::BumpArgs),

    /// Clears composer's internal package cache
    #[command(name = "clear-cache", visible_alias = "clearcache", alias = "cc")]
    CacheClear(cache::CacheClearArgs),

    /// Lists packages in the cache
    #[command(name = "cache:list")]
    CacheList(cache::CacheListArgs),

    /// Check that platform requirements are satisfied
    #[command(name = "check-platform-reqs")]
    CheckPlatformReqs(check_platform_reqs::CheckPlatformReqsArgs),

    /// Generate completion script for the specified shell
    Completion(completion::CompletionArgs),

    /// Sets config options. You can use -g to set global options.
    Config(config::ConfigArgs),

    /// Creates new project from a package into given directory
    #[command(name = "create-project")]
    CreateProject(create_project::CreateProjectArgs),

    /// Shows which packages cause the given package to be installed
    #[command(alias = "why")]
    Depends(depends::DependsArgs),

    /// Diagnoses the system to identify common errors
    Diagnose(diagnose::DiagnoseArgs),

    /// Regenerates the autoloader files
    #[command(name = "dump-autoload", visible_alias = "dumpautoload")]
    DumpAutoload(dump_autoload::DumpAutoloadArgs),

    /// Executes a vendored binary/script
    Exec(exec::ExecArgs),

    /// Discover how to help fund the maintenance of your dependencies
    Fund(fund::FundArgs),

    /// Allows running commands in the global composer dir ($`COMPOSER_HOME`)
    Global(global::GlobalArgs),

    /// Creates a basic composer.json file in current directory
    Init(init::InitArgs),

    /// Installs the project dependencies from the composer.lock file if present, or falls back on the composer.json
    #[command(visible_alias = "i")]
    Install(install::InstallArgs),

    /// Shows information about licenses of dependencies
    Licenses(licenses::LicensesArgs),

    /// Shows a list of locally modified packages
    Outdated(outdated::OutdatedArgs),

    /// Shows which packages prevent the given package from being installed
    #[command(alias = "why-not")]
    Prohibits(prohibits::ProhibitsArgs),

    /// Uninstalls and reinstalls the given package names
    Reinstall(reinstall::ReinstallArgs),

    /// Removes a package from the require or require-dev
    #[command(visible_alias = "rm", alias = "uninstall")]
    Remove(remove::RemoveArgs),

    /// Manages repositories (add, remove, list)
    #[command(alias = "repo")]
    Repository(repository::RepositoryArgs),

    /// Adds required packages to your composer.json and installs them
    #[command(visible_alias = "r")]
    Require(require::RequireArgs),

    /// Runs the scripts defined in composer.json
    #[command(name = "run-script", alias = "run")]
    RunScript(run_script::RunScriptArgs),

    /// Searches for packages
    Search(search::SearchArgs),

    /// Updates Libretto to the latest version
    #[command(name = "self-update", alias = "selfupdate")]
    SelfUpdate(self_update::SelfUpdateArgs),

    /// Shows information about packages
    #[command(alias = "info", disable_version_flag = true)]
    Show(show::ShowArgs),

    /// Shows a list of locally modified packages
    Status(status::StatusArgs),

    /// Shows package suggestions
    Suggests(suggests::SuggestsArgs),

    /// Updates your dependencies to the latest version according to composer.json, and updates the composer.lock file
    #[command(visible_alias = "u", alias = "upgrade")]
    Update(update::UpdateArgs),

    /// Validates a composer.json and composer.lock
    Validate(validate::ValidateArgs),
}

/// Output format for commands that support it
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
pub enum OutputFormat {
    /// Human-readable table format
    #[default]
    Table,
    /// JSON format
    Json,
    /// Plain text format
    Text,
}

/// Sort order for list outputs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
pub enum SortOrder {
    /// Sort alphabetically by name
    #[default]
    Name,
    /// Sort by version
    Version,
    /// Sort by type
    Type,
}

/// Get clap styles for colored help
const fn get_styles() -> clap::builder::Styles {
    clap::builder::Styles::styled()
        .header(clap::builder::styling::AnsiColor::Green.on_default().bold())
        .usage(clap::builder::styling::AnsiColor::Green.on_default().bold())
        .literal(clap::builder::styling::AnsiColor::Cyan.on_default())
        .placeholder(clap::builder::styling::AnsiColor::Yellow.on_default())
}

// Re-export for backwards compatibility

/// Backwards compatibility module
#[allow(unused_imports)]
pub mod cache_clear {
    pub use super::cache::run_clear as run;
}
