// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! A headless server accepts the attestation of a registered external host.
//!
//! The CEF host proves the AgentMux window's connection with its IPC token
//! (`operator_proof`). A headless server reached through a separate host process has no CEF host,
//! and a shared secret would prove nothing, because every agent on the machine can read what srv
//! can. So the external host is a key pair: it keeps the private key, and srv holds only the public
//! key, registered once at boot (`--external-host-key-file`) before any connection or agent exists.
//! There is no route or call that registers or replaces it, so a later or second registration
//! cannot happen.
//!
//! What the host attests is one connection, by the challenge nonce the connection asked srv for
//! (`operator.challenge`), for a short lease, naming the person. The host also gives each
//! connection it proxies an id of its own, signed, in the `x-amx-conn` header; srv binds the
//! challenge to that id, and the signed attestation carries it, so a connection that did not come
//! through the host (one opened on loopback by anything else) can never be attested.
//!
//! Signed text, fixed field order, version first (the signature is Ed25519; fields are checked to
//! hold no `|`):
//!
//! ```text
//! amx-conn/v1|<workspace>|<conn>                                  the connection header
//! amx-attest/v1|<workspace>|<conn>|<nonce>|<person>|<valid_until> an attestation
//! amx-revoke/v1|<workspace>|<conn>|<issued_at>                    ending a connection's proof
//! amx-host/v1|<workspace>|<op>|<issued_at>|<args>                 a call of the host's own (`host_control`)
//! ```

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use ed25519_dalek::{Signature, VerifyingKey};
use serde::Deserialize;
use serde_json::json;

use crate::backend::rpc::engine::WshRpcEngine;

/// The header the host puts on the websocket upgrade of a connection it proxies: `<conn>.<signature>`.
pub const CONN_HEADER: &str = "x-amx-conn";
/// The key file's version.
const KEY_FILE_VERSION: u32 = 1;
/// The longest lease srv grants, whatever the host signed: the host asks for five minutes.
const MAX_LEASE_SECS: u64 = 6 * 60;
/// How old a revocation, or a call of the host's own, may be.
const REVOKE_WINDOW_SECS: u64 = 60;

/// Why a call was refused: the status and a line for the host.
pub(crate) type Refusal = (StatusCode, &'static str);

/// The registered host and the connections it proxied that are open now.
pub(crate) struct Host {
    workspace: String,
    key: VerifyingKey,
    live: Mutex<HashMap<String, Weak<WshRpcEngine>>>,
}

static HOST: OnceLock<Host> = OnceLock::new();

/// The registered external host, if srv was started with one.
pub(crate) fn host() -> Option<&'static Host> {
    HOST.get()
}

/// Register the external host from its key file, once, at boot. A second registration is refused.
pub(crate) fn register_from_file(path: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("--external-host-key-file {}: {e}", path.display()))?;
    set_once(&HOST, Host::from_file_text(&text).map_err(|e| format!("--external-host-key-file {}: {e}", path.display()))?)
}

fn set_once(cell: &OnceLock<Host>, host: Host) -> Result<(), String> {
    cell.set(host).map_err(|_| "an external host is already registered".to_string())
}

fn plain(s: &str, min: usize, max: usize, extra: &str) -> bool {
    (min..=max).contains(&s.len()) && s.chars().all(|c| c.is_ascii_alphanumeric() || extra.contains(c))
}

fn workspace_ok(s: &str) -> bool {
    plain(s, 1, 64, "-")
}

fn conn_ok(s: &str) -> bool {
    plain(s, 16, 64, "_-")
}

