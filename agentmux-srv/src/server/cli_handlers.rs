use std::sync::Arc;

use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{
    CheckCliAuthResult, CommandCheckCliAuthData, CommandResolveCliData, CommandRunCliLoginData,
    ResolveCliResult, RunCliLoginResult, COMMAND_CHECK_CLI_AUTH, COMMAND_RESOLVE_CLI,
};

use super::AppState;

/// Register CLI-related RPC handlers (resolvecli, checkcliauth, runclilogin).
pub fn register_cli_handlers(engine: &Arc<WshRpcEngine>, state: &AppState) {
    // resolvecli → detect or install a CLI tool for an agent provider
    // Each AgentMux version gets its own isolated CLI install at:
    //   ~/.agentmux/<AGENTMUX_VERSION>/cli/<provider>/
    // Never falls back to system PATH.
    let broker_resolve = state.broker.clone();
    engine.register_handler(
        COMMAND_RESOLVE_CLI,
        Box::new(move |data, _ctx| {
            let broker = broker_resolve.clone();
            Box::pin(async move {
                const AGENTMUX_VERSION: &str = env!("CARGO_PKG_VERSION");

                let cmd: CommandResolveCliData = serde_json::from_value(data)
                    .map_err(|e| format!("resolvecli: {e}"))?;
                tracing::info!(
                    provider = %cmd.provider_id,
                    cli = %cmd.cli_command,
                    block_id = %cmd.block_id,
                    agentmux_version = AGENTMUX_VERSION,
                    "ResolveCli"
                );

                // Resolve home directory
                let home = std::env::var("HOME")
                    .or_else(|_| std::env::var("USERPROFILE"))
                    .map_err(|_| "cannot determine home directory".to_string())?;

                // Versioned install directory: ~/.agentmux/<version>/cli/<provider>/
                let provider_dir = format!(
                    "{}/.agentmux/{}/cli/{}",
                    home, AGENTMUX_VERSION, cmd.provider_id
                );
                // npm binary path — the only valid location for installed CLIs.
                let npm_bin = if cfg!(windows) {
                    format!("{}/node_modules/.bin/{}.cmd", provider_dir, cmd.cli_command)
                } else {
                    format!("{}/node_modules/.bin/{}", provider_dir, cmd.cli_command)
                };

                // Step 1: Check if already installed in versioned directory
                if std::path::Path::new(&npm_bin).exists() {
                    let version = get_cli_version(&npm_bin).await;
                    tracing::info!(
                        path = %npm_bin, version = %version,
                        "CLI found in versioned install"
                    );
                    return Ok(Some(serde_json::to_value(&ResolveCliResult {
                        cli_path: npm_bin,
                        version,
                        source: "local_install".to_string(),
                    }).unwrap()));
                }

                // Step 2: Not in versioned dir — install from network.
                // Never copy from system PATH — that defeats version isolation and
                // can copy the wrong binary type (e.g., .exe saved as .cmd).
                if cmd.npm_package.is_empty() {
                    return Err(format!(
                        "{} not found and no npm package configured for this provider",
                        cmd.cli_command
                    ));
                }

                tracing::info!(
                    provider = %cmd.provider_id,
                    npm_package = %cmd.npm_package,
                    pinned_version = %cmd.pinned_version,
                    target_dir = %provider_dir,
                    "CLI not found locally, installing via npm"
                );

                {
                    // Verify npm is available before attempting install.
                    let npm_available = if cfg!(windows) {
                        tokio::process::Command::new("where").arg("npm").output().await
                            .map(|o| o.status.success()).unwrap_or(false)
                    } else {
                        tokio::process::Command::new("which").arg("npm").output().await
                            .map(|o| o.status.success()).unwrap_or(false)
                    };
                    if !npm_available {
                        return Err(format!(
                            "{} requires Node.js/npm to install. \
                            Install Node.js from https://nodejs.org then restart AgentMux.",
                            cmd.cli_command
                        ));
                    }

                    // Use `npm install --prefix <dir> <pkg>@<ver>` to avoid cd+chaining issues.
                    // On Windows, normalize the prefix path to backslashes so npm handles it correctly.
                    // npm.cmd must be invoked via cmd /C on Windows — it's a batch script, not an exe.
                    let prefix_dir = if cfg!(windows) {
                        provider_dir.replace('/', "\\")
                    } else {
                        provider_dir.clone()
                    };
                    let package_arg = format!("{}@{}", cmd.npm_package, cmd.pinned_version);
                    tracing::info!(package = %package_arg, prefix = %prefix_dir, "running npm install");

                    // Collect all npm output after completion via .output().
                    // Pipe-based streaming (both async IOCP and sync blocking) does not receive
                    // data from cmd.exe /C batch script children on Windows — output only becomes
                    // available after the process exits. We run in spawn_blocking and publish all
                    // lines at once when done; users see the full install log after it completes.
                    let block_id_install = cmd.block_id.clone();
                    tracing::info!(block_id = %block_id_install, package = %package_arg, prefix = %prefix_dir, "running npm install");

                    let broker_npm = broker.clone();
                    let exit_status = tokio::task::spawn_blocking(move || {
                        let result = {
                            #[cfg(windows)]
                            {
                                // npm on Windows is a .cmd batch script — must be invoked via cmd.exe /C.
                                // Use raw_arg to pass the command string WITHOUT Rust's CreateProcess
                                // quoting. With .args(["/C", str]), Rust wraps str in outer quotes and
                                // escapes inner quotes as \", which cmd.exe treats as literal backslash+quote,
                                // corrupting paths: CWD + \"C:\path\" → ENOENT.
                                // raw_arg passes the string verbatim; cmd.exe sees:
                                //   cmd /C npm install ... --prefix "C:\path with spaces\..." pkg
                                // and tokenizes "..." as a quoted path correctly.
                                use std::os::windows::process::CommandExt;
                                let npm_cmd_str = format!(
                                    "npm install --loglevel=http --no-audit --no-fund --no-progress --prefix \"{}\" {}",
                                    prefix_dir, package_arg
                                );
                                std::process::Command::new("cmd")
                                    .arg("/C")
                                    .raw_arg(&npm_cmd_str)
                                    .env("CI", "true")
                                    .env("FORCE_COLOR", "0")
                                    .output()
                            }
                            #[cfg(not(windows))]
                            {
                                std::process::Command::new("npm")
                                    .args(["install", "--loglevel=http", "--no-audit", "--no-fund", "--no-progress", "--prefix", &prefix_dir, &package_arg])
                                    .env("CI", "true")
                                    .env("FORCE_COLOR", "0")
                                    .output()
                            }
                        };
                        match result {
                            Ok(out) => {
                                tracing::info!(exit_code = out.status.code().unwrap_or(-1), stdout_bytes = out.stdout.len(), stderr_bytes = out.stderr.len(), "npm install output collected");
                                // Publish stderr first (npm writes progress/errors there), then stdout
                                for line in String::from_utf8_lossy(&out.stderr).lines() {
                                    if !line.trim().is_empty() {
                                        tracing::info!(line = %line, "npm stderr");
                                        if !block_id_install.is_empty() {
                                            crate::backend::wps::publish_install_progress(&broker_npm, &block_id_install, line);
                                        }
                                    }
                                }
                                for line in String::from_utf8_lossy(&out.stdout).lines() {
                                    if !line.trim().is_empty() {
                                        tracing::info!(line = %line, "npm stdout");
                                        if !block_id_install.is_empty() {
                                            crate::backend::wps::publish_install_progress(&broker_npm, &block_id_install, line);
                                        }
                                    }
                                }
                                Ok(out.status)
                            }
                            Err(e) => Err(format!("failed to run npm install: {e}")),
                        }
                    }).await
                        .map_err(|e| format!("npm spawn_blocking panicked: {e}"))?
                        .map_err(|e| e)?;
                    tracing::info!(exit_code = exit_status.code().unwrap_or(-1), "npm install completed");

                    if !exit_status.success() {
                        return Err(format!(
                            "npm install failed (exit {}). Check the output above for details.",
                            exit_status.code().unwrap_or(-1)
                        ));
                    }

                    // Verify npm binary exists
                    if std::path::Path::new(&npm_bin).exists() {
                        let version = get_cli_version(&npm_bin).await;
                        tracing::info!(path = %npm_bin, version = %version, "CLI installed (npm)");
                        return Ok(Some(serde_json::to_value(&ResolveCliResult {
                            cli_path: npm_bin,
                            version,
                            source: "installed".to_string(),
                        }).unwrap()));
                    }

                    Err(format!(
                        "npm install completed but binary not found at {}",
                        npm_bin
                    ))
                }
            })
        }),
    );

    // checkcliauth → check if a CLI tool is authenticated
    // For Claude: reads ~/.claude/.credentials.json directly (instant, no subprocess).
    // For other providers: falls back to running the CLI auth check command.
    engine.register_handler(
        COMMAND_CHECK_CLI_AUTH,
        Box::new(|data, _ctx| {
            Box::pin(async move {
                let cmd: CommandCheckCliAuthData = serde_json::from_value(data)
                    .map_err(|e| format!("checkcliauth: {e}"))?;
                tracing::info!(cli = %cmd.cli_path, "CheckCliAuth");

                // Run CLI auth check command — all providers including Claude.
                //
                // A previous "fast path" read ~/.claude/.credentials.json directly and
                // declared authenticated=true if any token string was present. This caused
                // false positives: stale, expired, and revoked tokens all passed the check,
                // and the `email` field was set to the subscription tier ("max", "pro") instead
                // of the user's email address. See SPEC_AUTH_CHECK_FALSE_POSITIVE_2026_04_15.md.
                let output = tokio::time::timeout(
                    std::time::Duration::from_secs(25),
                    {
                        let mut check_cmd = make_cli_cmd(&cmd.cli_path);
                        check_cmd.args(&cmd.auth_check_args);
                        for (k, v) in &cmd.auth_env {
                            check_cmd.env(k, v);
                        }
                        check_cmd.output()
                    },
                ).await
                    .map_err(|_| "auth check timed out (25s)".to_string())?
                    .map_err(|e| format!("failed to run auth check: {e}"))?;

                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();

                let mut email = None;
                let mut auth_method = None;

                let authenticated = if let Ok(json) = serde_json::from_str::<serde_json::Value>(&stdout) {
                    // Claude outputs `emailAddress`; other CLIs use `email`. Check both.
                    email = json.get("emailAddress")
                        .or_else(|| json.get("email"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    auth_method = json.get("authMethod")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    json.get("loggedIn")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                } else {
                    output.status.success()
                };

                let raw_output = if !stdout.is_empty() { stdout } else { stderr };

                let result = CheckCliAuthResult {
                    authenticated,
                    email,
                    auth_method,
                    raw_output,
                };
                Ok(Some(serde_json::to_value(&result).unwrap()))
            })
        }),
    );

    // runclilogin → spawn CLI login flow, extract OAuth URL from output, return immediately
    engine.register_handler(
        "runclilogin",
        Box::new(|data, _ctx| {
            Box::pin(async move {
                let cmd: CommandRunCliLoginData = serde_json::from_value(data)
                    .map_err(|e| format!("runclilogin: {e}"))?;
                tracing::info!(cli = %cmd.cli_path, args = ?cmd.login_args, "RunCliLogin");

                // Spawn the login process. On most platforms it opens the browser
                // automatically and writes the URL to stderr. On Windows, stderr is
                // block-buffered when piped so we can't reliably read it in real-time.
                // Strategy: inherit stdout/stderr so the CLI can open the browser normally,
                // then return immediately — the frontend polls auth status until done.
                let mut child = make_cli_cmd(&cmd.cli_path)
                    .args(&cmd.login_args)
                    .envs(&cmd.auth_env)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                    .map_err(|e| format!("failed to spawn login: {e}"))?;

                // Keep child alive in background — it waits for the user to complete OAuth
                tokio::spawn(async move { let _ = child.wait().await; });

                let result = RunCliLoginResult { auth_url: None, raw_output: String::new() };
                Ok(Some(serde_json::to_value(&result).unwrap()))
            })
        }),
    );
}

/// Re-export from shared crate for internal use.
pub(crate) fn make_cli_cmd(cli_path: &str) -> tokio::process::Command {
    agentmux_common::make_cli_cmd(cli_path)
}

async fn get_cli_version(cli_path: &str) -> String {
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        make_cli_cmd(cli_path).arg("--version").output(),
    ).await;
    match result {
        Ok(Ok(output)) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        }
        Ok(_) => "unknown".to_string(),
        Err(_) => {
            tracing::warn!(cli_path = %cli_path, "get_cli_version timed out after 5s");
            "unknown".to_string()
        }
    }
}
