// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Vault` `rotate`: a new value for an entry, set at its service by a command,
//! without anyone seeing it
//! (docs/specs/SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md §4.4).
//!
//! 1. srv generates a new value as a **pending** version that nothing is given yet;
//! 2. runs `set` with it in `NEW_SECRET` and the current value in `OLD_SECRET`;
//! 3. runs `verify`, if given, with it in `NEW_SECRET`;
//! 4. on success makes it the entry's value (the old one kept 7 days); on any
//!    failure keeps the old one and the pending one, marked with the step that
//!    failed: if `set` succeeded and `verify` didn't, the service may already
//!    hold the new value, so nothing is thrown away.
//!
//! The person approves the rotation once, naming `set` and `verify` and saying
//! the value at the service changes; "Always" covers those two command lines for
//! this agent and entry. The commands run like `Terminal` `run` with secrets (in
//! the agent's folder, output cleaned of both values), approved with the rotation.

use std::collections::HashMap;
use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use agentmux_common::api_types::{ShellCreateRequest, UiAutomationAuth};

use crate::backend::vault::uses::{Decision, UseRequest};
use crate::backend::vault::{self, generate, versions, GENERATED_KINDS};

use super::AppState;

/// The activity's action for a rotation.
pub(crate) const ROTATED: &str = "rotated";
/// How long `set` or `verify` may run.
const COMMAND_WAIT: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, Deserialize)]
pub(crate) struct RotateBody {
    #[serde(default)]
    auth: Option<UiAutomationAuth>,
    id: String,
    set: String,
    #[serde(default)]
    verify: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    /// How the new value is made, as for `generate` (20 characters by default).
    #[serde(default)]
    rules: generate::RulesRequest,
}

fn error(code: StatusCode, why: impl Into<String>) -> Response {
    (code, Json(json!({ "error": why.into() }))).into_response()
}

/// The grant "Always" gives: this agent may rotate the entry with these two
/// command lines without asking again.
pub(crate) fn grant_key(agent: &str, set: &str, verify: &str) -> String {
    format!("rotate:{agent}:{set}\n{verify}")
}

/// What the approval prompt shows as the command.
fn shown_commands(set: &str, verify: &str) -> String {
    if verify.is_empty() {
        format!("set: {set}")
    } else {
        format!("set: {set}\nverify: {verify}")
    }
}