fn nonce_ok(s: &str) -> bool {
    s.len() == 36 && s.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

fn person_ok(s: &str) -> bool {
    plain(s, 1, 128, "._@-")
}

#[derive(Debug, Deserialize)]
struct KeyFile {
    v: u32,
    workspace: String,
    public_key: String,
}

impl Host {
    /// A host from its key file's text: `{"v":1,"workspace":"<id>","public_key":"<base64url, 32 bytes>"}`.
    pub(crate) fn from_file_text(text: &str) -> Result<Host, String> {
        let file: KeyFile = serde_json::from_str(text).map_err(|e| format!("not a key file: {e}"))?;
        if file.v != KEY_FILE_VERSION {
            return Err(format!("key file version {} is not supported", file.v));
        }
        if !workspace_ok(&file.workspace) {
            return Err("the workspace id is not valid".to_string());
        }
        let bytes = URL_SAFE_NO_PAD.decode(file.public_key.trim()).map_err(|_| "the public key is not base64url".to_string())?;
        let bytes: [u8; 32] = bytes.try_into().map_err(|_| "the public key is not 32 bytes".to_string())?;
        let key = VerifyingKey::from_bytes(&bytes).map_err(|_| "the public key is not a valid Ed25519 key".to_string())?;
        Ok(Host { workspace: file.workspace, key, live: Mutex::new(HashMap::new()) })
    }

    fn verify(&self, text: &str, signature_b64: &str) -> bool {
        let Ok(bytes) = URL_SAFE_NO_PAD.decode(signature_b64.trim()) else { return false };
        let Ok(bytes): Result<[u8; 64], _> = bytes.try_into() else { return false };
        self.key.verify_strict(text.as_bytes(), &Signature::from_bytes(&bytes)).is_ok()
    }

    /// The connection id in an `x-amx-conn` header value, if the host signed it for this workspace.
    pub(crate) fn conn_from_header(&self, value: &str) -> Option<String> {
        let (id, sig) = value.trim().split_once('.')?;
        if !conn_ok(id) || !self.verify(&format!("amx-conn/v1|{}|{id}", self.workspace), sig) {
            return None;
        }
        Some(id.to_string())
    }

    /// Note that the connection `id` (proxied by the host) is `engine`. False when `id` is already
    /// held by a connection that is still open: the first holder keeps it.
    pub(crate) fn attach(&self, id: &str, engine: &Arc<WshRpcEngine>) -> bool {
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        live.retain(|_, w| w.strong_count() > 0);
        if live.contains_key(id) {
            return false;
        }
        live.insert(id.to_string(), Arc::downgrade(engine));
        true
    }

    /// The connection `id` has closed.
    pub(crate) fn detach(&self, id: &str) {
        self.live.lock().unwrap_or_else(|e| e.into_inner()).remove(id);
    }

    fn engine_of(&self, id: &str) -> Option<Arc<WshRpcEngine>> {
        self.live.lock().unwrap_or_else(|e| e.into_inner()).get(id).and_then(Weak::upgrade)
    }

    /// The host attests the connection `nonce` was issued to. Everything is checked before the nonce is
    /// spent; once it is, it is spent whatever else fails. Returns the person that is now vouched for.
    fn attest(&self, req: &AttestRequest, now: u64) -> Result<String, Refusal> {
        if req.v != 1 {
            return Err((StatusCode::BAD_REQUEST, "unsupported version"));
        }
        if !nonce_ok(&req.nonce) || !conn_ok(&req.conn) || !person_ok(&req.person) || !workspace_ok(&req.workspace) {
            return Err((StatusCode::BAD_REQUEST, "a field is not valid"));
        }
        if req.workspace != self.workspace {
            return Err((StatusCode::FORBIDDEN, "signed for another workspace"));
        }
        let text = format!("amx-attest/v1|{}|{}|{}|{}|{}", req.workspace, req.conn, req.nonce, req.person, req.valid_until);
        if !self.verify(&text, &req.sig) {
            return Err((StatusCode::FORBIDDEN, "the signature is not the registered host's"));
        }
        if req.valid_until <= now || req.valid_until > now + MAX_LEASE_SECS {
            return Err((StatusCode::FORBIDDEN, "the lease is over or too long"));
        }
        let Some(engine) = super::operator_proof::take_engine(&req.nonce) else {
            return Err((StatusCode::CONFLICT, "that challenge is unknown, used or expired"));
        };
        if engine.gateway_conn().as_deref() != Some(req.conn.as_str()) {
            return Err((StatusCode::FORBIDDEN, "that connection did not come through the host"));
        }
        if !engine.mark_operator_proven_for(&req.person, req.valid_until) {
            return Err((StatusCode::CONFLICT, "that connection is registered as an agent"));
        }
        Ok(req.person.clone())
    }

    /// A call of the host's own (`host_control`: what srv is doing, a stop notice, a graceful shutdown) is the host's
    /// and nobody else's: it is signed over its operation and arguments, for this workspace, and no older than a minute.
    /// The instance key is not enough, because every agent can read it.
    pub(crate) fn check_call(&self, op: &str, workspace: &str, issued_at: u64, args: &str, sig: &str, now: u64) -> Result<(), Refusal> {
        if !workspace_ok(workspace) || !plain(op, 1, 32, "-") || args.contains('|') {
            return Err((StatusCode::BAD_REQUEST, "a field is not valid"));
        }
        if workspace != self.workspace {
            return Err((StatusCode::FORBIDDEN, "signed for another workspace"));
        }
        if !self.verify(&format!("amx-host/v1|{workspace}|{op}|{issued_at}|{args}"), sig) {
            return Err((StatusCode::FORBIDDEN, "the signature is not the registered host's"));
        }
        if now.abs_diff(issued_at) > REVOKE_WINDOW_SECS {
            return Err((StatusCode::FORBIDDEN, "the call is too old"));
        }
        Ok(())
    }

    /// The host ends the proof of one connection (the person signed out). Idempotent.
    fn revoke(&self, req: &RevokeRequest, now: u64) -> Result<(), Refusal> {
        if req.v != 1 || !conn_ok(&req.conn) || !workspace_ok(&req.workspace) {
            return Err((StatusCode::BAD_REQUEST, "a field is not valid"));
        }
        if req.workspace != self.workspace {
            return Err((StatusCode::FORBIDDEN, "signed for another workspace"));
        }
        let text = format!("amx-revoke/v1|{}|{}|{}", req.workspace, req.conn, req.issued_at);
        if !self.verify(&text, &req.sig) {
            return Err((StatusCode::FORBIDDEN, "the signature is not the registered host's"));
        }
        if now.abs_diff(req.issued_at) > REVOKE_WINDOW_SECS {
            return Err((StatusCode::FORBIDDEN, "the revocation is too old"));
        }
        if let Some(engine) = self.engine_of(&req.conn) {
            engine.clear_operator_proof();
        }
        Ok(())
    }
}

/// Holds a connection's id while it is open and gives it back when it closes.
pub(crate) struct ConnGuard(String);

impl Drop for ConnGuard {
    fn drop(&mut self) {
        if let Some(host) = host() {
            host.detach(&self.0);
        }
    }
}

/// A websocket upgrade's host-proxied identity: the verified id and the guard that frees it. `None`
/// when no host is registered, the header is missing or not the host's signature, or the id is held.
pub(crate) fn claim_connection(headers: &HeaderMap, engine: &Arc<WshRpcEngine>) -> Option<ConnGuard> {
    let host = host()?;
    let value = headers.get(CONN_HEADER)?.to_str().ok()?;
    let id = host.conn_from_header(value)?;
    if !host.attach(&id, engine) {
        return None;
    }
    engine.set_gateway_conn(id.clone());
    Some(ConnGuard(id))
}

#[derive(Debug, Deserialize)]
pub(crate) struct NonceRequest {
    nonce: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AttestRequest {
    v: u32,
    nonce: String,
    conn: String,
    workspace: String,
    person: String,
    valid_until: u64,
    sig: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RevokeRequest {
    v: u32,
    conn: String,
    workspace: String,
    issued_at: u64,
    sig: String,
}

fn refused((status, error): Refusal) -> Response {
    (status, Json(json!({ "ok": false, "error": error }))).into_response()
}

fn not_registered() -> Response {
    refused((StatusCode::NOT_FOUND, "no external host is registered"))
}

/// `POST /api/v1/host/external_nonce`: which host-proxied connection a challenge was issued to (its id),
/// without spending the challenge. The host asks before it signs; a connection that did not come
/// through the host answers `null`.
pub(crate) async fn handle_nonce(Json(req): Json<NonceRequest>) -> Response {
    let Some(_host) = host() else { return not_registered() };
    if !nonce_ok(&req.nonce) {
        return refused((StatusCode::BAD_REQUEST, "a field is not valid"));
    }
    let conn = super::operator_proof::peek_engine(&req.nonce).and_then(|e| e.gateway_conn());
    Json(json!({ "ok": true, "conn": conn })).into_response()
}

/// `POST /api/v1/host/external_attest`: the registered host vouches for one connection.
pub(crate) async fn handle_attest(Json(req): Json<AttestRequest>) -> Response {
    let Some(host) = host() else { return not_registered() };
    match host.attest(&req, agentmux_common::time::now_secs_u64()) {
        Ok(person) => {
            tracing::info!(person = %person, conn = %req.conn, "external host attested a connection");
            Json(json!({ "ok": true, "valid_until": req.valid_until })).into_response()
        }
        Err(r) => refused(r),
    }
}

/// `POST /api/v1/host/external_revoke`: the registered host ends one connection's proof.
pub(crate) async fn handle_revoke(Json(req): Json<RevokeRequest>) -> Response {
    let Some(host) = host() else { return not_registered() };
    match host.revoke(&req, agentmux_common::time::now_secs_u64()) {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(r) => refused(r),
    }
}

#[cfg(test)]
#[path = "external_host_tests.rs"]
mod tests;
