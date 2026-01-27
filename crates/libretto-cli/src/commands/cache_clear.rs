//! Cache clear command implementation.

use anyhow::Result;
use clap::Args;
use console::style;
use libretto_cache::Cache;
use tracing::info;

/// Arguments for the cache:clear command.
#[derive(Args, Debug)]
pub struct CacheClearArgs {
    /// Only clear packages cache
    #[arg(long)]
    pub packages: bool,

    /// Only clear repository cache
    #[arg(long)]
    pub repo: bool,

    /// Only clear VCS cache
    #[arg(long)]
    pub vcs: bool,

    /// Prune old entries instead of clearing all
    #[arg(long)]
    pub gc: bool,

    /// Max age in days for gc (default: 30)
    #[arg(long, default_value = "30")]
    pub max_age: i64,
}

/// Run the cache:clear command.
pub async fn run(args: CacheClearArgs) -> Result<()> {
    info!("running cache:clear command");

    println!(
        "{} {}",
        style("Libretto").cyan().bold(),
        style("Managing cache...").dim()
    );

    let cache = match Cache::new() {
        Ok(c) => c,
        Err(e) => {
            println!("{} Failed to access cache: {}", style("Error:").red(), e);
            return Ok(());
        }
    };

    if args.gc {
        println!(
            "{}",
            style(format!(
                "Pruning cache entries older than {} days...",
                args.max_age
            ))
            .dim()
        );

        match cache.prune(args.max_age) {
            Ok(removed) => {
                println!(
                    "{} Removed {} old cache entries",
                    style("Success:").green().bold(),
                    removed
                );
            }
            Err(e) => {
                println!("{} Failed to prune cache: {}", style("Error:").red(), e);
            }
        }
    } else {
        println!("{}", style("Clearing cache...").dim());

        match cache.clear() {
            Ok(()) => {
                println!(
                    "{} Cache cleared successfully",
                    style("Success:").green().bold()
                );
            }
            Err(e) => {
                println!("{} Failed to clear cache: {}", style("Error:").red(), e);
            }
        }
    }

    let stats = cache.legacy_stats();
    println!();
    println!(
        "{} {} entries, {} bytes",
        style("Cache stats:").dim(),
        stats.entries,
        stats.total_size
    );

    Ok(())
}
