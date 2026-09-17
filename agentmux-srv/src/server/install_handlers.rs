// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `install.start` / `install.cancel` RPC handlers.
//!
//! Phase α of `SPEC_AGENT_INSTALL_STAGE_2026_05_17.md`. Spawns
//! `npm install <package>` for the requested provider, streams every
//! line of stdout+stderr to `install_chunk` MPS events scoped to
//! `install:<sessionId>`, and emits a terminal `{ op: "done", ok,
//! error? }` event when the child exits (or the user cancels).
//!
//! Phase α scope:
//!  - npm-only install (existing per-version layout at
//!    `~/.agentmux/<version>/cli/<provider>/`).
//!  - Plain piped stdio (no PTY) — npm doesn't isatty-gate its output
//!    line-by-line, so pipes are fine here. PTY would be required for
//!    interactive post-install steps (Phase δ).
//!  - Single in-flight install per session id. The frontend modal owns
//!    the session id and prevents parallel installs at the UI layer.
//!  - Cancel kills the child via `kill_on_drop` when the abort handle
//!    fires. The partial `node_modules` dir is rm-rf'd best-effort.
//!  - No verify / doctor / post-install steps yet — those land in
//!    Phase β.

use std::sync::Arc;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::mps::{Broker, MuxEvent};
use crate::server::AppState;

pub const COMMAND_INSTALL_START: &str = "install.start";
pub const COMMAND_INSTALL_CANCEL: &str = "install.cancel";
pub const COMMAND_INSTALL_CHECK: &str = "install.check";
pub const COMMAND_RESOLVE_PREREQS: &str = "resolve.prereqs";

#[derive(Debug, Deserialize, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct InstallStartReq {
    pub provider_id: String,
    pub cli_command: String,
    pub npm_package: String,
    /// `#[serde(default)]`, so it is omittable on the wire — but the
    /// hand-written stub declared it REQUIRED, and the generated type keeps it
    /// required (ts-rs cannot express an optional non-`Option` property
    /// anyway). Callers are stricter than the server here, which is the safe
    /// direction; noting it so the asymmetry is deliberate rather than lost.
    #[serde(default)]
    pub pinned_version: String,
}

#[derive(Debug, Deserialize, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct InstallCancelReq {
    pub session_id: String,
}

#[derive(Debug, Deserialize, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct InstallCheckReq {
    pub provider_id: String,
    pub cli_command: String,
}

/// Request for `resolve.prereqs`. Was a function-local anonymous struct.
#[derive(Debug, Deserialize, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct ResolvePrereqsReq {
    pub tools: Vec<String>,
}

/// Result of `install.start`. Was an inline `json!({ "sessionId": .. })`.
#[derive(Debug, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct InstallStartResult {
    pub session_id: String,
}

/// Result of `install.check`.
#[derive(Debug, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct InstallCheckResult {
    pub installed: bool,
}

/// Result of `install.cancel`.
///
/// `error` is genuinely `string | null`, NOT an optional property: the handler
/// writes an explicit `Value::Null` on success, so the key is always present.
/// The hand-written stub declared it `error?: string`, which said the key could
/// be absent — it never is. Generating the type corrects that.
#[derive(Debug, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
#[serde(rename_all = "camelCase")]
pub struct InstallCancelResult {
    pub success: bool,
    pub error: Option<String>,
}

/// One tool's resolution in `resolve.prereqs`.
#[derive(Debug, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct PrereqToolResolution {
    pub tool: String,
    pub found: bool,
    /// Always present, null when the tool was not found.
    pub path: Option<String>,
}

/// Result of `resolve.prereqs`.
#[derive(Debug, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct ResolvePrereqsResult {
    pub results: Vec<PrereqToolResolution>,
}

