// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{
    CheckCliAuthResult, CommandCheckCliAuthData, CommandResolveCliData, CommandRunCliLoginData,
    ResolveCliResult, RunCliLoginResult, COMMAND_CHECK_CLI_AUTH, COMMAND_RESOLVE_CLI,
};

use super::AppState;

/// Register CLI-related RPC handlers (resolvecli, checkcliauth, runclilogin).
pub fn register_cli_handlers(engine: &Arc<WshRpcEngine>, state: &AppState) {
    // resolvecli → detect or install a CLI tool for an agent provider.
    // Each AgentMux version gets its own isolated CLI install at:
    //   <agentmux_home>/instances/v<AGENTMUX_VERSION>/cli/<provider>/
    // (shared with `install.start` / `install.check` and the frontend
    // launch path; resolved via `DataPaths::from_env()`).
    // Never falls back to system PATH for npm-backed providers.
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

                // Canonical install directory — shared with
                // `install.start` / `install.check` and the frontend's
                // `agent-model.ts::resolveCliDir`. Resolves to
                // `<agentmux_home>/instances/v<version>/cli/<provider>/`
                // via `DataPaths::from_env()` so portable, installed,
                // and `AGENTMUX_HOME_OVERRIDE` modes all agree.
                let paths = agentmux_common::DataPaths::from_env()
                    .ok_or_else(|| "DataPaths::from_env() failed".to_string())?;
                let provider_dir = paths
                    .home_dir
                    .join("instances")
                    .join(format!("v{AGENTMUX_VERSION}"))
                    .join("cli")
                    .join(&cmd.provider_id)
                    .to_string_lossy()
                    .to_string();
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

                // Step 2: Not in versioned dir — check system PATH for non-npm CLIs.
                if cmd.npm_package.is_empty() {
                    if let Some(path) = resolve_cli_on_path(&cmd.cli_command).await {
                        let version = get_cli_version(&path).await;
                        tracing::info!(
                            path = %path, version = %version,
                            "CLI found on system PATH"
                        );
                        return Ok(Some(serde_json::to_value(&ResolveCliResult {
                            cli_path: path,
                            version,
                            source: "system_path".to_string(),
                        }).unwrap()));
                    }
                    // This branch fires for PATH-only providers
                    // (`npm_package` empty) whose CLI isn't on the
                    // system PATH. AgentMux can't auto-install these
                    // — emit AMX-CLI-004 with a manual install hint
                    // instead of AMX-CLI-001 ("Click Install now"),
                    // which would point the user at an install
                    // affordance that doesn't apply.
                    let install_hint = if cfg!(target_os = "windows") {
                        cmd.windows_install_command.clone()
                    } else {
                        cmd.unix_install_command.clone()
                    };
                    return Err(agentmux_common::AgentMuxError::CliMissingOnPath {
                        provider: cmd.provider_id.clone(),
                        cli: cmd.cli_command.clone(),
                        install_hint,
                    }
                    .to_wire()
                    .to_string());
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
                        // CREATE_NO_WINDOW (0x08000000) suppresses cmd flash —
                        // see broader fix in this file's other spawns.
                        let mut probe = tokio::process::Command::new("where");
                        probe.arg("npm");
                        #[cfg(windows)]
                        {
                            use std::os::windows::process::CommandExt;
                            probe.creation_flags(0x08000000);
                        }
                        probe.output().await.map(|o| o.status.success()).unwrap_or(false)
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
                                // CREATE_NO_WINDOW (0x08000000): suppress the
                                // brief cmd.exe console flash that Windows
                                // shows by default when CreateProcess is
                                // called from a GUI process. Without this
                                // flag the user sees a black console
                                // window pop and disappear during npm
                                // install — observed during workspace
                                // setup paths (e.g. tear-off triggering
                                // CLI install on first agent block).
                                const CREATE_NO_WINDOW: u32 = 0x08000000;
                                let npm_cmd_str = format!(
                                    "npm install --loglevel=http --no-audit --no-fund --no-progress --prefix \"{}\" {}",
                                    prefix_dir, package_arg
                                );
                                std::process::Command::new("cmd")
                                    .arg("/C")
                                    .raw_arg(&npm_cmd_str)
                                    .creation_flags(CREATE_NO_WINDOW)
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
                        return Err(agentmux_common::AgentMuxError::NpmInstallFailed {
                            package: format!("{}@{}", cmd.npm_package, cmd.pinned_version),
                            message: format!(
                                "exit {}; check the output above",
                                exit_status.code().unwrap_or(-1)
                            ),
                        }
                        .to_wire()
                        .to_string());
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

                    Err(agentmux_common::AgentMuxError::CliShimMissing {
                        provider: cmd.provider_id.clone(),
                        expected_path: npm_bin.clone(),
                    }
                    .to_wire()
                    .to_string())
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

                // Two-phase auth check for Claude; single-phase for other providers.
                //
                // Phase 1 (fast, <1 ms): read the credentials file to determine whether
                // tokens exist at all. If no file / no tokens → return unauthenticated
                // immediately without spawning the CLI. This avoids a 10+ second cold-start
                // stall when the user is definitely not logged in.
                //
                // Phase 2 (CLI, 10 s timeout): only when tokens ARE present, run
                // `claude auth status --json` to validate them and obtain the real email.
                // This catches expired/revoked tokens — the false-positive that the old
                // file-only fast path missed.
                //
                // Other providers skip Phase 1 and go straight to the CLI (they don't have
                // a predictable credentials file layout).
                //
                // See SPEC_AUTH_CHECK_FALSE_POSITIVE_2026_04_15.md.
                if cmd.cli_path.to_lowercase().contains("claude") {
                    let home = std::env::var("HOME")
                        .or_else(|_| std::env::var("USERPROFILE"))
                        .unwrap_or_default();

                    // REMOVED 2026-08-31 — the first-run bootstrap that copied
                    // the user's global `~/.claude/.credentials.json` into the
                    // isolated CLAUDE_CONFIG_DIR (gated on a
                    // `.agentmux-cred-seeded` sentinel).
                    //
                    // It ran during a routine AUTH CHECK — no login, no user
                    // action, no Armory account — so an agent in a fresh
                    // channel silently acquired the operator's personal
                    // credential. That is the most direct of the four
                    // per-channel-isolation bypasses catalogued in
                    // `docs/analysis/ANALYSIS_PER_CHANNEL_AUTH_BYPASSES_2026_08_31.md`
                    // (#4), and the most likely answer to the reported "agents
                    // operate with an empty Armory" (it also explains why the
                    // symptom varied by machine — it depended on whether
                    // `~/.claude` happened to hold a valid credential).
                    //
                    // Its original justification
                    // (`docs/retro/retro-provider-auth-isolation-regression-2026-06-05.md`)
                    // predates the "agents never use ~/.claude" requirement
                    // (`SPEC_BLOCK_AMBIENT_HOME_DIR_IDENTITY_BINDING_2026_08_25.md`)
                    // and was never revisited against it.
                    //
                    // An unauthenticated isolated dir must now simply report
                    // `authenticated: false` and let the user log in FOR THIS
                    // CHANNEL. Do not reintroduce an import here: a check must
                    // report state, never mint credentials as a side effect.

                    // §4 INVARIANT (provider-auth-isolation.md): validate the SAME dir
                    // the agent runs in — the isolated CLAUDE_CONFIG_DIR if set, else
                    // global ~/.claude. NEVER "isolated OR global": that "check global /
                    // run isolated" split is the validate-spin regression's root cause
                    // (phase 2 below runs `claude auth status --json` against this exact
                    // dir, so phase 1 must check the same one).
                    let creds_path = cmd
                        .auth_env
                        .get("CLAUDE_CONFIG_DIR")
                        .map(|d| format!("{}/.credentials.json", d))
                        .unwrap_or_else(|| format!("{}/.claude/.credentials.json", home));

                    let tokens_exist = match std::fs::read_to_string(&creds_path) {
                        Ok(content) => serde_json::from_str::<serde_json::Value>(&content)
                            .ok()
                            .map(|json| {
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
                                has_token || has_refresh
                            })
                            .unwrap_or(false),
                        Err(_) => false,
                    };

                    // Login/logout robustness diagnostic: snapshot the SAME dir
                    // we validate (token fingerprint is redacted — see
                    // identity::auth_diag). Across login/logout rounds this
                    // surfaces the classic bugs — a check reading a different
                    // dir than login wrote, a token that didn't change after a
                    // re-login, or a credential that outlived a logout. The CLI
                    // verdict is logged separately below; compare the two.
                    // Cover BOTH the isolated dir (CLAUDE_CONFIG_DIR) and the
                    // global-fallback dir (~/.claude) — creds_path/tokens_exist
                    // above use exactly this resolution, so the snapshot always
                    // reflects the dir actually validated (not only the isolated
                    // case). "auth.credstate:" is `muxlog auth` vocabulary.
                    let checked_dir = cmd
                        .auth_env
                        .get("CLAUDE_CONFIG_DIR")
                        .cloned()
                        .unwrap_or_else(|| format!("{home}/.claude"));
                    tracing::info!(
                        "auth.credstate: check {}",
                        crate::identity::auth_diag::snapshot(&checked_dir)
                    );

                    if !tokens_exist {
                        // On macOS the Claude CLI stores credentials in the
                        // Keychain ("Claude Safe Storage"), NOT in
                        // .credentials.json — so a missing file does NOT mean
                        // logged out. Fall through to `claude auth status`, which
                        // reads the Keychain (and does so without a prompt — the
                        // CLI owns that Keychain item). On Windows/Linux the file
                        // IS the credential store, so the fast "definitely not
                        // authenticated" short-circuit stays correct there.
                        #[cfg(not(target_os = "macos"))]
                        {
                            tracing::info!("claude auth check: no credentials in provider dir, skipping CLI");
                            let result = CheckCliAuthResult {
                                authenticated: false,
                                email: None,
                                auth_method: None,
                                raw_output: "no credentials found".to_string(),
                            };
                            return Ok(Some(serde_json::to_value(&result).unwrap()));
                        }
                        #[cfg(target_os = "macos")]
                        tracing::info!(
                            "claude auth check: no credentials file — checking Keychain via CLI (macOS)"
                        );
                    } else {
                        // Tokens exist in the provider dir — validate with the CLI (10 s timeout).
                        tracing::info!("claude auth check: credentials found, validating via CLI");
                    }
                }

                let (mut authenticated, mut email, mut auth_method, mut raw_output) =
                    run_auth_check(&cmd.cli_path, &cmd.auth_check_args, &cmd.auth_env).await?;

                // Self-heal a stale isolated/shared Claude credential (the Pozl
                // 401). Validation failed, but if the user has a valid global
                // ~/.claude login whose access token differs from the checked
                // dir's, the isolated copy went stale (a global re-login rotated
                // the token and killed the isolated refresh token) and the
                // one-time import sentinel blocks auto-reimport. Refresh from
                // global and re-validate ONCE, so the agent recovers without the
                // user hunting for the right button. "auth.credstate:" /
                // "identity.spawn" are `muxlog auth` vocabulary.
                if !authenticated && cmd.cli_path.to_lowercase().contains("claude") {
                    if let Some(config_dir) = cmd.auth_env.get("CLAUDE_CONFIG_DIR") {
                        match refresh_claude_dir_from_global_if_stale(config_dir) {
                            Ok(true) => {
                                tracing::info!(
                                    config_dir = %config_dir,
                                    "auth.credstate: isolated dir failed validation but a newer global login exists — refreshed, re-validating"
                                );
                                match run_auth_check(
                                    &cmd.cli_path,
                                    &cmd.auth_check_args,
                                    &cmd.auth_env,
                                )
                                .await
                                {
                                    Ok((a, e, m, r)) => {
                                        authenticated = a;
                                        email = e;
                                        auth_method = m;
                                        raw_output = r;
                                        tracing::info!(
                                            authenticated,
                                            "auth.credstate: self-heal from global login complete"
                                        );
                                    }
                                    Err(e) => tracing::warn!(
                                        "auth.credstate: self-heal re-validation failed: {e}"
                                    ),
                                }
                            }
                            Ok(false) => {}
                            Err(e) => tracing::warn!(
                                "auth.credstate: self-heal refresh from global failed: {e}"
                            ),
                        }
                    }
                }

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

                // Dead path: the active login flow is the CEF host IPC
                // `run_cli_login`, which owns the child's lifecycle (supersede-kill
                // + timeout reaper). This srv-side variant previously spawned a
                // DETACHED, unkillable `auth login` child here — a process leak if
                // ever invoked. It has no live caller; do NOT spawn.
                let result = RunCliLoginResult { auth_url: None, raw_output: String::new() };
                Ok(Some(serde_json::to_value(&result).unwrap()))
            })
        }),
    );

    // toolchain.env — report the environment the srv resolves tools in: the
    // effective PATH, how it was derived (set by the host/srv PATH enricher,
    // see SPEC_TOOLCHAIN_MANAGER §3), and OS/arch. Powers the Toolchain
    // modal's Environment section so PATH problems are diagnosable.
    engine.register_handler(
        "toolchain.env",
        Box::new(|_data, _ctx| {
            Box::pin(async move {
                let path = std::env::var("PATH").unwrap_or_default();
                let path_source =
                    std::env::var("AGENTMUX_PATH_SOURCE").unwrap_or_else(|_| "inherited".to_string());
                Ok(Some(serde_json::json!({
                    "path": path,
                    "pathSource": path_source,
                    "os": std::env::consts::OS,
                    "arch": std::env::consts::ARCH,
                })))
            })
        }),
    );

    // widget.health — HTTP liveness probe for an external widget server running
    // on localhost. The frontend passes { port, health_check_path,
    // health_check_body_contains? } and gets back { healthy, status_code }.
    // Connection-refused or timeout → { healthy: false } (not an RPC error).
    // health_check_body_contains lets callers distinguish services that share
    // a default port (e.g. Flowise and Grafana both default to 3000).
    engine.register_handler(
        "widget.health",
        Box::new(|data, _ctx| {
            Box::pin(async move {
                let port_raw = data.get("port").and_then(|v| v.as_u64()).unwrap_or(0);
                if port_raw == 0 || port_raw > 65535 {
                    return Ok(Some(serde_json::json!({ "healthy": false, "status_code": null })));
                }
                let port = port_raw as u16;
                let path = data
                    .get("health_check_path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("/")
                    .to_string();
                let body_contains = data
                    .get("health_check_body_contains")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let url = format!("http://127.0.0.1:{}{}", port, path);
                let client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(3))
                    .build()
                    .map_err(|e| e.to_string())?;
                match client.get(&url).send().await {
                    Ok(resp) => {
                        let status = resp.status().as_u16();
                        let ok_status = resp.status().is_success();
                        if !ok_status {
                            return Ok(Some(serde_json::json!({ "healthy": false, "status_code": status })));
                        }
                        // Optionally verify response body for service identity.
                        let healthy = if let Some(needle) = body_contains {
                            let body = resp.text().await.unwrap_or_default();
                            body.contains(&needle)
                        } else {
                            true
                        };
                        Ok(Some(serde_json::json!({ "healthy": healthy, "status_code": status })))
                    }
                    Err(_) => Ok(Some(serde_json::json!({ "healthy": false, "status_code": null }))),
                }
            })
        }),
    );

    // widget.api — HTTP proxy to a widget's local server. Bypasses browser CORS
    // restrictions: the frontend sends { port, path, method?, headers?, body? }
    // and gets back { ok, status_code, body, error? }. Agents use this to call
    // ComfyUI /prompt, Grafana /api/query, etc. without needing a CORS header.
    // 30-second timeout accommodates generative tasks (image synthesis, etc.).
    engine.register_handler(
        "widget.api",
        Box::new(|data, _ctx| {
            Box::pin(async move {
                let port_raw = data.get("port").and_then(|v| v.as_u64()).unwrap_or(0);
                if port_raw == 0 || port_raw > 65535 {
                    return Ok(Some(serde_json::json!({
                        "ok": false, "status_code": null, "body": null,
                        "error": "invalid port"
                    })));
                }
                let port = port_raw as u16;
                let path = data.get("path").and_then(|v| v.as_str()).unwrap_or("/").to_string();
                // Reject paths that could escape localhost: must start with '/',
                // no '@' (user-info injection: 127.0.0.1@evil.com), no backslash,
                // no protocol-relative '//' prefix.
                if !path.starts_with('/') || path.contains('@') || path.contains('\\') || path.starts_with("//") {
                    return Ok(Some(serde_json::json!({
                        "ok": false, "status_code": null, "body": null,
                        "error": "invalid path"
                    })));
                }
                let method = data
                    .get("method")
                    .and_then(|v| v.as_str())
                    .unwrap_or("GET")
                    .to_uppercase();
                let body_str = data.get("body").and_then(|v| v.as_str()).map(|s| s.to_string());
                let headers_obj = data.get("headers").and_then(|v| v.as_object()).cloned();

                let url = format!("http://127.0.0.1:{}{}", port, path);
                let client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(30))
                    .redirect(reqwest::redirect::Policy::none())
                    .build()
                    .map_err(|e| e.to_string())?;

                let mut req = match method.as_str() {
                    "POST" => client.post(&url),
                    "PUT" => client.put(&url),
                    "DELETE" => client.delete(&url),
                    "PATCH" => client.patch(&url),
                    _ => client.get(&url),
                };

                if let Some(headers) = headers_obj {
                    for (k, v) in &headers {
                        if let Some(vs) = v.as_str() {
                            if let (Ok(hn), Ok(hv)) = (
                                reqwest::header::HeaderName::from_bytes(k.as_bytes()),
                                reqwest::header::HeaderValue::from_str(vs),
                            ) {
                                req = req.header(hn, hv);
                            }
                        }
                    }
                }

                if let Some(body) = body_str {
                    req = req
                        .header(reqwest::header::CONTENT_TYPE, "application/json")
                        .body(body);
                }

                match req.send().await {
                    Ok(resp) => {
                        let status = resp.status().as_u16();
                        let body = resp.text().await.unwrap_or_default();
                        Ok(Some(serde_json::json!({ "ok": true, "status_code": status, "body": body })))
                    }
                    Err(e) => Ok(Some(serde_json::json!({
                        "ok": false, "status_code": null, "body": null,
                        "error": e.to_string()
                    }))),
                }
            })
        }),
    );

    // toolchain.versions — fetch the latest published version for a list of npm
    // packages from the npm registry. Input: { packages: [{id, package}] }.
    // Output: { id: "x.y.z" | null, ... }. Each lookup is independent — a
    // network error for one package yields null for that entry, not a failure.
    // 5-second per-request timeout; all lookups run concurrently.
    engine.register_handler(
        "toolchain.versions",
        Box::new(|data, _ctx| {
            Box::pin(async move {
                #[derive(serde::Deserialize)]
                struct Pkg { id: String, package: String }
                let packages: Vec<Pkg> = data
                    .get("packages")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default();

                let client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(5))
                    .user_agent("agentmux/toolchain-versions")
                    .build()
                    .map_err(|e| e.to_string())?;

                let futs: Vec<_> = packages.into_iter().map(|pkg| {
                    let c = client.clone();
                    async move {
                        let url = format!("https://registry.npmjs.org/{}/latest", pkg.package);
                        let version = match c.get(&url).send().await {
                            Ok(resp) if resp.status().is_success() => {
                                resp.json::<serde_json::Value>().await.ok()
                                    .and_then(|v| v.get("version").and_then(|s| s.as_str()).map(|s| s.to_string()))
                            }
                            _ => None,
                        };
                        (pkg.id, version)
                    }
                }).collect();

                let results = futures_util::future::join_all(futs).await;
                let mut out = serde_json::Map::new();
                for (id, version) in results {
                    out.insert(id, version.map(serde_json::Value::String).unwrap_or(serde_json::Value::Null));
                }
                Ok(Some(serde_json::Value::Object(out)))
            })
        }),
    );
}

