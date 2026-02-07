//! Snapshot tests for CLI output stability.
//!
//! These tests ensure that CLI output format remains stable across versions.
//! Uses insta for snapshot testing.

use assert_cmd::cargo_bin;
use insta::{assert_snapshot, with_settings};
use std::process::Command;
use tempfile::TempDir;

/// Get the libretto binary command.
fn libretto() -> Command {
    Command::new(cargo_bin!("libretto"))
}

/// Run a command and capture stdout.
fn run_and_capture(args: &[&str]) -> String {
    let output = libretto()
        .args(args)
        .output()
        .expect("Failed to execute command");

    String::from_utf8_lossy(&output.stdout).to_string()
}

/// Run a command in a directory and capture stdout.
fn run_in_dir_and_capture(args: &[&str], dir: &std::path::Path) -> String {
    let output = libretto()
        .args(args)
        .current_dir(dir)
        .output()
        .expect("Failed to execute command");

    String::from_utf8_lossy(&output.stdout).to_string()
}

/// Normalize output by removing version-specific and platform-specific information.
fn normalize_output(output: &str) -> String {
    // Remove version numbers that might change
    let output = regex::Regex::new(r"\d+\.\d+\.\d+")
        .unwrap()
        .replace_all(output, "X.X.X");

    // Remove timestamps
    let output = regex::Regex::new(r"\d{4}-\d{2}-\d{2}")
        .unwrap()
        .replace_all(&output, "YYYY-MM-DD");

    // Remove absolute paths (Unix style)
    let output = regex::Regex::new(r"/[^\s]+/libretto")
        .unwrap()
        .replace_all(&output, "/path/to/libretto");

    // Normalize binary name: libretto.exe -> libretto (Windows compatibility)
    let output = output.replace("libretto.exe", "libretto");

    // Normalize bullet points: * -> • (Windows uses * instead of •)
    // Only replace * at the start of a line with optional leading whitespace
    let output = regex::Regex::new(r"(?m)^(\s*)\* ")
        .unwrap()
        .replace_all(&output, "$1• ");

    // Normalize error prefix: [ERR] -> ✘ (Windows compatibility)
    let output = output.replace("[ERR]", "✘");

    output.clone()
}

// ========== Help Output Snapshots ==========

#[test]
fn snapshot_main_help() {
    let output = run_and_capture(&["--help"]);
    let normalized = normalize_output(&output);

    with_settings!({
        description => "Main help output",
        omit_expression => true,
    }, {
        assert_snapshot!("main_help", normalized);
    });
}

#[test]
fn snapshot_install_help() {
    let output = run_and_capture(&["install", "--help"]);
    let normalized = normalize_output(&output);

    with_settings!({
        description => "Install command help",
        omit_expression => true,
    }, {
        assert_snapshot!("install_help", normalized);
    });
}

#[test]
fn snapshot_update_help() {
    let output = run_and_capture(&["update", "--help"]);
    let normalized = normalize_output(&output);

    with_settings!({
        description => "Update command help",
        omit_expression => true,
    }, {
        assert_snapshot!("update_help", normalized);
    });
}

#[test]
fn snapshot_require_help() {
    let output = run_and_capture(&["require", "--help"]);
    let normalized = normalize_output(&output);

    with_settings!({
        description => "Require command help",
        omit_expression => true,
    }, {
        assert_snapshot!("require_help", normalized);
    });
}

#[test]
fn snapshot_remove_help() {
    let output = run_and_capture(&["remove", "--help"]);
    let normalized = normalize_output(&output);

    with_settings!({
        description => "Remove command help",
        omit_expression => true,
    }, {
        assert_snapshot!("remove_help", normalized);
    });
}

#[test]
fn snapshot_show_help() {
    let output = run_and_capture(&["show", "--help"]);
    let normalized = normalize_output(&output);

    with_settings!({
        description => "Show command help",
        omit_expression => true,
    }, {
        assert_snapshot!("show_help", normalized);
    });
}

#[test]
fn snapshot_search_help() {
    let output = run_and_capture(&["search", "--help"]);
    let normalized = normalize_output(&output);

    with_settings!({
        description => "Search command help",
        omit_expression => true,
    }, {
        assert_snapshot!("search_help", normalized);
    });
}

#[test]
fn snapshot_validate_help() {
    let output = run_and_capture(&["validate", "--help"]);
    let normalized = normalize_output(&output);

    with_settings!({
        description => "Validate command help",
        omit_expression => true,
    }, {
        assert_snapshot!("validate_help", normalized);
    });
}

#[test]
fn snapshot_dump_autoload_help() {
    let output = run_and_capture(&["dump-autoload", "--help"]);
    let normalized = normalize_output(&output);

    with_settings!({
        description => "Dump-autoload command help",
        omit_expression => true,
    }, {
        assert_snapshot!("dump_autoload_help", normalized);
    });
}

#[test]
fn snapshot_audit_help() {
    let output = run_and_capture(&["audit", "--help"]);
    let normalized = normalize_output(&output);

    with_settings!({
        description => "Audit command help",
        omit_expression => true,
    }, {
        assert_snapshot!("audit_help", normalized);
    });
}

#[test]
fn snapshot_init_help() {
    let output = run_and_capture(&["init", "--help"]);
    let normalized = normalize_output(&output);

    with_settings!({
        description => "Init command help",
        omit_expression => true,
    }, {
        assert_snapshot!("init_help", normalized);
    });
}

