//! Search command implementation.

use anyhow::Result;
use clap::Args;

/// Arguments for the search command.
#[derive(Args, Debug, Clone)]
pub struct SearchArgs {
    /// Search query
    #[arg(required = true, value_name = "QUERY")]
    pub query: String,

    /// Only show package names
    #[arg(short = 'N', long)]
    pub only_name: bool,

    /// Filter by package type (library, project, etc.)
    #[arg(short = 't', long = "type")]
    pub package_type: Option<String>,

    /// Output format (text, json)
    #[arg(short = 'f', long = "output", default_value = "text")]
    pub output_format: String,
}

/// Run the search command.
pub async fn run(args: SearchArgs) -> Result<()> {
    use crate::output::progress::Spinner;
    use crate::output::{header, info, warning};
    use libretto_repository::Repository;

    header(&format!("Searching for '{}'", args.query));

    let spinner = Spinner::new("Searching Packagist...");

    let repo = Repository::packagist()?;
    repo.init_packagist().await?;

    match repo.search(&args.query).await {
        Ok(results) => {
            spinner.finish_and_clear();

            if results.is_empty() {
                warning(&format!("No packages found matching '{}'", args.query));
                return Ok(());
            }

            // Filter by type if specified (not supported yet, so just pass through)
            let filtered: Vec<_> = results;

            if filtered.is_empty() {
                warning(&format!("No packages found matching '{}'", args.query));
                return Ok(());
            }

            // Output format
            if args.output_format == "json" {
                return output_json(&filtered);
            }

            if args.only_name {
                return output_names_only(&filtered);
            }

            output_full(&filtered)?;

            println!();
            info(&format!("Found {} package(s)", filtered.len()));
        }
        Err(e) => {
            spinner.finish_and_clear();
            anyhow::bail!("Search failed: {e}");
        }
    }

    Ok(())
}

fn output_full(results: &[libretto_repository::PackageSearchResult]) -> Result<()> {
    use owo_colors::OwoColorize;

    let colors = crate::output::colors_enabled();

    // Limit to 15 results for display (like Composer)
    let display_count = 15.min(results.len());

    // Calculate name column width based on longest name + 1 padding (like Composer)
    let name_length = results
        .iter()
        .take(display_count)
        .map(|r| r.name.len())
        .max()
        .unwrap_or(20)
        + 1;

    // Get terminal width, default to 80 (like Composer)
    let term_width = console::Term::stdout().size().1 as usize;

    for result in results.iter().take(display_count) {
        let url = format!("https://packagist.org/packages/{}", result.name);

        // Build warning prefix for abandoned packages (like Composer)
        let warning = if result.abandoned {
            if colors {
                format!("{} ", "! Abandoned !".red().bold())
            } else {
                "! Abandoned ! ".to_string()
            }
        } else {
            String::new()
        };
        // "! Abandoned ! " = 14 chars (but ANSI codes add more, so use plain length)
        let warning_len = if result.abandoned { 14 } else { 0 };

        // Calculate remaining space for description (like Composer)
        // width - nameLength - warningLength - 2 (for spacing)
        let remaining = term_width.saturating_sub(name_length + warning_len + 2);
        let description = truncate(&result.description, remaining);

        // Print the row - Composer style with hyperlinked name
        // Calculate padding separately so hyperlink only covers the name text
        let padding = " ".repeat(name_length.saturating_sub(result.name.len()));

        if colors {
            // OSC 8 hyperlink: \x1b]8;;URL\x07TEXT\x1b]8;;\x07
            // Close the hyperlink BEFORE padding to avoid underline extending
            println!(
                "\x1b]8;;{}\x07{}\x1b]8;;\x07{}{}{}",
                url,
                result.name.green(),
                padding,
                warning,
                description,
            );
        } else {
            println!("{}{}{}{}", result.name, padding, warning, description,);
        }
    }

    Ok(())
}

fn output_names_only(results: &[libretto_repository::PackageSearchResult]) -> Result<()> {
    for result in results {
        println!("{}", result.name);
    }
    Ok(())
}

fn output_json(results: &[libretto_repository::PackageSearchResult]) -> Result<()> {
    let output: Vec<_> = results
        .iter()
        .map(|r| {
            sonic_rs::json!({
                "name": r.name,
                "description": r.description,
                "url": format!("https://packagist.org/packages/{}", r.name),
                "repository": r.repository,
                "downloads": r.downloads,
                "favers": r.favers,
                "abandoned": r.abandoned,
                "replacement": r.replacement
            })
        })
        .collect();

    println!("{}", sonic_rs::to_string_pretty(&output)?);
    Ok(())
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len - 3])
    }
}