/// Re-export from shared crate for internal use.
pub(crate) fn make_cli_cmd(cli_path: &str) -> tokio::process::Command {
    agentmux_common::make_cli_cmd(cli_path)
}

/// Run the provider auth-check CLI against `auth_env` and parse the verdict.
/// Returns `(authenticated, email, auth_method, raw_output)`. Extracted so the
/// stale-credential self-heal can re-run the exact same check after refreshing.
async fn run_auth_check(
    cli_path: &str,
    auth_check_args: &[String],
    auth_env: &std::collections::HashMap<String, String>,
) -> Result<(bool, Option<String>, Option<String>, String), String> {
    let output = tokio::time::timeout(std::time::Duration::from_secs(10), {
        let mut check_cmd = make_cli_cmd(cli_path);
        check_cmd.args(auth_check_args);
        for (k, v) in auth_env {
            check_cmd.env(k, v);
        }
        // Null stdin: prevents the CLI from blocking on interactive first-run
        // prompts (onboarding, theme selection) that only appear on a TTY.
        check_cmd.stdin(std::process::Stdio::null());
        check_cmd.output()
    })
    .await
    .map_err(|_| "auth check timed out (10s)".to_string())?
    .map_err(|e| format!("failed to run auth check: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    let mut email = None;
    let mut auth_method = None;
    let authenticated = if let Ok(json) = serde_json::from_str::<serde_json::Value>(&stdout) {
        // Claude outputs `emailAddress`; other CLIs use `email`. Check both.
        email = json
            .get("emailAddress")
            .or_else(|| json.get("email"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        auth_method = json
            .get("authMethod")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        json.get("loggedIn").and_then(|v| v.as_bool()).unwrap_or(false)
    } else {
        output.status.success()
    };
    let raw_output = if !stdout.is_empty() { stdout } else { stderr };
    Ok((authenticated, email, auth_method, raw_output))
}

/// Read the non-empty Claude access token from a credentials file, if any.
fn claude_access_token(creds_path: &str) -> Option<String> {
    let content = std::fs::read_to_string(creds_path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&content).ok()?;
    json.get("claudeAiOauth")
        .and_then(|o| o.get("accessToken"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Refresh a stale Claude isolated/shared config dir from the user's GLOBAL
/// `~/.claude` login. Copies global → dir ONLY when the global credentials file
/// has a non-empty access token that DIFFERS from the dir's current one.
/// Returns `Ok(true)` if it wrote a refreshed credential.
///
/// This is the recovery for the Pozl 401: a global re-login rotates the access
/// token and invalidates the isolated copy's refresh token, but the one-time
/// import sentinel (`.agentmux-cred-seeded`) blocks any auto-reimport — so the
/// isolated dir stays stale and 401s with no self-recovery. Called ONLY after a
/// validation failure, so at worst it copies a global that is itself expired
/// (harmless — the re-validation just fails again).
fn refresh_claude_dir_from_global_if_stale(config_dir: &str) -> std::io::Result<bool> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_default();
    if home.is_empty() {
        return Ok(false);
    }
    let global = format!("{home}/.claude/.credentials.json");
    refresh_dir_from_global(&global, config_dir)
}

/// Path-explicit core of the refresh (see `refresh_claude_dir_from_global_if_stale`).
/// Copies `global_creds` → `<config_dir>/.credentials.json` only when the global
/// file has a non-empty access token that differs from the dir's current one.
fn refresh_dir_from_global(global_creds: &str, config_dir: &str) -> std::io::Result<bool> {
    let iso = format!("{config_dir}/.credentials.json");
    let global_tok = match claude_access_token(global_creds) {
        Some(t) => t,
        None => return Ok(false), // no global login to refresh from
    };
    if claude_access_token(&iso).as_deref() == Some(global_tok.as_str()) {
        return Ok(false); // already identical — nothing to refresh
    }
    std::fs::create_dir_all(config_dir)?;
    std::fs::copy(global_creds, &iso)?;
    Ok(true)
}

/// Resolve a CLI command on the system PATH.
///
/// Uses `where` on Windows and `which` on Unix. Returns the absolute path
/// if the command is found and exists, otherwise `None`.
///
/// On Windows we pass the bare command name (no `.cmd` suffix) so that
/// `where` resolves the correct extension via PATHEXT. This correctly finds
/// `docker.exe`, `git.exe`, `node.exe` AND `npm.cmd` — previously the
/// hard-coded `.cmd` suffix caused all `.exe`-based tools (docker, git,
/// node) to report as not installed even when present on PATH.
///
/// `where` can return multiple lines (e.g. Node.js ships both an extensionless
/// `npm` Unix shell script and `npm.cmd` on Windows). We filter to the first
/// line whose extension `make_cli_cmd` can actually spawn (.exe / .cmd / .bat).
/// Taking the raw first line would yield the extensionless entry, which
/// `Command::new` cannot run on Windows without a shell.
pub(crate) async fn resolve_cli_on_path(cli_command: &str) -> Option<String> {
    let which_result = if cfg!(windows) {
        let mut probe = tokio::process::Command::new("where");
        probe.arg(cli_command);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            probe.creation_flags(0x08000000);
        }
        probe.output().await
    } else {
        tokio::process::Command::new("which").arg(cli_command).output().await
    };
    if let Ok(out) = which_result {
        if out.status.success() {
            let stdout_str = String::from_utf8_lossy(&out.stdout);
            #[cfg(windows)]
            let path: &str = stdout_str
                .lines()
                .map(str::trim)
                .find(|l| {
                    let lo = l.to_lowercase();
                    lo.ends_with(".exe") || lo.ends_with(".cmd") || lo.ends_with(".bat")
                })
                .unwrap_or("");
            #[cfg(not(windows))]
            let path: &str = stdout_str.lines().next().unwrap_or("").trim();
            if !path.is_empty() && std::path::Path::new(path).exists() {
                return Some(path.to_string());
            }
        }
    }
    None
}

async fn get_cli_version(cli_path: &str) -> String {
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        {
            let mut c = make_cli_cmd(cli_path);
            c.arg("--version").stdin(std::process::Stdio::null());
            c.output()
        },
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

#[cfg(test)]
mod selfheal_tests {
    use super::{claude_access_token, refresh_dir_from_global};

    fn write(p: &std::path::Path, tok: &str) {
        std::fs::write(
            p,
            format!(r#"{{"claudeAiOauth":{{"accessToken":"{tok}","refreshToken":"rt"}}}}"#),
        )
        .unwrap();
    }

    #[test]
    fn access_token_parsing() {
        let dir = std::env::temp_dir().join(format!("amx-sh-tok-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("c.json");
        write(&f, "sk-ant-AAA");
        assert_eq!(claude_access_token(f.to_str().unwrap()).as_deref(), Some("sk-ant-AAA"));
        assert_eq!(claude_access_token("/no/such/file").as_deref(), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn refreshes_when_global_token_differs() {
        let base = std::env::temp_dir().join(format!("amx-sh-diff-{}", std::process::id()));
        let gdir = base.join("global");
        let iso = base.join("iso");
        std::fs::create_dir_all(&gdir).unwrap();
        std::fs::create_dir_all(&iso).unwrap();
        let global = gdir.join(".credentials.json");
        write(&global, "sk-ant-NEW");
        write(&iso.join(".credentials.json"), "sk-ant-OLD");

        let did = refresh_dir_from_global(global.to_str().unwrap(), iso.to_str().unwrap()).unwrap();
        assert!(did, "should refresh when tokens differ");
        assert_eq!(
            claude_access_token(iso.join(".credentials.json").to_str().unwrap()).as_deref(),
            Some("sk-ant-NEW"),
            "iso dir should now hold the global token"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn no_op_when_tokens_identical() {
        let base = std::env::temp_dir().join(format!("amx-sh-same-{}", std::process::id()));
        let gdir = base.join("global");
        let iso = base.join("iso");
        std::fs::create_dir_all(&gdir).unwrap();
        std::fs::create_dir_all(&iso).unwrap();
        let global = gdir.join(".credentials.json");
        write(&global, "sk-ant-SAME");
        write(&iso.join(".credentials.json"), "sk-ant-SAME");
        assert!(!refresh_dir_from_global(global.to_str().unwrap(), iso.to_str().unwrap()).unwrap());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn no_op_when_no_global_login() {
        let base = std::env::temp_dir().join(format!("amx-sh-noglobal-{}", std::process::id()));
        let iso = base.join("iso");
        std::fs::create_dir_all(&iso).unwrap();
        // Global path does not exist → nothing to refresh from, iso untouched.
        assert!(!refresh_dir_from_global(
            base.join("global/.credentials.json").to_str().unwrap(),
            iso.to_str().unwrap()
        )
        .unwrap());
        let _ = std::fs::remove_dir_all(&base);
    }
}