#[test]
fn snapshot_outdated_help() {
    let output = run_and_capture(&["outdated", "--help"]);
    let normalized = normalize_output(&output);

    with_settings!({
        description => "Outdated command help",
        omit_expression => true,
    }, {
        assert_snapshot!("outdated_help", normalized);
    });
}

#[test]
fn snapshot_config_help() {
    let output = run_and_capture(&["config", "--help"]);
    let normalized = normalize_output(&output);

    with_settings!({
        description => "Config command help",
        omit_expression => true,
    }, {
        assert_snapshot!("config_help", normalized);
    });
}

#[test]
fn snapshot_licenses_help() {
    let output = run_and_capture(&["licenses", "--help"]);
    let normalized = normalize_output(&output);

    with_settings!({
        description => "Licenses command help",
        omit_expression => true,
    }, {
        assert_snapshot!("licenses_help", normalized);
    });
}

#[test]
fn snapshot_diagnose_help() {
    let output = run_and_capture(&["diagnose", "--help"]);
    let normalized = normalize_output(&output);

    with_settings!({
        description => "Diagnose command help",
        omit_expression => true,
    }, {
        assert_snapshot!("diagnose_help", normalized);
    });
}

#[test]
fn snapshot_cache_clear_help() {
    let output = run_and_capture(&["cache:clear", "--help"]);
    let normalized = normalize_output(&output);

    with_settings!({
        description => "Cache clear command help",
        omit_expression => true,
    }, {
        assert_snapshot!("cache_clear_help", normalized);
    });
}

// ========== About Command Snapshot ==========

#[test]
fn snapshot_about() {
    let output = run_and_capture(&["about"]);
    let normalized = normalize_output(&output);

    with_settings!({
        description => "About command output",
        omit_expression => true,
    }, {
        assert_snapshot!("about", normalized);
    });
}

// ========== Completion Snapshots ==========

#[test]
fn snapshot_completion_bash() {
    let output = run_and_capture(&["completion", "bash"]);
    // Don't normalize completions - they should be stable

    with_settings!({
        description => "Bash completion script",
        omit_expression => true,
    }, {
        assert_snapshot!("completion_bash", output);
    });
}

#[test]
fn snapshot_completion_zsh() {
    let output = run_and_capture(&["completion", "zsh"]);

    with_settings!({
        description => "Zsh completion script",
        omit_expression => true,
    }, {
        assert_snapshot!("completion_zsh", output);
    });
}

#[test]
fn snapshot_completion_fish() {
    let output = run_and_capture(&["completion", "fish"]);

    with_settings!({
        description => "Fish completion script",
        omit_expression => true,
    }, {
        assert_snapshot!("completion_fish", output);
    });
}

// ========== Validate Command Snapshots ==========

#[test]
fn snapshot_validate_valid() {
    let temp = TempDir::new().expect("Failed to create temp dir");
    std::fs::write(
        temp.path().join("composer.json"),
        r#"{
            "name": "test/project",
            "description": "A test project",
            "type": "project",
            "require": {
                "php": ">=8.0"
            },
            "autoload": {
                "psr-4": {
                    "App\\": "src/"
                }
            }
        }"#,
    )
    .expect("Failed to write composer.json");

    let output = run_in_dir_and_capture(&["validate"], temp.path());

    with_settings!({
        description => "Validate command with valid composer.json",
        omit_expression => true,
    }, {
        assert_snapshot!("validate_valid", output);
    });
}

// ========== Show Command Snapshots ==========

#[test]
fn snapshot_show_empty_project() {
    let temp = TempDir::new().expect("Failed to create temp dir");
    std::fs::write(
        temp.path().join("composer.json"),
        r#"{
            "name": "test/project",
            "require": {}
        }"#,
    )
    .expect("Failed to write composer.json");

    let output = run_in_dir_and_capture(&["show"], temp.path());

    with_settings!({
        description => "Show command with no packages",
        omit_expression => true,
    }, {
        assert_snapshot!("show_empty", output);
    });
}

// ========== Error Output Snapshots ==========

#[test]
fn snapshot_error_no_composer_json() {
    let temp = TempDir::new().expect("Failed to create temp dir");

    let output = libretto()
        .arg("install")
        .current_dir(temp.path())
        .output()
        .expect("Failed to execute command");

    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let normalized = normalize_output(&stderr);

    with_settings!({
        description => "Error when composer.json is missing",
        omit_expression => true,
    }, {
        assert_snapshot!("error_no_composer_json", normalized);
    });
}

#[test]
fn snapshot_error_invalid_json() {
    let temp = TempDir::new().expect("Failed to create temp dir");
    std::fs::write(temp.path().join("composer.json"), "{ invalid json }")
        .expect("Failed to write composer.json");

    let output = libretto()
        .arg("validate")
        .current_dir(temp.path())
        .output()
        .expect("Failed to execute command");

    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let normalized = normalize_output(&stderr);

    with_settings!({
        description => "Error with invalid JSON",
        omit_expression => true,
    }, {
        assert_snapshot!("error_invalid_json", normalized);
    });
}

#[test]
fn snapshot_error_invalid_command() {
    let output = libretto()
        .arg("nonexistent-command")
        .output()
        .expect("Failed to execute command");

    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let normalized = normalize_output(&stderr);

    with_settings!({
        description => "Error with invalid command",
        omit_expression => true,
    }, {
        assert_snapshot!("error_invalid_command", normalized);
    });
}
