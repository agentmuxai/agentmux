// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `install.start` / `install.cancel` RPC handlers.
//!
//! Phase α of `SPEC_AGENT_INSTALL_STAGE_2026_05_17.md`. Spawns
//! `npm install <package>` for the requested provider, streams every
//! line of stdout+stderr to `install_chunk` WPS events scoped to
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
use serde::Deserialize;
use serde_json::json;

use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::wps::{Broker, WaveEvent};
use crate::server::AppState;

pub const COMMAND_INSTALL_START: &str = "install.start";
pub const COMMAND_INSTALL_CANCEL: &str = "install.cancel";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstallStartReq {
    provider_id: String,
    cli_command: String,
    npm_package: String,
    #[serde(default)]
    pinned_version: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstallCancelReq {
    session_id: String,
}

/// Per-session abort handle so `install.cancel` can kill an in-flight
/// install. `parking_lot::Mutex` since the engine is sync at the
/// handler boundary.
#[derive(Default)]
pub struct InstallSessionRegistry {
    sessions: Mutex<std::collections::HashMap<String, tokio::sync::oneshot::Sender<()>>>,
}

impl InstallSessionRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn insert(&self, session_id: String, tx: tokio::sync::oneshot::Sender<()>) {
        self.sessions.lock().insert(session_id, tx);
    }

    fn cancel(&self, session_id: &str) -> bool {
        if let Some(tx) = self.sessions.lock().remove(session_id) {
            let _ = tx.send(());
            true
        } else {
            false
        }
    }

    fn drop_session(&self, session_id: &str) {
        self.sessions.lock().remove(session_id);
    }
}

pub fn register_install_handlers(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let registry = state.install_sessions.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_INSTALL_START,
        Box::new(move |data, _ctx| {
            let registry = registry.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let req: InstallStartReq = serde_json::from_value(data)
                    .map_err(|e| format!("install.start: {e}"))?;
                if req.npm_package.is_empty() {
                    return Err(format!(
                        "install.start: provider {} has no npm_package — only npm-installable providers are supported in Phase α",
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

                Ok(Some(json!({ "sessionId": session_id })))
            })
        }),
    );

    let registry = state.install_sessions.clone();
    engine.register_handler(
        COMMAND_INSTALL_CANCEL,
        Box::new(move |data, _ctx| {
            let registry = registry.clone();
            Box::pin(async move {
                let req: InstallCancelReq = serde_json::from_value(data)
                    .map_err(|e| format!("install.cancel: {e}"))?;
                let ok = registry.cancel(&req.session_id);
                Ok(Some(json!({
                    "success": ok,
                    "error": if ok { serde_json::Value::Null } else {
                        json!(format!("unknown or already-terminal session: {}", req.session_id))
                    }
                })))
            })
        }),
    );
}

#[allow(clippy::too_many_arguments)]
fn spawn_install_task(
    broker: Arc<Broker>,
    registry: Arc<InstallSessionRegistry>,
    session_id: String,
    provider_id: String,
    _cli_command: String,
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
            let event = WaveEvent {
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
        let emit_done = |broker: &Broker, ok: bool, error: Option<String>| {
            let event = WaveEvent {
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

        // Resolve install dir. Mirrors `cli_handlers.rs::resolve_cli`:
        // `~/.agentmux/<version>/cli/<provider>/`.
        let home = match std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")) {
            Ok(h) => h,
            Err(_) => {
                emit_done(&broker, false, Some("cannot determine home directory".into()));
                registry.drop_session(&session_id);
                return;
            }
        };
        let version = env!("CARGO_PKG_VERSION");
        let provider_dir = format!("{}/.agentmux/{}/cli/{}", home, version, provider_id);
        if let Err(e) = std::fs::create_dir_all(&provider_dir) {
            emit_done(&broker, false, Some(format!("mkdir {provider_dir}: {e}")));
            registry.drop_session(&session_id);
            return;
        }

        let pkg_arg = if pinned_version.is_empty() {
            npm_package.clone()
        } else {
            format!("{}@{}", npm_package, pinned_version)
        };

        emit_line(&broker, format!("$ npm install {} --prefix {}", pkg_arg, provider_dir), "stdout");

        let mut cmd = Command::new(if cfg!(windows) { "npm.cmd" } else { "npm" });
        cmd.args([
            "install",
            &pkg_arg,
            "--prefix",
            &provider_dir,
            "--no-audit",
            "--no-fund",
            "--progress=false",
        ]);
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                emit_done(&broker, false, Some(format!("spawn npm: {e}")));
                registry.drop_session(&session_id);
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
                let event = WaveEvent {
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
                let event = WaveEvent {
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
                    Ok(s) if s.success() => emit_done(&broker, true, None),
                    Ok(s) => emit_done(&broker, false, Some(format!("npm exited {:?}", s.code()))),
                    Err(e) => emit_done(&broker, false, Some(format!("wait: {e}"))),
                }
            }
            _ = &mut cancel_rx => {
                tracing::info!(session_id = %session_id, "install.cancel: killing child");
                let _ = child.kill().await;
                let _ = stdout_task.await;
                let _ = stderr_task.await;
                emit_done(&broker, false, Some("cancelled".into()));
            }
        }

        registry.drop_session(&session_id);
    });
}
