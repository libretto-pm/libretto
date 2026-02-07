//! Run-script command - execute scripts from composer.json.

use anyhow::{Context, Result, bail};
use clap::Args;
use sonic_rs::{JsonContainerTrait, JsonValueTrait};
use std::collections::HashMap;
use std::process::Stdio;
use std::time::{Duration, Instant};

/// Arguments for the run-script command
#[derive(Args, Debug, Clone)]
pub struct RunScriptArgs {
    /// Script name to run
    #[arg(value_name = "SCRIPT")]
    pub script: Option<String>,

    /// Arguments to pass to the script
    #[arg(trailing_var_arg = true, value_name = "ARGS")]
    pub args: Vec<String>,

    /// List available scripts
    #[arg(short = 'l', long)]
    pub list: bool,

    /// Set script timeout in seconds (0 for no timeout)
    #[arg(long, default_value = "300")]
    pub timeout: u64,

    /// Run in dev mode (includes dev dependencies in path)
    #[arg(long)]
    pub dev: bool,

    /// Run in no-dev mode (excludes dev dependencies)
    #[arg(long)]
    pub no_dev: bool,
}

/// Run the run-script command
pub async fn run(args: RunScriptArgs) -> Result<()> {
    use crate::output::{error, header, info, success};

    let composer_path = std::env::current_dir()?.join("composer.json");
    if !composer_path.exists() {
        anyhow::bail!("composer.json not found in current directory");
    }

    let composer_content = std::fs::read_to_string(&composer_path)?;
    let composer: sonic_rs::Value = sonic_rs::from_str(&composer_content)?;

    // Get scripts section
    let scripts = composer.get("scripts").and_then(|s| s.as_object());

    // Handle list mode
    if args.list || args.script.is_none() {
        header("Available scripts");
        return list_scripts(&composer);
    }

    let script_name = args.script.as_ref().unwrap();

    // Check if script exists
    let script = scripts
        .and_then(|s| s.get(script_name))
        .context(format!("Script '{script_name}' not found"))?;

    header(&format!("Running script: {script_name}"));

    // Collect commands to run
    let commands: Vec<String> = if let Some(cmd) = script.as_str() {
        vec![cmd.to_string()]
    } else if let Some(arr) = script.as_array() {
        arr.iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect()
    } else {
        anyhow::bail!("Invalid script format for '{script_name}'")
    };

    // Set up environment
    let mut env: HashMap<String, String> = std::env::vars().collect();

    // Add vendor/bin to PATH
    let vendor_bin = std::env::current_dir()?.join("vendor").join("bin");
    if vendor_bin.exists() {
        let path = env.get("PATH").cloned().unwrap_or_default();
        let new_path = format!(
            "{}{}{}",
            vendor_bin.display(),
            if cfg!(windows) { ";" } else { ":" },
            path
        );
        env.insert("PATH".to_string(), new_path);
    }

    // Add COMPOSER_* environment variables
    env.insert(
        "COMPOSER_BINARY".to_string(),
        std::env::current_exe()?.to_string_lossy().to_string(),
    );
    env.insert(
        "COMPOSER_DEV_MODE".to_string(),
        if args.dev { "1" } else { "0" }.to_string(),
    );

    // Run each command
    let mut success_count = 0;
    let mut failed = false;

    for cmd in &commands {
        // Check for @reference syntax
        if let Some(ref_script) = cmd.strip_prefix('@') {
            // Run referenced script
            let ref_args = RunScriptArgs {
                script: Some(ref_script.to_string()),
                args: args.args.clone(),
                list: false,
                timeout: args.timeout,
                dev: args.dev,
                no_dev: args.no_dev,
            };
            if let Err(e) = Box::pin(run(ref_args)).await {
                error(&format!("Referenced script '{ref_script}' failed: {e}"));
                failed = true;
                break;
            }
            success_count += 1;
            continue;
        }

        // Check for @php or @composer
        let actual_cmd = if cmd.starts_with("@php ") {
            cmd.replacen("@php ", "php ", 1)
        } else if cmd.starts_with("@composer ") {
            cmd.replacen(
                "@composer ",
                &format!("{} ", std::env::current_exe()?.display()),
                1,
            )
        } else if cmd.starts_with("@putenv ") {
            // Handle @putenv directive
            let putenv = cmd.strip_prefix("@putenv ").unwrap();
            if let Some((key, value)) = putenv.split_once('=') {
                env.insert(key.to_string(), value.to_string());
            }
            continue;
        } else {
            cmd.clone()
        };

        // Append script arguments
        let full_cmd = if args.args.is_empty() {
            actual_cmd
        } else {
            format!("{} {}", actual_cmd, args.args.join(" "))
        };

        info(&format!("> {full_cmd}"));

        // Execute command with timeout enforcement
        let shell = if cfg!(windows) { "cmd" } else { "sh" };
        let shell_arg = if cfg!(windows) { "/C" } else { "-c" };

        let status = if args.timeout > 0 {
            // Spawn process and enforce timeout
            let mut child = std::process::Command::new(shell)
                .arg(shell_arg)
                .arg(&full_cmd)
                .envs(&env)
                .stdin(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .spawn()
                .context(format!("Failed to spawn: {full_cmd}"))?;

            let timeout_duration = Duration::from_secs(args.timeout);
            let start = Instant::now();

            let mut timed_out = false;
            let status = loop {
                match child.try_wait() {
                    Ok(Some(status)) => break Some(status),
                    Ok(None) => {
                        if start.elapsed() >= timeout_duration {
                            let _ = child.kill();
                            let _ = child.wait();
                            error(&format!(
                                "Script timed out after {}s: {}",
                                args.timeout, full_cmd
                            ));
                            timed_out = true;
                            break None;
                        }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    Err(e) => {
                        bail!("Failed to wait for process: {e}");
                    }
                }
            };

            if timed_out {
                failed = true;
                break;
            }

            status.unwrap()
        } else {
            // No timeout, wait indefinitely
            std::process::Command::new(shell)
                .arg(shell_arg)
                .arg(&full_cmd)
                .envs(&env)
                .status()
                .context(format!("Failed to execute: {full_cmd}"))?
        };

        if !status.success() {
            error(&format!(
                "Command failed with exit code {}",
                status.code().unwrap_or(-1)
            ));
            failed = true;
            break;
        }

        success_count += 1;
    }

    if failed {
        anyhow::bail!("Script '{script_name}' failed");
    }

    success(&format!(
        "Script '{script_name}' completed ({success_count} command(s) executed)"
    ));

    Ok(())
}

fn list_scripts(composer: &sonic_rs::Value) -> Result<()> {
    use crate::output::table::Table;

    let scripts = composer.get("scripts").and_then(|s| s.as_object());

    if scripts.is_none() {
        crate::output::info("No scripts defined in composer.json");
        return Ok(());
    }

    let scripts = scripts.unwrap();

    // Separate event scripts from custom scripts
    let event_scripts = [
        "pre-install-cmd",
        "post-install-cmd",
        "pre-update-cmd",
        "post-update-cmd",
        "pre-status-cmd",
        "post-status-cmd",
        "pre-archive-cmd",
        "post-archive-cmd",
        "pre-autoload-dump",
        "post-autoload-dump",
        "post-root-package-install",
        "post-create-project-cmd",
        "pre-operations-exec",
        "pre-pool-create",
    ];

    let mut custom: Vec<(String, String)> = Vec::new();
    let mut events: Vec<(String, String)> = Vec::new();

    for (name, cmd) in scripts {
        let cmd_str = if let Some(s) = cmd.as_str() {
            s.to_string()
        } else if let Some(arr) = cmd.as_array() {
            let cmds: Vec<String> = arr
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
            if cmds.len() == 1 {
                cmds[0].clone()
            } else {
                format!("[{} commands]", cmds.len())
            }
        } else {
            continue;
        };

        // Skip description fields
        if name.ends_with("-descriptions") {
            continue;
        }

        let name_str = name.to_string();
        if event_scripts.contains(&name_str.as_str()) {
            events.push((name_str, cmd_str));
        } else {
            custom.push((name_str, cmd_str));
        }
    }

    // Get descriptions if available
    let descriptions = composer
        .get("scripts-descriptions")
        .and_then(|d| d.as_object());

    if !custom.is_empty() {
        println!("Custom scripts:");

        let mut table = Table::new();
        table.headers(["Script", "Description"]);

        custom.sort_by(|a, b| a.0.cmp(&b.0));

        for (name, cmd) in &custom {
            let desc = descriptions
                .and_then(|d| d.get(&name))
                .and_then(|d| d.as_str())
                .unwrap_or(cmd);

            table.row([name.as_str(), desc]);
        }

        table.print();
    }

    if !events.is_empty() {
        println!();
        println!("Event scripts:");

        let mut table = Table::new();
        table.headers(["Event", "Commands"]);

        events.sort_by(|a, b| a.0.cmp(&b.0));

        for (name, cmd) in &events {
            table.row([name.as_str(), cmd.as_str()]);
        }

        table.print();
    }

    if custom.is_empty() && events.is_empty() {
        crate::output::info("No scripts defined");
    }

    Ok(())
}