/// `POST /api/v1/vault/rotate`.
pub(crate) async fn handle_vault_rotate(State(state): State<AppState>, Json(body): Json<RotateBody>) -> Response {
    let Some(auth) = body.auth.clone() else {
        return error(StatusCode::FORBIDDEN, "rotate needs the agent's signed identity");
    };
    let block_id = match super::ui_handlers::verified_block_id(&state, None, &auth) {
        Ok(b) => b,
        Err(e) => return error(StatusCode::FORBIDDEN, format!("rotate needs the agent's signed identity: {e}")),
    };
    let agent: String = auth.agent_id.chars().take(64).collect();
    let (set, verify) = (body.set.trim().to_string(), body.verify.as_deref().unwrap_or("").trim().to_string());
    if set.is_empty() {
        return error(StatusCode::BAD_REQUEST, "rotate needs set: the command that gives the service the new value (NEW_SECRET)");
    }
    let row = match state.identity_store.vault_get(&body.id) {
        Ok(Some(r)) => r,
        Ok(None) => return error(StatusCode::BAD_REQUEST, format!("no Vault entry {:?}: Vault's list action shows the entries", body.id)),
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };
    if !GENERATED_KINDS.iter().any(|(k, _)| *k == row.kind) {
        return error(StatusCode::BAD_REQUEST, format!("{} is a {}: only an API key, a login's password or a note can be rotated", row.label, row.kind));
    }
    let rules = match generate::rules(&body.rules) {
        Ok(r) => r,
        Err(e) => return error(StatusCode::BAD_REQUEST, e),
    };

    // The person approves the rotation once: both commands, and that the value changes.
    let key = grant_key(&agent, &set, &verify);
    let (store, id) = (state.identity_store.clone(), row.id.clone());
    let k = key.clone();
    let granted = tokio::task::spawn_blocking(move || store.vault_grant_check(&id, &k).unwrap_or(false)).await.unwrap_or(false);
    if !granted {
        let request = UseRequest {
            id: uuid::Uuid::new_v4().to_string(),
            agent: agent.clone(),
            entry_id: row.id.clone(),
            entry_label: row.label.clone(),
            what: "value".to_string(),
            origin: String::new(),
            field: "NEW_SECRET, OLD_SECRET".to_string(),
            offer_always: true,
            reason: "rotating changes this entry's value at the service: the commands give it a new value AgentMux makes".to_string(),
            needs_value: false,
            kind: row.kind.clone(),
            command: shown_commands(&set, &verify),
            cwd: body.cwd.clone().unwrap_or_default(),
            replaces: true,
            requested_ms: agentmux_common::time::now_ms_u64(),
        };
        match super::shell_secrets::ask(&state, &block_id, request, &format!("{agent} wants to rotate {}", row.label)).await {
            Some(Decision::Once) => {}
            Some(Decision::Always) => {
                let (store, id, created) = (state.identity_store.clone(), row.id.clone(), row.created_at);
                let _ = tokio::task::spawn_blocking(move || store.vault_grant_add(&id, &key, agentmux_common::time::now_ms(), created)).await;
            }
            other => {
                let outcome = if other.is_none() { "timed out" } else { "denied" };
                vault::record(&state.identity_store, &row.id, &row.label, &agent, ROTATED, &clip(&set), outcome);
                super::vault_handlers::publish(&state);
                return error(StatusCode::FORBIDDEN, format!("the person didn't allow it ({outcome}): nothing was changed"));
            }
        }
    }

    // 1. The new value, pending.
    let value = match generate::generate(&rules) {
        Ok(v) => v,
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    let (store, id, a) = (state.identity_store.clone(), row.id.clone(), agent.clone());
    let rotation = match tokio::task::spawn_blocking(move || versions::start_rotation(&store, &id, &value, &a)).await {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => return error(StatusCode::CONFLICT, e),
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };
    super::vault_handlers::publish(&state);

    // 2. set, with the pending value and the current one.
    let cwd = body.cwd.as_deref();
    let pending = format!("{}@pending", row.id);
    let set_env = HashMap::from([("NEW_SECRET".to_string(), pending.clone()), ("OLD_SECRET".to_string(), row.id.clone())]);
    let set_ran = run(&state, &block_id, &auth, &agent, &set, cwd, set_env).await;
    // 3. verify, with the pending value.
    let verify_ran = match (&set_ran, verify.is_empty()) {
        (Ok(r), false) if r.ok() => Some(run(&state, &block_id, &auth, &agent, &verify, cwd, HashMap::from([("NEW_SECRET".to_string(), pending)])).await),
        _ => None,
    };
    let failed = match (&set_ran, &verify_ran) {
        (Ok(s), _) if !s.ok() => Some(2),
        (Err(_), _) => Some(2),
        (_, Some(Ok(v))) if !v.ok() => Some(3),
        (_, Some(Err(_))) => Some(3),
        _ => None,
    };

    // 4. Current on success; kept, marked, on failure.
    let (store, id, a) = (state.identity_store.clone(), row.id.clone(), agent.clone());
    let finished = tokio::task::spawn_blocking(move || versions::finish_rotation(&store, &id, &rotation, failed.map_or(Ok(()), Err), &a))
        .await
        .map_err(|e| e.to_string())
        .and_then(|r| r);
    let outcome = match (&failed, &finished) {
        (None, Ok(())) => "done".to_string(),
        (Some(step), _) => format!("failed at step {step}"),
        (None, Err(_)) => "failed: the entry changed meanwhile".to_string(),
    };
    vault::record(&state.identity_store, &row.id, &row.label, &agent, ROTATED, &clip(&set), &outcome);
    super::vault_handlers::publish(&state);

    let rotated = failed.is_none() && finished.is_ok();
    let note = match (failed, &finished) {
        (None, Ok(())) => "Rotated: the entry holds the new value; the old one is kept 7 days (Terminal run can name it as id@previous).".to_string(),
        (Some(2), _) => "set failed, so the entry keeps its old value. The new one is kept 7 days as a version, marked \"rotation failed at step 2\", in case the service took it anyway.".to_string(),
        (Some(_), _) => "verify failed after set succeeded: the service may already hold the new value. The entry keeps its old value, and the new one is kept 7 days as a version, marked \"rotation failed at step 3\"; the person can put it in from the Vault pane.".to_string(),
        (None, Err(e)) => e.clone(),
    };
    Json(json!({
        "id": row.id,
        "label": row.label,
        "rotated": rotated,
        "failed_step": failed,
        "set": ran_json(&set_ran),
        "verify": verify_ran.as_ref().map(ran_json),
        "note": note,
    }))
    .into_response()
}

/// What a command did: its exit code and its last lines, already cleaned of the
/// values it was given.
#[derive(Debug)]
struct Ran {
    exit_code: Option<i32>,
    output: Vec<String>,
}

impl Ran {
    fn ok(&self) -> bool {
        self.exit_code == Some(0)
    }
}

fn ran_json(r: &Result<Ran, String>) -> Value {
    match r {
        Ok(r) => json!({ "exit_code": r.exit_code, "output": r.output }),
        Err(e) => json!({ "error": e }),
    }
}

fn clip(s: &str) -> String {
    let mut out: String = s.chars().take(160).collect();
    if s.chars().count() > 160 {
        out.push('…');
    }
    out
}

/// Runs `cmd` as `Terminal` `run` would with `secrets`, approved with the
/// rotation, and waits for it to end.
async fn run(
    state: &AppState,
    block_id: &str,
    auth: &UiAutomationAuth,
    agent: &str,
    cmd: &str,
    cwd: Option<&str>,
    secrets: HashMap<String, String>,
) -> Result<Ran, String> {
    let req: ShellCreateRequest = serde_json::from_value(json!({
        "agent_block_id": block_id,
        "cmd": cmd,
        "cwd": cwd,
        "title": format!("rotate: {}", clip(cmd)),
        "auth": auth,
        "secrets": &secrets,
    }))
    .map_err(|e| e.to_string())?;
    let approved = super::shell_secrets::Preapproved { env: secrets };
    let resp = super::http_shell::create_shell(state.clone(), req, Some(&approved)).await;
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), 1 << 20).await.map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(v.get("error").and_then(Value::as_str).unwrap_or("the command couldn't start").to_string());
    }
    let shell_id = v.get("shell_id").and_then(Value::as_str).ok_or("the command couldn't start")?.to_string();
    let deadline = tokio::time::Instant::now() + COMMAND_WAIT;
    loop {
        if let Some(s) = state.shell_sessions.get_status_if_known(&shell_id).filter(|s| !s.running) {
            let output = s.tail.map(|t| t.lock().iter().cloned().collect()).unwrap_or_default();
            return Ok(Ran { exit_code: s.exit_code, output });
        }
        if tokio::time::Instant::now() >= deadline {
            state.shell_sessions.stop(&shell_id);
            return Err(format!("{} was still running after {} minutes, so it was stopped", clip(cmd), COMMAND_WAIT.as_secs() / 60));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

#[cfg(test)]
#[path = "vault_rotate_tests.rs"]
mod tests;