/// Per-session abort handle so `install.cancel` can kill an in-flight
/// install. Also tracks `active_providers` so concurrent sessions for
/// the *same* provider directory are rejected — without that, cancel
/// of one would `rm_rf` the shared dir mid-install for the other.
/// `parking_lot::Mutex` since the engine is sync at the handler
/// boundary.
#[derive(Default)]
pub struct InstallSessionRegistry {
    sessions: Mutex<std::collections::HashMap<String, tokio::sync::oneshot::Sender<()>>>,
    active_providers: Mutex<std::collections::HashSet<String>>,
    /// Same idea as `active_providers`, but a separate set keyed by
    /// system-tool id (`"git"`, `"node"`, …) rather than provider id — a
    /// distinct namespace so a future provider literally named e.g. "git"
    /// can never collide with a system-tool claim. See
    /// `system_install_handlers.rs` / `SPEC_SYSTEM_TOOLCHAIN_INSTALLER_2026_08_24.md`.
    active_system_tools: Mutex<std::collections::HashSet<String>>,
}

impl InstallSessionRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub(crate) fn insert(&self, session_id: String, tx: tokio::sync::oneshot::Sender<()>) {
        self.sessions.lock().insert(session_id, tx);
    }

    /// Try to claim a provider; returns false if another session is
    /// already installing this provider.
    fn try_claim_provider(&self, provider_id: &str) -> bool {
        self.active_providers.lock().insert(provider_id.to_string())
    }

    fn release_provider(&self, provider_id: &str) {
        self.active_providers.lock().remove(provider_id);
    }

    /// Try to claim a system tool (git/node/npm/python); returns false if
    /// another session is already installing this tool.
    pub(crate) fn try_claim_system_tool(&self, tool_id: &str) -> bool {
        self.active_system_tools.lock().insert(tool_id.to_string())
    }

    pub(crate) fn release_system_tool(&self, tool_id: &str) {
        self.active_system_tools.lock().remove(tool_id);
    }

    fn cancel(&self, session_id: &str) -> bool {
        if let Some(tx) = self.sessions.lock().remove(session_id) {
            let _ = tx.send(());
            true
        } else {
            false
        }
    }

    pub(crate) fn drop_session(&self, session_id: &str) {
        self.sessions.lock().remove(session_id);
    }
}

/// Provider ids feed into the install dir path; reject anything that
/// could escape `~/.agentmux/<version>/cli/<provider>/`.
fn is_safe_provider_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// CLI command names are joined into the bin-resolution path; same
/// allowlist as provider ids, plus `.` (some real CLIs include dots,
/// e.g. `eslint.cmd`).
fn is_safe_cli_command(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
        && !s.contains("..")
}

/// Canonical install directory for a provider —
/// `<agentmux_home>/instances/v<version>/cli/<provider>/`. This is the
/// same path the frontend uses to launch the agent
/// (`agent-model.ts::resolveCliDir`), so the bin we drop in
/// `node_modules/.bin/` is what the launch path will execute.
/// Honors portable / installed mode + the `AGENTMUX_HOME_OVERRIDE`
/// test override via `DataPaths::from_env()`.
fn provider_install_dir(provider_id: &str) -> Option<std::path::PathBuf> {
    let paths = agentmux_common::DataPaths::from_env()?;
    let version = env!("CARGO_PKG_VERSION");
    Some(
        paths
            .home_dir
            .join("instances")
            .join(format!("v{version}"))
            .join("cli")
            .join(provider_id),
    )
}

