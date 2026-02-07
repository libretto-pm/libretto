//! Terminal output utilities for beautiful CLI output.
//!
//! Provides unified styling, progress bars, tables, and interactive prompts
//! with support for TTY detection, `NO_COLOR` environment variable, and
//! graceful degradation to ASCII when Unicode is not supported.
//!
//! These utilities provide a complete UI toolkit for CLI commands.

#![allow(dead_code)]

pub mod colors;
pub mod json;
pub mod live;
pub mod progress;
pub mod prompt;
pub mod style;
pub mod table;

// JSON output functions available via json:: module but not re-exported
pub use style::{Icon, OutputMode, Theme};

use std::io::{IsTerminal, stderr, stdout};
use std::sync::atomic::{AtomicBool, Ordering};

/// Global color configuration
static COLOR_ENABLED: AtomicBool = AtomicBool::new(true);
static UNICODE_ENABLED: AtomicBool = AtomicBool::new(true);

/// Detect if we're running in a terminal
static IS_TTY: std::sync::LazyLock<bool> =
    std::sync::LazyLock::new(|| stdout().is_terminal() && stderr().is_terminal());

/// Check if `NO_COLOR` environment variable is set
static NO_COLOR: std::sync::LazyLock<bool> =
    std::sync::LazyLock::new(|| std::env::var("NO_COLOR").is_ok());

/// Initialize output settings based on environment and flags
pub fn init(force_ansi: Option<bool>, quiet: bool) {
    let colors = match force_ansi {
        Some(true) => true,
        Some(false) => false,
        None => *IS_TTY && !*NO_COLOR,
    };
    COLOR_ENABLED.store(colors, Ordering::Relaxed);

    // Detect Unicode support (simple heuristic: check LANG/LC_ALL)
    let unicode = std::env::var("LANG")
        .or_else(|_| std::env::var("LC_ALL"))
        .map(|l| l.contains("UTF") || l.contains("utf"))
        .unwrap_or(cfg!(not(windows)));
    UNICODE_ENABLED.store(unicode && !quiet, Ordering::Relaxed);
}

/// Check if colors are enabled
pub fn colors_enabled() -> bool {
    COLOR_ENABLED.load(Ordering::Relaxed)
}

/// Check if Unicode is supported
pub fn unicode_enabled() -> bool {
    UNICODE_ENABLED.load(Ordering::Relaxed)
}

/// Check if we're in a TTY
pub fn is_tty() -> bool {
    *IS_TTY
}

/// Print a styled header
pub fn header(text: &str) {
    use owo_colors::OwoColorize;
    if colors_enabled() {
        println!("{} {}", "Libretto".cyan().bold(), text.dimmed());
    } else {
        println!("Libretto {text}");
    }
}

/// Print a success message
pub fn success(text: &str) {
    use owo_colors::OwoColorize;
    let icon = if unicode_enabled() {
        Icon::Success.as_str()
    } else {
        Icon::Success.ascii()
    };
    if colors_enabled() {
        println!("{} {}", icon.green(), text);
    } else {
        println!("{icon} {text}");
    }
}

/// Print a warning message
pub fn warning(text: &str) {
    use owo_colors::OwoColorize;
    let icon = if unicode_enabled() {
        Icon::Warning.as_str()
    } else {
        Icon::Warning.ascii()
    };
    if colors_enabled() {
        eprintln!("{} {}", icon.yellow(), text.yellow());
    } else {
        eprintln!("{icon} {text}");
    }
}

/// Print an error message
pub fn error(text: &str) {
    use owo_colors::OwoColorize;
    let icon = if unicode_enabled() {
        Icon::Error.as_str()
    } else {
        Icon::Error.ascii()
    };
    if colors_enabled() {
        eprintln!("{} {}", icon.red(), text.red());
    } else {
        eprintln!("{icon} {text}");
    }
}

/// Print an info message
pub fn info(text: &str) {
    use owo_colors::OwoColorize;
    let icon = if unicode_enabled() {
        Icon::Info.as_str()
    } else {
        Icon::Info.ascii()
    };
    if colors_enabled() {
        println!("{} {}", icon.blue(), text);
    } else {
        println!("{icon} {text}");
    }
}

/// Print a debug message (only in verbose mode)
pub fn debug(text: &str) {
    use owo_colors::OwoColorize;
    if colors_enabled() {
        eprintln!("{}", text.dimmed());
    } else {
        eprintln!("{text}");
    }
}

/// Print package name with optional version
pub fn package(name: &str, version: Option<&str>) {
    use owo_colors::OwoColorize;
    if colors_enabled() {
        if let Some(v) = version {
            println!("  {} {}", name.green(), v.yellow());
        } else {
            println!("  {}", name.green());
        }
    } else if let Some(v) = version {
        println!("  {name} {v}");
    } else {
        println!("  {name}");
    }
}

/// Format a duration for display
pub fn format_duration(duration: std::time::Duration) -> String {
    let secs = duration.as_secs_f64();
    if secs < 0.001 {
        format!("{:.0}us", secs * 1_000_000.0)
    } else if secs < 1.0 {
        format!("{:.0}ms", secs * 1000.0)
    } else if secs < 60.0 {
        format!("{secs:.2}s")
    } else {
        let mins = secs / 60.0;
        format!("{mins:.1}m")
    }
}

/// Format bytes for display
pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes < KB {
        format!("{bytes} B")
    } else if bytes < MB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else if bytes < GB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_duration() {
        assert!(format_duration(std::time::Duration::from_micros(500)).contains("us"));
        assert!(format_duration(std::time::Duration::from_millis(500)).contains("ms"));
        assert!(format_duration(std::time::Duration::from_secs(5)).contains('s'));
        assert!(format_duration(std::time::Duration::from_secs(120)).contains('m'));
    }

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(500), "500 B");
        assert!(format_bytes(2048).contains("KB"));
        assert!(format_bytes(2 * 1024 * 1024).contains("MB"));
    }
}
