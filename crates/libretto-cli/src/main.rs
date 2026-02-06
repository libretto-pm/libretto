//! Libretto CLI - A high-performance Composer-compatible package manager.
//!
//! Libretto is a drop-in replacement for Composer written in Rust,
//! offering significantly improved performance through parallel operations,
//! SIMD optimizations, and efficient caching.

#![warn(clippy::all)]
#![allow(clippy::module_name_repetitions)]

mod auth_manager;
mod cas_cache;
mod commands;
mod context;
mod fetcher;
mod installer_paths;
mod output;
mod platform;
mod scripts;

use clap::Parser;
use commands::{Cli, Commands};
use context::Context;
use std::process::ExitCode;
use std::time::Instant;
use tracing::Level;
use tracing_subscriber::EnvFilter;

fn main() -> ExitCode {
    let start = Instant::now();
    let cli = Cli::parse();

    // Initialize tracing based on verbosity
    let log_level = match cli.verbose {
        0 if cli.quiet => Level::ERROR,
        0 => Level::WARN,
        1 => Level::INFO,
        2 => Level::DEBUG,
        _ => Level::TRACE,
    };

    let filter = EnvFilter::builder()
        .with_default_directive(log_level.into())
        .from_env_lossy();

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .without_time()
        .init();

    // Enable JSON output if requested
    if matches!(cli.format, commands::OutputFormat::Json) {
        output::json::enable();
    }

    // Create context
    let ctx = match Context::new(&cli.to_context_args()) {
        Ok(ctx) => ctx,
        Err(e) => {
            output::json::print_error(&anyhow::anyhow!("Failed to initialize: {e}"));
            return ExitCode::FAILURE;
        }
    };

    // Run the command
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to create runtime");

    let result = runtime.block_on(run_command(&cli, &ctx));

    // Show profiling info if requested (only in text mode)
    if ctx.profile && !output::json::is_enabled() {
        let elapsed = start.elapsed();
        eprintln!(
            "\n[profile] Total time: {}",
            output::format_duration(elapsed)
        );
    }

    match result {
        Ok(code) => code,
        Err(e) => {
            output::json::print_error(&e);
            ExitCode::FAILURE
        }
    }
}

async fn run_command(cli: &Cli, _ctx: &Context) -> anyhow::Result<ExitCode> {
    match &cli.command {
        // Core commands
        Commands::Install(args) => {
            commands::install::run(args.clone()).await?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Update(args) => {
            commands::update::run(args.clone()).await?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Require(args) => {
            commands::require::run(args.clone()).await?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Remove(args) => {
            commands::remove::run(args.clone()).await?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Search(args) => {
            commands::search::run(args.clone()).await?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Show(args) => {
            commands::show::run(args.clone()).await?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Init(args) => {
            commands::init::run(args.clone()).await?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Validate(args) => {
            commands::validate::run(args.clone()).await?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::DumpAutoload(args) => {
            commands::dump_autoload::run(args.clone()).await?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Audit(args) => {
            commands::audit::run(args.clone()).await?;
            Ok(ExitCode::SUCCESS)
        }

        // Additional commands
        Commands::About(args) => {
            commands::about::run(args.clone()).await?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Archive(args) => {
            commands::archive::run(args.clone()).await?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Browse(args) => {
            commands::browse::run(args.clone()).await?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Bump(args) => {
            commands::bump::run(args.clone()).await?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::CacheClear(args) => {
            commands::cache::run_clear(args.clone()).await?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::CacheList(args) => {
            commands::cache::run_list(args.clone()).await?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::CheckPlatformReqs(args) => {
            commands::check_platform_reqs::run(args.clone()).await?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Completion(args) => {
            commands::completion::run(args.clone()).await?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Config(args) => {
            commands::config::run(args.clone()).await?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::CreateProject(args) => {
            commands::create_project::run(args.clone()).await?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Depends(args) => {
            commands::depends::run(args.clone()).await?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Diagnose(args) => {
            commands::diagnose::run(args.clone()).await?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Exec(args) => {
            commands::exec::run(args.clone()).await?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Fund(args) => {
            commands::fund::run(args.clone())?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Global(args) => {
            commands::global::run(args.clone()).await?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Licenses(args) => {
            commands::licenses::run(args.clone()).await?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Outdated(args) => {
            commands::outdated::run(args.clone()).await?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Prohibits(args) => {
            commands::prohibits::run(args.clone()).await?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Reinstall(args) => {
            commands::reinstall::run(args.clone()).await?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Repository(args) => {
            commands::repository::run(args.clone()).await?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::RunScript(args) => {
            commands::run_script::run(args.clone()).await?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::SelfUpdate(args) => {
            commands::self_update::run(args.clone()).await?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Status(args) => {
            commands::status::run(args.clone())?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Suggests(args) => {
            commands::suggests::run(args.clone()).await?;
            Ok(ExitCode::SUCCESS)
        }
    }
}