/// Returns the path to the installed CLI binary if present in the
/// per-version cache, else None. Used by `install.check`.
/// Locate a system tool (e.g. `git`, `gh`) on PATH. Uses the platform
/// equivalent of `which` and returns the resolved absolute path. None
/// when the tool isn't on PATH or the lookup itself failed.
///
/// Used by `resolve.prereqs` to pre-launch-check whether a provider's
/// system dependencies are installed. The probe is path-only — never
/// executes the tool — so it's safe to call without side effects.
/// See SPEC_PROVIDER_SYSTEM_PREREQS_2026_05_18.md.
pub(crate) async fn resolve_tool_path(tool: &str) -> Option<String> {
    let cmd = if cfg!(windows) { "where" } else { "which" };
    let mut c = tokio::process::Command::new(cmd);
    c.arg(tool);
    // CREATE_NO_WINDOW: `where` is console-subsystem; without this it flashes
    // a console window on every pre-launch prereq check. See cli.rs's note.
    #[cfg(windows)]
    {
        use agentmux_common::win32::CREATE_NO_WINDOW;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    let output = c.output().await.ok()?;
    if !output.status.success() {
        return None;
    }
    // `where` on Windows can return multiple lines; first is canonical.
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.lines().next().map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn resolve_installed_bin(provider_id: &str, cli_command: &str) -> Option<std::path::PathBuf> {
    let dir = provider_install_dir(provider_id)?;
    let bin_dir = dir.join("node_modules").join(".bin");
    let candidates: &[&str] = if cfg!(windows) {
        &[".cmd", ".exe", ""]
    } else {
        &["", ".cmd"]
    };
    for suffix in candidates {
        let p = bin_dir.join(format!("{cli_command}{suffix}"));
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

pub fn register_install_handlers(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let registry = state.install_sessions.clone();
    let broker = state.broker.clone();
    engine.register_typed(
        COMMAND_INSTALL_START,
        move |req: InstallStartReq, _ctx| {
            let registry = registry.clone();
            let broker = broker.clone();
            async move {
                if !is_safe_provider_id(&req.provider_id) {
                    return Err(format!(
                        "install.start: invalid provider id {:?} — must match [a-zA-Z0-9_-]+",
                        req.provider_id
                    ));
                }
                if !is_safe_cli_command(&req.cli_command) {
                    return Err(format!(
                        "install.start: invalid cli command {:?}",
                        req.cli_command
                    ));
                }
                if req.npm_package.is_empty() {
                    return Err(format!(
                        "install.start: provider {} has no npm_package — only npm-installable providers are supported in Phase α",
                        req.provider_id
                    ));
                }
                if !registry.try_claim_provider(&req.provider_id) {
                    return Err(format!(
                        "install.start: provider {} is already being installed in another session",
                        req.provider_id
                    ));
                }
                let session_id = format!("install-{}", uuid::Uuid::new_v4());
                tracing::info!(
                    session_id = %session_id,
                    provider_id = %req.provider_id,
                    npm_package = %req.npm_package,
                    pinned_version = %req.pinned_version,
                    "install.start"
                );

                let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
                registry.insert(session_id.clone(), cancel_tx);

                spawn_install_task(
                    broker,
                    registry,
                    session_id.clone(),
                    req.provider_id,
                    req.cli_command,
                    req.npm_package,
                    req.pinned_version,
                    cancel_rx,
                );

                Ok(InstallStartResult { session_id })
            }
        },
    );

    engine.register_typed(
        COMMAND_INSTALL_CHECK,
        move |req: InstallCheckReq, _ctx| {
            async move {
                if !is_safe_provider_id(&req.provider_id) {
                    return Err(format!(
                        "install.check: invalid provider id {:?}",
                        req.provider_id
                    ));
                }
                if !is_safe_cli_command(&req.cli_command) {
                    return Err(format!(
                        "install.check: invalid cli command {:?}",
                        req.cli_command
                    ));
                }
                let installed = resolve_installed_bin(&req.provider_id, &req.cli_command).is_some();
                Ok(InstallCheckResult { installed })
            }
        },
    );

    engine.register_typed(
        COMMAND_RESOLVE_PREREQS,
        move |req: ResolvePrereqsReq, _ctx| async move {
                let mut results = Vec::with_capacity(req.tools.len());
                for tool in &req.tools {
                    if !is_safe_cli_command(tool) {
                        return Err(format!(
                            "resolve.prereqs: invalid tool name {:?}",
                            tool
                        ));
                    }
                    let path = resolve_tool_path(tool).await;
                    results.push(PrereqToolResolution {
                        tool: tool.clone(),
                        found: path.is_some(),
                        path,
                    });
                }
                Ok(ResolvePrereqsResult { results })
        },
    );

    let registry = state.install_sessions.clone();
    engine.register_typed(
        COMMAND_INSTALL_CANCEL,
        move |req: InstallCancelReq, _ctx| {
            let registry = registry.clone();
            async move {
                let ok = registry.cancel(&req.session_id);
                Ok(InstallCancelResult {
                    success: ok,
                    error: if ok {
                        None
                    } else {
                        Some(format!("unknown or already-terminal session: {}", req.session_id))
                    },
                })
            }
        },
    );
}

#[allow(clippy::too_many_arguments)]
fn spawn_install_task(
    broker: Arc<Broker>,
    registry: Arc<InstallSessionRegistry>,
    session_id: String,
    provider_id: String,
    cli_command: String,
    npm_package: String,
    pinned_version: String,
    mut cancel_rx: tokio::sync::oneshot::Receiver<()>,
) {
    tokio::spawn(async move {
        use std::process::Stdio;
        use tokio::io::{AsyncBufReadExt, BufReader};
        use tokio::process::Command;

        let scope = format!("install:{}", session_id);
        let emit_line = |broker: &Broker, line: String, stream: &'static str| {
            let event = MuxEvent {
                event: "install_chunk".to_string(),
                scopes: vec![scope.clone()],
                sender: String::new(),
                persist: 1024,
                data: Some(json!({
                    "sessionId": session_id,
                    "line": line,
                    "stream": stream,
                })),
            };
            broker.publish(event);
        };
        // Legacy emit path — the `error` field is a free-text string.
        // New code paths should use `emit_done_typed` below to emit
        // the wire-format `AgentMuxError` object so the frontend can
        // render a friendly `<ErrorBanner />`.
        let emit_done = |broker: &Broker, ok: bool, error: Option<String>| {
            let event = MuxEvent {
                event: "install_chunk".to_string(),
                scopes: vec![scope.clone()],
                sender: String::new(),
                persist: 1024,
                data: Some(json!({
                    "sessionId": session_id,
                    "op": "done",
                    "ok": ok,
                    "error": error,
                })),
            };
            broker.publish(event);
        };
        let emit_done_typed = |broker: &Broker, err: agentmux_common::AgentMuxError| {
            let event = MuxEvent {
                event: "install_chunk".to_string(),
                scopes: vec![scope.clone()],
                sender: String::new(),
                persist: 1024,
                data: Some(json!({
                    "sessionId": session_id,
                    "op": "done",
                    "ok": false,
                    "error": err.to_wire(),
                })),
            };
            broker.publish(event);
        };

        let provider_dir = match provider_install_dir(&provider_id) {
            Some(p) => p,
            None => {
                emit_done(&broker, false, Some("cannot determine home directory".into()));
                registry.drop_session(&session_id);
                registry.release_provider(&provider_id);
                return;
            }
        };
        if let Err(e) = std::fs::create_dir_all(&provider_dir) {
            // The disk-full / permission-denied / path-not-found cases
            // route to the typed catalog so the frontend renders a
            // friendly "Device out of space" message instead of the
            // raw OS error. Other IO kinds fall through to Legacy.
            let err = agentmux_common::AgentMuxError::from_io_with_path(
                provider_dir.display().to_string(),
                e,
            );
            emit_done_typed(&broker, err);
            registry.drop_session(&session_id);
            registry.release_provider(&provider_id);
            return;
        }
        let provider_dir_str = provider_dir.to_string_lossy().to_string();

        let pkg_arg = if pinned_version.is_empty() {
            npm_package.clone()
        } else {
            format!("{}@{}", npm_package, pinned_version)
        };

        // `--progress=false` is unconditional: npm only renders the
        // progress bar when both stdout and stderr are TTYs, and this
        // task pipes both, so leaving progress at the default would
        // produce no visible spinner anyway. `--loglevel=verbose` is
        // also unconditional: the user's only signal of progress
        // during long installs is the per-package fetch/extract
        // chatter, so we always pay for the noise to gain the signal.
        let npm_args: Vec<String> = vec![
            "install".to_string(),
            pkg_arg.clone(),
            "--prefix".to_string(),
            provider_dir_str.clone(),
            "--no-audit".to_string(),
            "--no-fund".to_string(),
            "--progress=false".to_string(),
            "--loglevel=verbose".to_string(),
        ];

        emit_line(
            &broker,
            format!("$ npm {}", npm_args.join(" ")),
            "stdout",
        );

        let mut cmd = Command::new(if cfg!(windows) { "npm.cmd" } else { "npm" });
        // `npm install` runs arbitrary postinstall scripts — the textbook case for
        // not handing a process this instance's identity or API endpoint.
        crate::backend::pane_env::sanitize_external_command(&mut cmd);
        cmd.args(&npm_args);
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(agentmux_common::win32::CREATE_NO_WINDOW); // CREATE_NO_WINDOW
        }

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                emit_done(&broker, false, Some(format!("spawn npm: {e}")));
                registry.drop_session(&session_id);
                registry.release_provider(&provider_id);
                return;
            }
        };

        let stdout = child.stdout.take().expect("piped");
        let stderr = child.stderr.take().expect("piped");

        let broker_out = broker.clone();
        let session_out = session_id.clone();
        let scope_out = scope.clone();
        let stdout_task = tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let event = MuxEvent {
                    event: "install_chunk".to_string(),
                    scopes: vec![scope_out.clone()],
                    sender: String::new(),
                    persist: 1024,
                    data: Some(json!({
                        "sessionId": session_out,
                        "line": line,
                        "stream": "stdout",
                    })),
                };
                broker_out.publish(event);
            }
        });

        let broker_err = broker.clone();
        let session_err = session_id.clone();
        let scope_err = scope.clone();
        let stderr_task = tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let event = MuxEvent {
                    event: "install_chunk".to_string(),
                    scopes: vec![scope_err.clone()],
                    sender: String::new(),
                    persist: 1024,
                    data: Some(json!({
                        "sessionId": session_err,
                        "line": line,
                        "stream": "stderr",
                    })),
                };
                broker_err.publish(event);
            }
        });

        tokio::select! {
            wait = child.wait() => {
                let _ = stdout_task.await;
                let _ = stderr_task.await;
                match wait {
                    Ok(s) if s.success() => {
                        // npm exit 0 ≠ "binary is on disk". Verify the
                        // expected bin shim exists so a package/bin
                        // rename or provider-config mismatch surfaces
                        // as an install failure rather than a phantom
                        // launch with a non-existent cmd path.
                        if resolve_installed_bin(&provider_id, &cli_command).is_some() {
                            emit_done(&broker, true, None);
                        } else {
                            emit_done(
                                &broker,
                                false,
                                Some(format!(
                                    "npm install reported success but {} not found in {}/node_modules/.bin/",
                                    cli_command,
                                    provider_dir.display()
                                )),
                            );
                        }
                    }
                    Ok(s) => emit_done(&broker, false, Some(format!("npm exited {:?}", s.code()))),
                    Err(e) => emit_done(&broker, false, Some(format!("wait: {e}"))),
                }
            }
            _ = &mut cancel_rx => {
                tracing::info!(session_id = %session_id, "install.cancel: killing child");
                let _ = child.kill().await;
                let _ = stdout_task.await;
                let _ = stderr_task.await;
                // Wipe the partial install so a retry doesn't inherit
                // a half-written package-lock.json. Best-effort —
                // logging only on failure.
                if let Err(e) = std::fs::remove_dir_all(&provider_dir) {
                    // Best-effort cleanup; tag the log with the typed
                    // code so grouped support requests can grep by
                    // `amx_code` rather than free-text matching.
                    let mux = agentmux_common::AgentMuxError::from_io_with_path(
                        provider_dir.display().to_string(),
                        e,
                    );
                    tracing::warn!(
                        target: "amx::error",
                        session_id = %session_id,
                        amx_code = %mux.code(),
                        provider_dir = %provider_dir.display(),
                        error = %mux,
                        "install.cancel: remove partial dir failed"
                    );
                }
                emit_done(&broker, false, Some("cancelled".into()));
            }
        }

        registry.drop_session(&session_id);
        registry.release_provider(&provider_id);
    });
}

