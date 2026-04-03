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
                let bin_dir = format!("{}/bin", provider_dir);

                // Expected binary path.
                // On Windows, Node.js CLIs installed by npm are .cmd batch wrappers, not .exe.
                // Use .cmd so make_cli_cmd() routes them through cmd.exe /C correctly.
                let cli_bin = if cfg!(windows) {
                    format!("{}/{}.cmd", bin_dir, cmd.cli_command)
                } else {
                    format!("{}/{}", bin_dir, cmd.cli_command)
                };

                // Also check npm-style path (for npm-based providers like codex/gemini)
                let npm_bin = if cfg!(windows) {
                    format!("{}/node_modules/.bin/{}.cmd", provider_dir, cmd.cli_command)
                } else {
                    format!("{}/node_modules/.bin/{}", provider_dir, cmd.cli_command)
                };

                // Step 1: Check if already installed in versioned directory
                for candidate in [&cli_bin, &npm_bin] {
                    if std::path::Path::new(candidate).exists() {
                        let version = get_cli_version(candidate).await;
                        tracing::info!(
                            path = %candidate, version = %version,
                            "CLI found in versioned install"
                        );
                        return Ok(Some(serde_json::to_value(&ResolveCliResult {
                            cli_path: candidate.clone(),
                            version,
                            source: "local_install".to_string(),
                        }).unwrap()));
                    }
                }

                // Step 2: Not in versioned dir yet. Try to copy from a known location.
                let exe_name = if cfg!(windows) {
                    format!("{}.exe", cmd.cli_command)
                } else {
                    cmd.cli_command.clone()
                };

                // Known locations where CLIs get installed on the system
                let known_paths: Vec<String> = vec![
                    format!("{}/.local/bin/{}", home, exe_name),
                    format!("{}/.claude/local/bin/{}", home, exe_name),
                    format!("{}/AppData/Local/Programs/{}/{}", home, cmd.cli_command, exe_name),
                ];

                // Also check PATH via where/which (to find binary, NOT to use directly)
                let mut system_bin: Option<String> = None;
                for path in &known_paths {
                    if std::path::Path::new(path).exists() {
                        system_bin = Some(path.clone());
                        break;
                    }
                }
                if system_bin.is_none() {
                    let which_cmd = if cfg!(windows) { "where" } else { "which" };
                    if let Ok(output) = tokio::process::Command::new(which_cmd)
                        .arg(&cmd.cli_command)
                        .output()
                        .await
                    {
                        if output.status.success() {
                            let path = String::from_utf8_lossy(&output.stdout)
                                .lines().next().unwrap_or("").trim().to_string();
                            if !path.is_empty() && std::path::Path::new(&path).exists() {
                                // On Windows, npm CLIs ship as both a bare script (no ext) and a
                                // .cmd batch wrapper. Prefer the .cmd sibling so make_cli_cmd()
                                // routes it through cmd.exe /C correctly.  If the path already
                                // has .cmd/.bat/.exe we use it as-is.
                                #[cfg(windows)]
                                {
                                    let p = std::path::Path::new(&path);
                                    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
                                    if ext.eq_ignore_ascii_case("cmd") || ext.eq_ignore_ascii_case("bat") || ext.eq_ignore_ascii_case("exe") {
                                        system_bin = Some(path);
                                    } else {
                                        let cmd_sibling = p.with_extension("cmd").to_string_lossy().to_string();
                                        if std::path::Path::new(&cmd_sibling).exists() {
                                            tracing::info!(script = %path, cmd = %cmd_sibling, "preferring .cmd sibling over bare script");
                                            system_bin = Some(cmd_sibling);
                                        } else {
                                            system_bin = Some(path);
                                        }
                                    }
                                }
                                #[cfg(not(windows))]
                                {
                                    system_bin = Some(path);
                                }
                            }
                        }
                    }
                }

                // Create versioned directory
                std::fs::create_dir_all(&bin_dir).map_err(|e| {
                    format!("failed to create {}: {e}", bin_dir)
                })?;

                // Fast path: copy existing binary to versioned dir (no network needed)
                if let Some(ref source) = system_bin {
                    tracing::info!(
                        source = %source, target = %cli_bin,
                        "copying existing CLI binary to versioned directory"
                    );
                    std::fs::copy(source, &cli_bin).map_err(|e| {
                        format!("failed to copy {} → {}: {e}", source, cli_bin)
                    })?;
                    let version = get_cli_version(&cli_bin).await;
                    tracing::info!(path = %cli_bin, version = %version, "CLI copied to versioned dir");
                    return Ok(Some(serde_json::to_value(&ResolveCliResult {
                        cli_path: cli_bin,
                        version,
                        source: "local_install".to_string(),
                    }).unwrap()));
                }

                // Slow path: binary not found anywhere — need to install from network
                let install_cmd = if cfg!(windows) {
                    &cmd.windows_install_command
                } else {
                    &cmd.unix_install_command
                };

                if install_cmd.is_empty() {
                    return Err(format!(
                        "{} not found and no install command configured for this provider",
                        cmd.cli_command
                    ));
                }

                tracing::info!(
                    provider = %cmd.provider_id,
                    install_cmd = %install_cmd,
                    target_dir = %provider_dir,
                    "CLI not found locally, installing from network"
                );

                // Determine if this is an npm-based provider or official installer
                let is_npm_install = install_cmd.contains("npm install");

                if is_npm_install {
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

                    return Err(format!(
                        "npm install completed but binary not found at {}",
                        npm_bin
                    ));
                }

                // Official installer (Claude): run installer streaming output to block
                let mut child_installer = if cfg!(windows) {
                    tokio::process::Command::new("powershell")
                        .args(["-NoProfile", "-Command", install_cmd])
                        .stdout(std::process::Stdio::piped())
                        .stderr(std::process::Stdio::piped())
                        .spawn()
                } else {
                    tokio::process::Command::new("bash")
                        .args(["-c", install_cmd])
                        .stdout(std::process::Stdio::piped())
                        .stderr(std::process::Stdio::piped())
                        .spawn()
                }.map_err(|e| format!("failed to spawn installer: {e}"))?;

                let block_id_inst = cmd.block_id.clone();
                let broker_inst = broker.clone();
                let stdout_inst = child_installer.stdout.take();
                let stderr_inst = child_installer.stderr.take();

                let block_id_inst2 = cmd.block_id.clone();
                let broker_inst2 = broker.clone();
                let (_, _, install_exit) = tokio::time::timeout(
                    std::time::Duration::from_secs(120),
                    async move {
                        tokio::join!(
                            async move {
                                if let Some(p) = stdout_inst {
                                    use tokio::io::AsyncBufReadExt;
                                    let mut reader = tokio::io::BufReader::new(p).lines();
                                    while let Ok(Some(line)) = reader.next_line().await {
                                        tracing::info!(line = %line, "installer stdout");
                                        if !block_id_inst.is_empty() {
                                            crate::backend::wps::publish_install_progress(&broker_inst, &block_id_inst, &line);
                                        }
                                    }
                                }
                            },
                            async move {
                                if let Some(p) = stderr_inst {
                                    use tokio::io::AsyncBufReadExt;
                                    let mut reader = tokio::io::BufReader::new(p).lines();
                                    while let Ok(Some(line)) = reader.next_line().await {
                                        tracing::info!(line = %line, "installer stderr");
                                        if !block_id_inst2.is_empty() {
                                            crate::backend::wps::publish_install_progress(&broker_inst2, &block_id_inst2, &line);
                                        }
                                    }
                                }
                            },
                            child_installer.wait(),
                        )
                    }
                ).await.map_err(|_| format!("install timed out after 120s — try manually:\n  {}", install_cmd))?;

                let install_exit = install_exit.map_err(|e| format!("installer wait failed: {e}"))?;
                tracing::info!(exit_code = install_exit.code().unwrap_or(-1), "official installer completed");

                if !install_exit.success() {
                    return Err(format!(
                        "installer failed (exit {}). Check output above for details.",
                        install_exit.code().unwrap_or(-1)
                    ));
                }

                // Find where the official installer placed the binary
                let search_paths = known_paths;

                let mut found_source: Option<String> = None;
                for search in &search_paths {
                    if std::path::Path::new(search).exists() {
                        found_source = Some(search.clone());
                        break;
                    }
                }

                // Also try `where`/`which` as last resort to find installed binary
                if found_source.is_none() {
                    let which_cmd = if cfg!(windows) { "where" } else { "which" };
                    if let Ok(output) = tokio::process::Command::new(which_cmd)
                        .arg(&cmd.cli_command)
                        .output()
                        .await
                    {
                        if output.status.success() {
                            let path = String::from_utf8_lossy(&output.stdout)
                                .lines().next().unwrap_or("").trim().to_string();
                            if !path.is_empty() {
                                found_source = Some(path);
                            }
                        }
                    }
                }

                let source_path = found_source.ok_or_else(|| format!(
                    "installer ran successfully but cannot find {} binary. \
                     Searched: {:?}",
                    cmd.cli_command, search_paths
                ))?;

                // Copy binary to versioned directory
                tracing::info!(
                    source = %source_path,
                    target = %cli_bin,
                    "copying CLI binary to versioned directory"
                );
                std::fs::copy(&source_path, &cli_bin).map_err(|e| {
                    format!("failed to copy {} → {}: {e}", source_path, cli_bin)
                })?;

                let version = get_cli_version(&cli_bin).await;
                tracing::info!(path = %cli_bin, version = %version, "CLI installed successfully");
                Ok(Some(serde_json::to_value(&ResolveCliResult {
                    cli_path: cli_bin,
                    version,
                    source: "installed".to_string(),
                }).unwrap()))
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

                // Fast path: read credentials file directly (Claude)
                if cmd.cli_path.contains("claude") {
                    // Use CLAUDE_CONFIG_DIR from auth_env if provided (isolated auth dir).
                    // Fall back to ~/.claude/ for legacy/non-isolated invocations.
                    let creds_path = if let Some(config_dir) = cmd.auth_env.get("CLAUDE_CONFIG_DIR") {
                        format!("{}/.credentials.json", config_dir)
                    } else {
                        let home = std::env::var("HOME")
                            .or_else(|_| std::env::var("USERPROFILE"))
                            .unwrap_or_default();
                        format!("{}/.claude/.credentials.json", home)
                    };

                    if let Ok(content) = std::fs::read_to_string(&creds_path) {
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                            // Check claudeAiOauth credentials
                            let oauth = json.get("claudeAiOauth");
                            let has_token = oauth
                                .and_then(|o| o.get("accessToken"))
                                .and_then(|v| v.as_str())
                                .map(|s| !s.is_empty())
                                .unwrap_or(false);

                            let has_refresh = oauth
                                .and_then(|o| o.get("refreshToken"))
                                .and_then(|v| v.as_str())
                                .map(|s| !s.is_empty())
                                .unwrap_or(false);

                            // Authenticated if we have an access token OR a refresh token
                            // (CLI auto-refreshes expired tokens transparently)
                            let authenticated = has_token || has_refresh;

                            let subscription = oauth
                                .and_then(|o| o.get("subscriptionType"))
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());

                            let auth_method = Some("claude.ai oauth".to_string());

                            tracing::info!(
                                authenticated = authenticated,
                                subscription = ?subscription,
                                has_refresh = has_refresh,
                                "claude auth check (credentials file)"
                            );

                            let result = CheckCliAuthResult {
                                authenticated,
                                email: subscription.clone(), // no email in creds, show subscription
                                auth_method,
                                raw_output: format!("subscription: {}", subscription.unwrap_or_default()),
                            };
                            return Ok(Some(serde_json::to_value(&result).unwrap()));
                        }
                    }
                    // Credentials file not found or unparseable — not authenticated
                    let result = CheckCliAuthResult {
                        authenticated: false,
                        email: None,
                        auth_method: None,
                        raw_output: "no credentials file found".to_string(),
                    };
                    return Ok(Some(serde_json::to_value(&result).unwrap()));
                }

                // Slow path: run CLI auth check command (other providers)
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
                    email = json.get("email")
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

/// Create a Command for a CLI binary, transparently wrapping Windows `.cmd` batch scripts
/// with `cmd.exe /C` so they can be spawned via the Win32 API.
pub(crate) fn make_cli_cmd(cli_path: &str) -> tokio::process::Command {
    #[cfg(windows)]
    if cli_path.ends_with(".cmd") || cli_path.ends_with(".bat") {
        let mut c = tokio::process::Command::new("cmd.exe");
        c.args(["/C", cli_path]);
        return c;
    }
    tokio::process::Command::new(cli_path)
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
