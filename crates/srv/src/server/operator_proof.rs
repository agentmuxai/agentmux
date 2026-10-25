// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! Proof that a websocket connection is the AgentMux window's.
//!
//! Every agent holds the instance auth key, so a connection that never sent
//! `bus:register` proves nothing by itself. Operator-only actions need the
//! host's proof (`RpcContext::operator_only`):
//!
//! 1. the window asks srv for a challenge over its own websocket
//!    (`operator.challenge`) and gets a single-use nonce bound to that
//!    connection;
//! 2. it hands the nonce to the CEF host (`operator_attest`), which posts it to
//!    `POST /api/v1/host/operator_attest` with its IPC token, which agents never
//!    see;
//! 3. srv marks the connection the nonce was issued to as the operator's.
//!
//! A nonce is used once and expires after [`TTL`]. Registering the connection
//! as an agent afterwards drops the proof (`WshRpcEngine::set_rpc_context`).

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, Weak};
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{NoArgsReq, RpcContext};

use super::AppState;

pub const COMMAND_OPERATOR_CHALLENGE: &str = "operator.challenge";

/// How long a challenge waits for the host's attestation.
const TTL: Duration = Duration::from_secs(60);
/// The most challenges waiting at once; past it the oldest is dropped.
const MAX_PENDING: usize = 256;

/// `operator.challenge`'s answer: the nonce to hand to the CEF host.
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct OperatorChallengeResult {
    pub nonce: String,
}

struct Pending {
    engine: Weak<WshRpcEngine>,
    issued: Instant,
}

/// Challenges waiting for the host, by nonce.
#[derive(Default)]
struct Challenges {
    pending: HashMap<String, Pending>,
}

impl Challenges {
    fn issue(&mut self, engine: &Arc<WshRpcEngine>, now: Instant) -> String {
        self.pending.retain(|_, p| now.duration_since(p.issued) < TTL && p.engine.strong_count() > 0);
        while self.pending.len() >= MAX_PENDING {
            let oldest = self.pending.iter().min_by_key(|(_, p)| p.issued).map(|(n, _)| n.clone());
            match oldest {
                Some(n) => self.pending.remove(&n),
                None => break,
            };
        }
        let nonce = uuid::Uuid::new_v4().to_string();
        self.pending.insert(nonce.clone(), Pending { engine: Arc::downgrade(engine), issued: now });
        nonce
    }

    /// The connection `nonce` was issued to, if it is still waiting, unexpired
    /// and open. The nonce is spent either way.
    fn take(&mut self, nonce: &str, now: Instant) -> Option<Arc<WshRpcEngine>> {
        let p = self.pending.remove(nonce)?;
        if now.duration_since(p.issued) >= TTL {
            return None;
        }
        p.engine.upgrade()
    }
}

static CHALLENGES: LazyLock<Mutex<Challenges>> = LazyLock::new(Default::default);

fn challenges() -> std::sync::MutexGuard<'static, Challenges> {
    CHALLENGES.lock().unwrap_or_else(|e| e.into_inner())
}

/// A fresh single-use nonce bound to `engine`'s connection.
pub fn issue(engine: &Arc<WshRpcEngine>) -> String {
    challenges().issue(engine, Instant::now())
}

/// Spend `nonce` and mark the connection it was issued to as the operator's.
/// False when the nonce is unknown, used, expired, its connection closed, or
/// that connection has registered as an agent.
pub fn attest(nonce: &str) -> bool {
    let engine = challenges().take(nonce, Instant::now());
    engine.is_some_and(|e| e.mark_operator_proven())
}

/// Register `operator.challenge` on one connection's engine. The handler holds
/// only a weak reference to the engine that owns it.
pub(super) fn register_challenge(engine: &Arc<WshRpcEngine>) {
    let weak = Arc::downgrade(engine);
    engine.register_typed(COMMAND_OPERATOR_CHALLENGE, move |_req: Option<NoArgsReq>, ctx: RpcContext| {
        let weak = weak.clone();
        async move {
            if !ctx.agent_id.is_empty() {
                return Err("FORBIDDEN: a connection registered as an agent isn't the AgentMux window".to_string());
            }
            let engine = weak.upgrade().ok_or("this connection is closing")?;
            Ok(OperatorChallengeResult { nonce: issue(&engine) })
        }
    });
}

#[derive(Debug, Deserialize)]
pub(crate) struct AttestRequest {
    nonce: String,
}

/// `POST /api/v1/host/operator_attest`: the CEF host vouches, with its own
/// token, for the connection a challenge was issued to. The instance auth
/// key, which every agent has, isn't enough.
pub(crate) async fn handle_host_attest(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<AttestRequest>,
) -> Response {
    if !super::ui_handlers::from_host(&state, &headers).await {
        let error = "only the AgentMux host can prove the operator's window";
        return (StatusCode::FORBIDDEN, Json(json!({ "ok": false, "error": error }))).into_response();
    }
    if attest(&req.nonce) {
        Json(json!({ "ok": true })).into_response()
    } else {
        let error = "that challenge is unknown, used or expired";
        (StatusCode::CONFLICT, Json(json!({ "ok": false, "error": error }))).into_response()
    }
}

#[cfg(test)]
#[path = "operator_proof_tests.rs"]
mod tests;