#[cfg(test)]
mod tests {
    use super::{is_safe_cli_command, is_safe_provider_id};

    #[test]
    fn safe_provider_ids_accepted() {
        for id in ["claude", "claude-code", "open_claw", "Codex42"] {
            assert!(is_safe_provider_id(id), "{id} should be accepted");
        }
    }

    #[test]
    fn unsafe_provider_ids_rejected() {
        for id in [
            "",
            "../escape",
            "a/b",
            "a\\b",
            "a b",
            ".",
            "..",
            "a..b",
            "a/../b",
            "\0null",
            &"x".repeat(65),
        ] {
            assert!(!is_safe_provider_id(id), "{id:?} should be rejected");
        }
    }

    #[test]
    fn safe_cli_commands_accepted() {
        for cmd in ["claude", "claude-code", "kimi.cmd", "agentmux-srv", "open_claw"] {
            assert!(is_safe_cli_command(cmd), "{cmd} should be accepted");
        }
    }

    #[test]
    fn unsafe_cli_commands_rejected() {
        for cmd in [
            "",
            "../etc/passwd",
            "../../etc/passwd",
            "a/b",
            "a\\b",
            "a b",
            "..",
            "a..b",
            "a/../b",
            "\0null",
            &"x".repeat(65),
        ] {
            assert!(!is_safe_cli_command(cmd), "{cmd:?} should be rejected");
        }
    }
}

// Request/result shape tests for the four install-related commands.
#[cfg(test)]
mod req_shape_tests {
    use super::*;
    use serde_json::json;

    // These structs rely on rename_all = "camelCase" to match the wire. Drop
    // that attribute and everything still compiles and the binding still
    // generates -- it just silently stops parsing real payloads.
    #[test]
    fn the_requests_accept_the_camel_case_payloads_the_stub_sends() {
        let start: InstallStartReq = serde_json::from_value(json!({
            "providerId": "claude",
            "cliCommand": "claude",
            "npmPackage": "@anthropic-ai/claude-code",
            "pinnedVersion": "1.2.3",
        }))
        .expect("install.start");
        assert_eq!(start.pinned_version, "1.2.3");

        // pinnedVersion is #[serde(default)], so the server accepts it missing
        // even though the generated binding marks it required -- callers are
        // stricter than the wire, which is the safe direction.
        let no_version: InstallStartReq = serde_json::from_value(json!({
            "providerId": "claude",
            "cliCommand": "claude",
            "npmPackage": "p",
        }))
        .expect("pinnedVersion is optional on the wire");
        assert_eq!(no_version.pinned_version, "");

        serde_json::from_value::<InstallCancelReq>(json!({"sessionId": "s1"}))
            .expect("install.cancel");
        serde_json::from_value::<InstallCheckReq>(
            json!({"providerId": "claude", "cliCommand": "claude"}),
        )
        .expect("install.check");
        serde_json::from_value::<ResolvePrereqsReq>(json!({"tools": ["node", "git"]}))
            .expect("resolve.prereqs");

        assert!(
            serde_json::from_value::<InstallCancelReq>(json!({"session_id": "s1"})).is_err(),
            "snake_case is not the wire format for these"
        );
    }

    // THE DRIFT THIS SLICE FIXES. The handler writes an explicit null for
    // `error` on success, so the key is ALWAYS present. The hand-written stub
    // said `error?: string`, i.e. the key may be absent -- it never is.
    #[test]
    fn cancel_result_always_carries_the_error_key() {
        let ok = serde_json::to_value(InstallCancelResult { success: true, error: None })
            .expect("serializable");
        assert_eq!(ok, json!({"success": true, "error": null}));
        assert!(
            ok.as_object().unwrap().contains_key("error"),
            "error is `string | null`, not an optional property"
        );
    }

    #[test]
    fn prereq_results_carry_a_nullable_path() {
        let v = serde_json::to_value(ResolvePrereqsResult {
            results: vec![PrereqToolResolution {
                tool: "node".to_string(),
                found: false,
                path: None,
            }],
        })
        .expect("serializable");
        assert_eq!(v, json!({"results": [{"tool": "node", "found": false, "path": null}]}));
    }
}
