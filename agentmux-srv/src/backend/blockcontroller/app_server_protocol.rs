// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Codex App Server's stable thread/turn adapter.
//!
//! The transport in [`super::app_server`] owns framing and request lifecycle.
//! This module owns the first protocol-semantic layer: typed request shapes,
//! thread identity, turn identity, and notification reconciliation. It is
//! intentionally not registered as a production block controller yet.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};
use thiserror::Error;

use super::app_server::{
    AppServerError, AppServerIncoming, AppServerTransport, RpcId, ServerResponseError,
};

const MAX_RAW_EVENTS: usize = 128;
const MAX_PENDING_SERVER_REQUESTS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionPhase {
    Idle,
    Running,
    Completed,
    /// The turn ended via `TurnStatus::interrupted` (user cancellation) —
    /// deliberately distinct from `Failed`, which was collapsing an
    /// expected cancellation with a genuine provider/execution failure
    /// (ReAgent P1, PR #3210). Consumers care about the difference: a
    /// cancelled turn isn't an error to surface the same way.
    Interrupted,
    Failed,
}

impl Default for SessionPhase {
    fn default() -> Self {
        Self::Idle
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ItemSnapshot {
    pub id: String,
    pub kind: String,
    /// Preview text accumulated from delta notifications. This is cleared
    /// when the authoritative item completion is observed.
    pub preview: String,
    pub completed: bool,
    pub authoritative: Option<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionSnapshot {
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
    pub phase: SessionPhase,
    pub items: HashMap<String, ItemSnapshot>,
    pub raw_unknown_events: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CodexAppServerEvent {
    ThreadStarted {
        thread_id: String,
    },
    TurnStarted {
        thread_id: String,
        turn_id: String,
    },
    AgentMessageDelta {
        thread_id: String,
        turn_id: String,
        item_id: String,
        delta: String,
        preview: String,
    },
    ItemStarted {
        thread_id: String,
        turn_id: String,
        item_id: String,
        kind: String,
    },
    ItemCompleted {
        thread_id: String,
        turn_id: String,
        item_id: String,
        kind: String,
        item: Value,
    },
    TurnCompleted {
        thread_id: String,
        turn_id: String,
        status: String,
    },
    ThreadStatusChanged {
        thread_id: String,
        status: Value,
    },
    UnknownNotification {
        method: String,
        params: Value,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerRequestKind {
    CommandApproval,
    FileChangeApproval,
    PermissionsApproval,
    UserInput,
    McpElicitation,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PendingServerRequest {
    pub id: RpcId,
    pub kind: ServerRequestKind,
    pub method: String,
    pub params: Value,
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
    pub item_id: Option<String>,
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum CodexAppServerProtocolError {
    #[error(transparent)]
    Transport(#[from] AppServerError),
    #[error("App Server response is missing a string field: {0}")]
    MissingStringField(&'static str),
    #[error("App Server response is missing an object field: {0}")]
    MissingObjectField(&'static str),
    #[error("App Server event is missing a string field: {0}")]
    MissingEventField(&'static str),
    #[error("App Server event belongs to thread {actual}, but active thread is {expected}")]
    ThreadScopeMismatch { expected: String, actual: String },
    #[error("App Server event belongs to turn {actual}, but active turn is {expected}")]
    TurnScopeMismatch { expected: String, actual: String },
    #[error("cannot start a turn before a thread is loaded")]
    ThreadNotLoaded,
    #[error("cannot steer or interrupt without an active turn")]
    TurnNotActive,
    #[error("cannot start a second turn while turn {0} is active")]
    TurnAlreadyActive(String),
    #[error("resume response did not identify the requested thread")]
    ResumeMissingThread,
    #[error("unsupported or unsafe App Server request method: {0}")]
    UnsupportedServerRequest(String),
    #[error("no pending App Server request matches id")]
    UnknownServerRequest,
    #[error("App Server request id is already pending")]
    DuplicateServerRequest,
    #[error("too many App Server requests are pending a decision (max {0})")]
    TooManyPendingRequests(usize),
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ThreadStartOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub developer_instructions: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ThreadResumeParams {
    thread_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cwd: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TurnStartParams {
    thread_id: String,
    input: Vec<TextUserInput>,
}

#[derive(Debug, Clone, Serialize)]
struct TextUserInput {
    #[serde(rename = "type")]
    kind: &'static str,
    text: String,
}

pub struct CodexAppServerSession {
    transport: Arc<AppServerTransport>,
    state: Mutex<SessionState>,
}

#[derive(Default)]
struct SessionState {
    thread_id: Option<String>,
    turn_id: Option<String>,
    phase: SessionPhase,
    items: HashMap<String, ItemSnapshot>,
    raw_unknown_events: VecDeque<Value>,
    /// The most recently `turn/completed`-notified turn id. `start_turn`
    /// checks this after its own `turn/start` response resolves — the
    /// response and the completion notification arrive on independent
    /// channels (the request/response oneshot vs. run_reader's
    /// notification stream), so a `turn/completed` for the SAME turn can
    /// be applied before `start_turn`'s continuation resumes. Without this
    /// check, `start_turn`'s unconditional post-await write would resurrect
    /// an already-terminal turn as `Running`, permanently stuck (ReAgent
    /// P1, PR #3210). One id is enough, not a growing set: only one turn is
    /// ever active at a time, so at most one termination can be "pending
    /// recognition" by a not-yet-resumed `start_turn` call.
    last_terminated_turn_id: Option<String>,
    /// Reserved atomically under the same lock as the `turn_id.is_some()`
    /// check, before the `turn/start` request is even sent. Without this,
    /// two concurrent `start_turn` calls both observe `turn_id == None`
    /// and both release the lock before either gets a response/notification
    /// back — nothing was reserved under the lock, so both send `turn/start`
    /// (ReAgent P1, PR #3210). Cleared once the response (or an error)
    /// resolves.
    turn_starting: bool,
    pending_requests: HashMap<RpcId, PendingServerRequest>,
}

impl CodexAppServerSession {
    pub fn new(transport: Arc<AppServerTransport>) -> Self {
        Self {
            transport,
            state: Mutex::new(SessionState {
                phase: SessionPhase::Idle,
                ..SessionState::default()
            }),
        }
    }

    pub async fn start_thread(
        &self,
        options: ThreadStartOptions,
        timeout: Duration,
    ) -> Result<String, CodexAppServerProtocolError> {
        let params = serde_json::to_value(options)
            .map_err(|error| AppServerError::Protocol(error.to_string()))?;
        let result = self
            .transport
            .request("thread/start", params, timeout)
            .await?;
        let thread_id = extract_thread_id(&result)?;
        let mut state = self.state.lock().unwrap();
        state.thread_id = Some(thread_id.clone());
        state.turn_id = None;
        state.phase = SessionPhase::Idle;
        state.items.clear();
        // A pending approval queued under a prior thread/turn must not
        // remain answerable once that context is gone (ReAgent P1, PR
        // #3212) -- only turn/completed cleared this before.
        state.pending_requests.clear();
        Ok(thread_id)
    }

    pub async fn resume_thread(
        &self,
        thread_id: String,
        model: Option<String>,
        cwd: Option<String>,
        timeout: Duration,
    ) -> Result<String, CodexAppServerProtocolError> {
        let requested_id = thread_id.clone();
        let result = self
            .transport
            .request(
                "thread/resume",
                serde_json::to_value(ThreadResumeParams {
                    thread_id,
                    model,
                    cwd,
                })
                .map_err(|error| AppServerError::Protocol(error.to_string()))?,
                timeout,
            )
            .await?;
        let resumed_id = extract_thread_id(&result)?;
        if resumed_id != requested_id {
            return Err(CodexAppServerProtocolError::ResumeMissingThread);
        }
        // The 0.154.0 contract can resume a thread that has a turn already
        // `inProgress` (ThreadResumeResponse's `thread.turns`, populated on
        // resume specifically) — unconditionally clearing turn_id/phase
        // here silently dropped that live turn state instead of deriving
        // it from what the response actually reports (ReAgent P1, PR #3210).
        let in_progress_turn_id = extract_in_progress_turn_id(&result);
        let mut state = self.state.lock().unwrap();
        state.thread_id = Some(resumed_id.clone());
        state.turn_id = in_progress_turn_id.clone();
        state.phase = if in_progress_turn_id.is_some() {
            SessionPhase::Running
        } else {
            SessionPhase::Idle
        };
        state.items.clear();
        // Same as start_thread: a pending approval from a prior thread/turn
        // must not survive into this one (ReAgent P1, PR #3212).
        state.pending_requests.clear();
        Ok(resumed_id)
    }

    pub async fn start_turn(
        &self,
        text: String,
        timeout: Duration,
    ) -> Result<String, CodexAppServerProtocolError> {
        let thread_id = {
            let mut state = self.state.lock().unwrap();
            if let Some(turn_id) = &state.turn_id {
                return Err(CodexAppServerProtocolError::TurnAlreadyActive(
                    turn_id.clone(),
                ));
            }
            if state.turn_starting {
                // A different start_turn call already reserved the slot and
                // hasn't gotten its response/error back yet — reject this
                // one the same way an already-active turn would be
                // rejected, rather than also sending a second turn/start.
                return Err(CodexAppServerProtocolError::TurnAlreadyActive(
                    "(turn start already in flight)".to_string(),
                ));
            }
            let thread_id = state
                .thread_id
                .clone()
                .ok_or(CodexAppServerProtocolError::ThreadNotLoaded)?;
            // Reserved under the SAME lock as the checks above, before the
            // request is even sent — this is what makes the check-then-act
            // atomic. Without it, two concurrent callers could both observe
            // turn_id == None and turn_starting == false before either
            // released the lock, and both would send turn/start (ReAgent
            // P1, PR #3210).
            state.turn_starting = true;
            thread_id
        };

        // Scoped so the fallible network round-trip is entirely separate
        // from the state updates before/after it.
        let turn_id_result: Result<String, CodexAppServerProtocolError> = async {
            let result = self
                .transport
                .request(
                    "turn/start",
                    serde_json::to_value(TurnStartParams {
                        thread_id,
                        input: vec![TextUserInput { kind: "text", text }],
                    })
                    .map_err(|error| AppServerError::Protocol(error.to_string()))?,
                    timeout,
                )
                .await?;
            let turn = result
                .get("turn")
                .and_then(Value::as_object)
                .ok_or(CodexAppServerProtocolError::MissingObjectField("turn"))?;
            string_field(turn.get("id"), "turn.id")
        }
        .await;

        let mut state = self.state.lock().unwrap();
        state.turn_starting = false;
        let turn_id = turn_id_result?;
        if state.last_terminated_turn_id.as_deref() == Some(turn_id.as_str()) {
            // A `turn/completed` notification for THIS turn already landed
            // (on the independent notification channel) while we were
            // awaiting this same turn's `turn/start` response. The turn is
            // already terminal — don't resurrect it as Running.
            return Ok(turn_id);
        }
        state.turn_id = Some(turn_id.clone());
        state.phase = SessionPhase::Running;
        Ok(turn_id)
    }

    /// Add input to the active turn only when the caller supplies the exact
    /// turn precondition. Ordinary queued messages should not call this API.
    pub async fn steer_turn(
        &self,
        text: String,
        expected_turn_id: String,
        timeout: Duration,
    ) -> Result<Value, CodexAppServerProtocolError> {
        let (thread_id, active_turn_id) = self.active_turn()?;
        if active_turn_id != expected_turn_id {
            return Err(CodexAppServerProtocolError::TurnScopeMismatch {
                expected: active_turn_id,
                actual: expected_turn_id,
            });
        }
        Ok(self
            .transport
            .request(
                "turn/steer",
                json!({
                    "threadId": thread_id,
                    "expectedTurnId": expected_turn_id,
                    "input": [{"type": "text", "text": text}],
                }),
                timeout,
            )
            .await?)
    }

    /// Interrupt the active turn. The turn remains active locally until its
    /// authoritative `turn/completed` notification arrives.
    pub async fn interrupt_turn(
        &self,
        timeout: Duration,
    ) -> Result<Value, CodexAppServerProtocolError> {
        let (thread_id, turn_id) = self.active_turn()?;
        Ok(self
            .transport
            .request(
                "turn/interrupt",
                json!({"threadId": thread_id, "turnId": turn_id}),
                timeout,
            )
            .await?)
    }

    fn active_turn(&self) -> Result<(String, String), CodexAppServerProtocolError> {
        let state = self.state.lock().unwrap();
        let thread_id = state
            .thread_id
            .clone()
            .ok_or(CodexAppServerProtocolError::ThreadNotLoaded)?;
        let turn_id = state
            .turn_id
            .clone()
            .ok_or(CodexAppServerProtocolError::TurnNotActive)?;
        Ok((thread_id, turn_id))
    }

    /// Validate and queue a server request for an explicit UI/policy decision.
    /// No request is approved implicitly.
    pub fn accept_server_request(
        &self,
        incoming: AppServerIncoming,
    ) -> Result<PendingServerRequest, CodexAppServerProtocolError> {
        let AppServerIncoming::Request { id, method, params } = incoming else {
            return Err(CodexAppServerProtocolError::UnsupportedServerRequest(
                "notifications cannot be queued as requests".to_string(),
            ));
        };
        let kind = match method.as_str() {
            "item/commandExecution/requestApproval" => ServerRequestKind::CommandApproval,
            "item/fileChange/requestApproval" => ServerRequestKind::FileChangeApproval,
            "item/permissions/requestApproval" => ServerRequestKind::PermissionsApproval,
            "item/tool/requestUserInput" => ServerRequestKind::UserInput,
            "mcpServer/elicitation/request" => ServerRequestKind::McpElicitation,
            _ => {
                return Err(CodexAppServerProtocolError::UnsupportedServerRequest(
                    method,
                ))
            }
        };
        // Every kind except McpElicitation is item-scoped and MUST carry a
        // threadId/turnId — silently treating an omitted field the same as
        // "no scope claimed" (rather than erroring) contradicted this API's
        // own "scoped to active thread/turn" fail-closed claim: a
        // malformed/unexpected request missing these fields would have been
        // queued unconditionally instead of rejected (ReAgent P1, PR #3212).
        // McpElicitation genuinely has no thread/turn context (see its own
        // test fixture), so it alone stays optional.
        let (thread_id, turn_id) = if kind == ServerRequestKind::McpElicitation {
            (
                params.get("threadId").and_then(Value::as_str).map(str::to_string),
                params.get("turnId").and_then(Value::as_str).map(str::to_string),
            )
        } else {
            (
                Some(string_field(params.get("threadId"), "threadId")?),
                Some(string_field(params.get("turnId"), "turnId")?),
            )
        };
        let item_id = params
            .get("itemId")
            .and_then(Value::as_str)
            .map(str::to_string);
        let mut state = self.state.lock().unwrap();
        if state.pending_requests.contains_key(&id) {
            return Err(CodexAppServerProtocolError::DuplicateServerRequest);
        }
        if state.pending_requests.len() >= MAX_PENDING_SERVER_REQUESTS {
            return Err(CodexAppServerProtocolError::TooManyPendingRequests(
                MAX_PENDING_SERVER_REQUESTS,
            ));
        }
        // Strict, not the notification-reconciliation ensure_thread_scope/
        // ensure_scopes: those trivially pass when state.thread_id/turn_id
        // is None, which is correct for a notification establishing state
        // (e.g. the first thread/started) but wrong here — a request
        // claiming a threadId/turnId while no thread/turn is actually
        // active must be rejected, not queued as if it were in scope
        // (ReAgent P1, PR #3212 3rd review). Matched on both fields
        // together, not two independent `if let Some`s: a request with
        // turnId but no threadId is a real, distinct case (McpElicitation
        // may carry either field alone) and must still validate the turn
        // it claims — two independent checks silently skip it whenever
        // thread_id is None, whatever turn_id says (ReAgent P1, PR #3212
        // 4th review).
        match (&thread_id, &turn_id) {
            (Some(thread), Some(turn)) => ensure_active_scopes(&state, thread, turn)?,
            (Some(thread), None) => ensure_active_thread_scope(&state, thread)?,
            (None, Some(turn)) => ensure_active_turn_scope(&state, turn)?,
            (None, None) => {}
        }
        let pending = PendingServerRequest {
            id: id.clone(),
            kind,
            method,
            params,
            thread_id,
            turn_id,
            item_id,
        };
        state.pending_requests.insert(id, pending.clone());
        Ok(pending)
    }

    /// Send a decision for a queued request. The request remains pending if the
    /// transport write fails, allowing a retry; successful responses consume it.
    pub async fn respond_server_request(
        &self,
        id: &RpcId,
        result: Result<Value, ServerResponseError>,
        timeout: Duration,
    ) -> Result<(), CodexAppServerProtocolError> {
        // Removed atomically (check-and-take in one lock acquisition)
        // rather than checked-then-removed-later — the old
        // check-then-await-then-remove sequence let two concurrent calls
        // for the same id both pass the membership check before either
        // removed it, so both would send a JSON-RPC response for the same
        // id and both report success (ReAgent P2, PR #3212). Re-inserted
        // below on a transport failure so the documented retry-on-failure
        // behavior is preserved — only one caller can ever be "holding" the
        // entry between the take and a possible reinsert.
        let pending = self
            .state
            .lock()
            .unwrap()
            .pending_requests
            .remove(id)
            .ok_or(CodexAppServerProtocolError::UnknownServerRequest)?;

        // RestoreOnDrop reinserts the removed entry (if still scoped to the
        // active thread/turn) whenever it is dropped still armed — which
        // covers an explicit transport-write failure below AND this
        // future being dropped/cancelled while still suspended at the
        // `.await` (e.g. a caller-side select!/timeout racing the write).
        // A plain check-then-await-then-reinsert-on-Err only runs that
        // reinsert code if the future resolves; drop the future mid-await
        // instead and Rust never runs it at all, silently stranding the
        // entry with no response ever sent and no error surfaced to
        // anyone (ReAgent P1, PR #3212 5th review). Folding the ordinary
        // failure path into the same Drop impl means both cases are
        // handled by one piece of logic instead of two copies of it.
        struct RestoreOnDrop<'a> {
            state: &'a Mutex<SessionState>,
            id: RpcId,
            pending: Option<PendingServerRequest>,
        }
        impl Drop for RestoreOnDrop<'_> {
            fn drop(&mut self) {
                let Some(pending) = self.pending.take() else {
                    return;
                };
                let mut state = self.state.lock().unwrap();
                // Matched on both fields together, not `(None, _) =>
                // true`: a pending entry with turn_id but no thread_id is
                // still turn-scoped and must not be resurrected once that
                // turn has ended, whatever thread_id says (ReAgent P1,
                // PR #3212 4th review).
                let still_current = match (&pending.thread_id, &pending.turn_id) {
                    (Some(thread), Some(turn)) => {
                        state.thread_id.as_deref() == Some(thread.as_str())
                            && state.turn_id.as_deref() == Some(turn.as_str())
                    }
                    (Some(thread), None) => state.thread_id.as_deref() == Some(thread.as_str()),
                    (None, Some(turn)) => state.turn_id.as_deref() == Some(turn.as_str()),
                    (None, None) => true,
                };
                if still_current {
                    state.pending_requests.insert(self.id.clone(), pending);
                }
            }
        }

        let mut guard = RestoreOnDrop {
            state: &self.state,
            id: id.clone(),
            pending: Some(pending),
        };
        let outcome = self.transport.respond(id.clone(), result, timeout).await;
        if outcome.is_ok() {
            // Consumed successfully — nothing left to restore.
            guard.pending = None;
        }
        outcome.map_err(Into::into)
    }

    pub fn pending_server_requests(&self) -> Vec<PendingServerRequest> {
        self.state
            .lock()
            .unwrap()
            .pending_requests
            .values()
            .cloned()
            .collect()
    }

    pub fn snapshot(&self) -> SessionSnapshot {
        let state = self.state.lock().unwrap();
        SessionSnapshot {
            thread_id: state.thread_id.clone(),
            turn_id: state.turn_id.clone(),
            phase: state.phase,
            items: state.items.clone(),
            raw_unknown_events: state.raw_unknown_events.iter().cloned().collect(),
        }
    }

    pub fn apply_incoming(
        &self,
        incoming: AppServerIncoming,
    ) -> Result<CodexAppServerEvent, CodexAppServerProtocolError> {
        let AppServerIncoming::Notification { method, params } = incoming else {
            return Err(CodexAppServerProtocolError::Transport(
                AppServerError::Protocol(
                    "server request requires an explicit approval adapter".to_string(),
                ),
            ));
        };
        let mut state = self.state.lock().unwrap();
        match method.as_str() {
            "thread/started" => {
                let thread = params
                    .get("thread")
                    .and_then(Value::as_object)
                    .ok_or(CodexAppServerProtocolError::MissingObjectField("thread"))?;
                let thread_id = string_field(thread.get("id"), "thread.id")?;
                // Only reset turn state for a GENUINELY new thread. If this
                // notification is for the thread we already have loaded, a
                // turn may already be active on it (e.g. this notification
                // was delayed and arrives after `start_turn` already ran) —
                // resetting unconditionally would wipe that live state out
                // from under it (ReAgent P1, PR #3210).
                let is_new_thread = state.thread_id.as_deref() != Some(thread_id.as_str());
                state.thread_id = Some(thread_id.clone());
                if is_new_thread {
                    state.turn_id = None;
                    state.phase = SessionPhase::Idle;
                    state.items.clear();
                    // Same as start_thread/resume_thread: a pending
                    // approval from the previous thread must not remain
                    // answerable once it's been replaced (ReAgent P1, PR
                    // #3212).
                    state.pending_requests.clear();
                }
                Ok(CodexAppServerEvent::ThreadStarted { thread_id })
            }
            "turn/started" => {
                let thread_id = string_field(params.get("threadId"), "threadId")?;
                ensure_thread_scope(&state, &thread_id)?;
                let turn = params
                    .get("turn")
                    .and_then(Value::as_object)
                    .ok_or(CodexAppServerProtocolError::MissingObjectField("turn"))?;
                let turn_id = string_field(turn.get("id"), "turn.id")?;
                state.turn_id = Some(turn_id.clone());
                state.phase = SessionPhase::Running;
                Ok(CodexAppServerEvent::TurnStarted { thread_id, turn_id })
            }
            "item/agentMessage/delta" => {
                let thread_id = string_field(params.get("threadId"), "threadId")?;
                let turn_id = string_field(params.get("turnId"), "turnId")?;
                ensure_scopes(&state, &thread_id, &turn_id)?;
                let item_id = string_field(params.get("itemId"), "itemId")?;
                let delta = string_field(params.get("delta"), "delta")?;
                let item = state.items.entry(item_id.clone()).or_insert(ItemSnapshot {
                    id: item_id.clone(),
                    kind: "agentMessage".to_string(),
                    preview: String::new(),
                    completed: false,
                    authoritative: None,
                });
                item.preview.push_str(&delta);
                Ok(CodexAppServerEvent::AgentMessageDelta {
                    thread_id,
                    turn_id,
                    item_id,
                    delta,
                    preview: item.preview.clone(),
                })
            }
            "item/started" => {
                let thread_id = string_field(params.get("threadId"), "threadId")?;
                let turn_id = string_field(params.get("turnId"), "turnId")?;
                ensure_scopes(&state, &thread_id, &turn_id)?;
                let item = params
                    .get("item")
                    .and_then(Value::as_object)
                    .ok_or(CodexAppServerProtocolError::MissingObjectField("item"))?;
                let item_id = string_field(item.get("id"), "item.id")?;
                let kind = string_field(item.get("type"), "item.type")?;
                state.items.insert(
                    item_id.clone(),
                    ItemSnapshot {
                        id: item_id.clone(),
                        kind: kind.clone(),
                        preview: String::new(),
                        completed: false,
                        authoritative: None,
                    },
                );
                Ok(CodexAppServerEvent::ItemStarted {
                    thread_id,
                    turn_id,
                    item_id,
                    kind,
                })
            }
            "item/completed" => {
                let thread_id = string_field(params.get("threadId"), "threadId")?;
                let turn_id = string_field(params.get("turnId"), "turnId")?;
                ensure_scopes(&state, &thread_id, &turn_id)?;
                let item = params
                    .get("item")
                    .and_then(Value::as_object)
                    .ok_or(CodexAppServerProtocolError::MissingObjectField("item"))?;
                let item_id = string_field(item.get("id"), "item.id")?;
                let kind = string_field(item.get("type"), "item.type")?;
                state.items.insert(
                    item_id.clone(),
                    ItemSnapshot {
                        id: item_id.clone(),
                        kind: kind.clone(),
                        preview: String::new(),
                        completed: true,
                        authoritative: Some(Value::Object(item.clone())),
                    },
                );
                Ok(CodexAppServerEvent::ItemCompleted {
                    thread_id,
                    turn_id,
                    item_id,
                    kind,
                    item: Value::Object(item.clone()),
                })
            }
            "turn/completed" => {
                let thread_id = string_field(params.get("threadId"), "threadId")?;
                ensure_thread_scope(&state, &thread_id)?;
                let turn = params
                    .get("turn")
                    .and_then(Value::as_object)
                    .ok_or(CodexAppServerProtocolError::MissingObjectField("turn"))?;
                let turn_id = string_field(turn.get("id"), "turn.id")?;
                if let Some(expected) = &state.turn_id {
                    if expected != &turn_id {
                        return Err(CodexAppServerProtocolError::TurnScopeMismatch {
                            expected: expected.clone(),
                            actual: turn_id.clone(),
                        });
                    }
                }
                let status = string_field(turn.get("status"), "turn.status")?;
                state.turn_id = None;
                state.last_terminated_turn_id = Some(turn_id.clone());
                // TurnStatus's terminal values: "completed", "interrupted"
                // (user cancellation), "failed". Collapsing "interrupted"
                // into Failed hid an expected cancellation behind the same
                // signal as a genuine provider/execution failure (ReAgent
                // P1, PR #3210). Anything else unrecognized still maps to
                // Failed rather than erroring — this notification has
                // already been accepted; the phase is best-effort.
                state.phase = match status.as_str() {
                    "completed" => SessionPhase::Completed,
                    "interrupted" => SessionPhase::Interrupted,
                    _ => SessionPhase::Failed,
                };
                state.pending_requests.clear();
                Ok(CodexAppServerEvent::TurnCompleted {
                    thread_id,
                    turn_id,
                    status,
                })
            }
            "thread/status/changed" => {
                let thread_id = string_field(params.get("threadId"), "threadId")?;
                ensure_thread_scope(&state, &thread_id)?;
                let status = params.get("status").cloned().unwrap_or(Value::Null);
                Ok(CodexAppServerEvent::ThreadStatusChanged { thread_id, status })
            }
            _ => {
                let raw = json!({ "method": method, "params": params });
                if state.raw_unknown_events.len() == MAX_RAW_EVENTS {
                    state.raw_unknown_events.pop_front();
                }
                state.raw_unknown_events.push_back(raw.clone());
                Ok(CodexAppServerEvent::UnknownNotification {
                    method: raw["method"].as_str().unwrap_or_default().to_string(),
                    params: raw["params"].clone(),
                })
            }
        }
    }
}

fn extract_thread_id(result: &Value) -> Result<String, CodexAppServerProtocolError> {
    let thread = result
        .get("thread")
        .and_then(Value::as_object)
        .ok_or(CodexAppServerProtocolError::MissingObjectField("thread"))?;
    string_field(thread.get("id"), "thread.id")
}

/// The id of whichever turn in `thread.turns` (a `ThreadResumeResponse`
/// field, populated on `thread/resume`) has `status: "inProgress"`, if any.
/// `TurnStatus`'s only other values (`completed`/`interrupted`/`failed`)
/// are all terminal, so at most one entry can ever match. Malformed/missing
/// shapes intentionally fall through to `None` rather than erroring — this
/// is best-effort derivation of pre-existing state, not a required field.
fn extract_in_progress_turn_id(result: &Value) -> Option<String> {
    result
        .get("thread")?
        .get("turns")?
        .as_array()?
        .iter()
        .find(|turn| turn.get("status").and_then(Value::as_str) == Some("inProgress"))?
        .get("id")?
        .as_str()
        .map(str::to_string)
}

fn string_field(
    value: Option<&Value>,
    field: &'static str,
) -> Result<String, CodexAppServerProtocolError> {
    value
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or(CodexAppServerProtocolError::MissingStringField(field))
}

fn ensure_thread_scope(
    state: &SessionState,
    actual: &str,
) -> Result<(), CodexAppServerProtocolError> {
    if let Some(expected) = &state.thread_id {
        if expected != actual {
            return Err(CodexAppServerProtocolError::ThreadScopeMismatch {
                expected: expected.clone(),
                actual: actual.to_string(),
            });
        }
    }
    Ok(())
}

fn ensure_scopes(
    state: &SessionState,
    thread_id: &str,
    turn_id: &str,
) -> Result<(), CodexAppServerProtocolError> {
    ensure_thread_scope(state, thread_id)?;
    if let Some(expected) = &state.turn_id {
        if expected != turn_id {
            return Err(CodexAppServerProtocolError::TurnScopeMismatch {
                expected: expected.clone(),
                actual: turn_id.to_string(),
            });
        }
    }
    Ok(())
}

/// Unlike ensure_thread_scope, fails closed when no thread is active — a
/// server request claiming a threadId must be scoped to a *real, active*
/// thread, not merely "no conflicting thread claimed yet" the way an
/// incoming notification establishing state is allowed to be.
fn ensure_active_thread_scope(
    state: &SessionState,
    actual: &str,
) -> Result<(), CodexAppServerProtocolError> {
    match &state.thread_id {
        Some(expected) if expected == actual => Ok(()),
        Some(expected) => Err(CodexAppServerProtocolError::ThreadScopeMismatch {
            expected: expected.clone(),
            actual: actual.to_string(),
        }),
        None => Err(CodexAppServerProtocolError::ThreadNotLoaded),
    }
}

/// See ensure_active_thread_scope — fails closed when no turn is active.
fn ensure_active_scopes(
    state: &SessionState,
    thread_id: &str,
    turn_id: &str,
) -> Result<(), CodexAppServerProtocolError> {
    ensure_active_thread_scope(state, thread_id)?;
    ensure_active_turn_scope(state, turn_id)
}

/// See ensure_active_thread_scope — fails closed when no turn is active.
/// Split out from ensure_active_scopes so a turn can be validated on its
/// own when a request carries a turnId without a threadId (ReAgent P1,
/// PR #3212 4th review).
fn ensure_active_turn_scope(
    state: &SessionState,
    actual: &str,
) -> Result<(), CodexAppServerProtocolError> {
    match &state.turn_id {
        Some(expected) if expected == actual => Ok(()),
        Some(expected) => Err(CodexAppServerProtocolError::TurnScopeMismatch {
            expected: expected.clone(),
            actual: actual.to_string(),
        }),
        None => Err(CodexAppServerProtocolError::TurnNotActive),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{duplex, AsyncBufReadExt, AsyncWriteExt, BufReader};

    fn session() -> (
        Arc<CodexAppServerSession>,
        tokio::io::ReadHalf<tokio::io::DuplexStream>,
        tokio::io::WriteHalf<tokio::io::DuplexStream>,
    ) {
        let (client, server) = duplex(8192);
        let (client_reader, client_writer) = tokio::io::split(client);
        let (server_reader, server_writer) = tokio::io::split(server);
        let transport = Arc::new(AppServerTransport::new(
            client_reader,
            client_writer,
            Default::default(),
        ));
        (
            Arc::new(CodexAppServerSession::new(transport)),
            server_reader,
            server_writer,
        )
    }

    #[tokio::test]
    async fn starts_thread_and_turn_with_schema_shaped_requests() {
        let (session, server_reader, mut server_writer) = session();
        let server = tokio::spawn(async move {
            let mut lines = BufReader::new(server_reader).lines();
            let start: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(start["method"], "thread/start");
            assert_eq!(start["params"]["model"], "gpt-5-codex");
            server_writer
                .write_all(
                    br#"{"id":1,"result":{"thread":{"id":"thread-1"}}}
"#,
                )
                .await
                .unwrap();
            let turn: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(turn["method"], "turn/start");
            assert_eq!(turn["params"]["threadId"], "thread-1");
            assert_eq!(
                turn["params"]["input"][0],
                json!({"type":"text","text":"hello"})
            );
            server_writer
                .write_all(
                    br#"{"id":2,"result":{"turn":{"id":"turn-1"}}}
"#,
                )
                .await
                .unwrap();
        });

        assert_eq!(
            session
                .start_thread(
                    ThreadStartOptions {
                        model: Some("gpt-5-codex".to_string()),
                        ..Default::default()
                    },
                    Duration::from_secs(1),
                )
                .await
                .unwrap(),
            "thread-1"
        );
        assert_eq!(
            session
                .start_turn("hello".to_string(), Duration::from_secs(1))
                .await
                .unwrap(),
            "turn-1"
        );
        assert_eq!(session.snapshot().phase, SessionPhase::Running);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn resumes_only_the_requested_thread() {
        let (session, server_reader, mut server_writer) = session();
        let server = tokio::spawn(async move {
            let mut lines = BufReader::new(server_reader).lines();
            let request: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(request["method"], "thread/resume");
            assert_eq!(request["params"]["threadId"], "thread-2");
            server_writer
                .write_all(
                    br#"{"id":1,"result":{"thread":{"id":"thread-2"}}}
"#,
                )
                .await
                .unwrap();
        });
        assert_eq!(
            session
                .resume_thread(
                    "thread-2".to_string(),
                    None,
                    Some("C:\\workspace".to_string()),
                    Duration::from_secs(1),
                )
                .await
                .unwrap(),
            "thread-2"
        );
        assert_eq!(session.snapshot().thread_id.as_deref(), Some("thread-2"));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn steers_and_interrupts_only_the_active_turn() {
        let (session, server_reader, mut server_writer) = session();
        {
            let mut state = session.state.lock().unwrap();
            state.thread_id = Some("thread-1".to_string());
            state.turn_id = Some("turn-1".to_string());
        }
        let server = tokio::spawn(async move {
            let mut lines = BufReader::new(server_reader).lines();
            let steer: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(steer["method"], "turn/steer");
            assert_eq!(steer["params"]["expectedTurnId"], "turn-1");
            server_writer
                .write_all(
                    br#"{"id":1,"result":{"accepted":true}}
"#,
                )
                .await
                .unwrap();
            let interrupt: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(interrupt["method"], "turn/interrupt");
            assert_eq!(interrupt["params"]["turnId"], "turn-1");
            server_writer
                .write_all(
                    br#"{"id":2,"result":{"accepted":true}}
"#,
                )
                .await
                .unwrap();
        });
        assert_eq!(
            session
                .steer_turn(
                    "continue".to_string(),
                    "turn-1".to_string(),
                    Duration::from_secs(1)
                )
                .await
                .unwrap()["accepted"],
            true
        );
        assert_eq!(
            session
                .interrupt_turn(Duration::from_secs(1))
                .await
                .unwrap()["accepted"],
            true
        );
        assert!(matches!(
            session
                .steer_turn(
                    "stale".to_string(),
                    "turn-old".to_string(),
                    Duration::from_secs(1)
                )
                .await,
            Err(CodexAppServerProtocolError::TurnScopeMismatch { .. })
        ));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn reconciles_delta_preview_with_authoritative_completion() {
        let (session, _reader, _writer) = session();
        session.state.lock().unwrap().thread_id = Some("thread-1".to_string());
        session.state.lock().unwrap().turn_id = Some("turn-1".to_string());

        let delta = session
            .apply_incoming(AppServerIncoming::Notification {
                method: "item/agentMessage/delta".to_string(),
                params: json!({"threadId":"thread-1","turnId":"turn-1","itemId":"item-1","delta":"hel"}),
            })
            .unwrap();
        assert!(
            matches!(delta, CodexAppServerEvent::AgentMessageDelta { ref preview, .. } if preview == "hel")
        );
        session
            .apply_incoming(AppServerIncoming::Notification {
                method: "item/agentMessage/delta".to_string(),
                params: json!({"threadId":"thread-1","turnId":"turn-1","itemId":"item-1","delta":"lo"}),
            })
            .unwrap();
        session
            .apply_incoming(AppServerIncoming::Notification {
                method: "item/completed".to_string(),
                params: json!({"threadId":"thread-1","turnId":"turn-1","item":{"id":"item-1","type":"agentMessage","text":"hello"}}),
            })
            .unwrap();
        let item = session.snapshot().items.remove("item-1").unwrap();
        assert_eq!(item.preview, "");
        assert!(item.completed);
        assert_eq!(item.authoritative.unwrap()["text"], "hello");

        session
            .apply_incoming(AppServerIncoming::Notification {
                method: "turn/completed".to_string(),
                params: json!({"threadId":"thread-1","turn":{"id":"turn-1","status":"completed"}}),
            })
            .unwrap();
        let snapshot = session.snapshot();
        assert_eq!(snapshot.phase, SessionPhase::Completed);
        assert_eq!(snapshot.turn_id, None);
    }

    #[tokio::test]
    async fn rejects_cross_thread_events_and_retains_unknown_notifications_bounded() {
        let (session, _reader, _writer) = session();
        session.state.lock().unwrap().thread_id = Some("thread-1".to_string());
        let error = session
            .apply_incoming(AppServerIncoming::Notification {
                method: "thread/status/changed".to_string(),
                params: json!({"threadId":"thread-2","status":{"type":"idle"}}),
            })
            .unwrap_err();
        assert!(matches!(
            error,
            CodexAppServerProtocolError::ThreadScopeMismatch { .. }
        ));

        for _ in 0..(MAX_RAW_EVENTS + 7) {
            session
                .apply_incoming(AppServerIncoming::Notification {
                    method: "future/event".to_string(),
                    params: json!({"ok":true}),
                })
                .unwrap();
        }
        assert_eq!(session.snapshot().raw_unknown_events.len(), MAX_RAW_EVENTS);
    }

    /// ReAgent P1, PR #3210: the `turn/start` response and a `turn/completed`
    /// notification for the SAME turn arrive on independent channels (the
    /// request/response oneshot vs. run_reader's notification stream), so
    /// the notification can be applied before `start_turn`'s own
    /// continuation resumes. Before the fix, `start_turn`'s unconditional
    /// post-await write would resurrect the already-terminal turn as
    /// `Running`, permanently stuck (every later `start_turn` call would
    /// see a phantom `TurnAlreadyActive`).
    #[tokio::test]
    async fn a_turn_completed_notification_that_lands_before_the_turn_start_response_is_not_overwritten(
    ) {
        let (session, server_reader, mut server_writer) = session();
        let server = tokio::spawn(async move {
            let mut lines = BufReader::new(server_reader).lines();
            let _start: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            server_writer
                .write_all(br#"{"id":1,"result":{"thread":{"id":"thread-1"}}}
"#)
                .await
                .unwrap();
            let _turn: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            // Completion notification written and processed BEFORE the
            // turn/start response, on purpose.
            server_writer
                .write_all(
                    b"{\"method\":\"turn/completed\",\"params\":{\"threadId\":\"thread-1\",\"turn\":{\"id\":\"turn-1\",\"status\":\"completed\"}}}\n",
                )
                .await
                .unwrap();
            server_writer
                .write_all(br#"{"id":2,"result":{"turn":{"id":"turn-1"}}}
"#)
                .await
                .unwrap();
        });

        session
            .start_thread(ThreadStartOptions::default(), Duration::from_secs(1))
            .await
            .unwrap();

        let session_for_turn = session.clone();
        let start_turn_task = tokio::spawn(async move {
            session_for_turn
                .start_turn("hello".to_string(), Duration::from_secs(1))
                .await
        });

        // Drain and apply the notification BEFORE start_turn's own response
        // resolves — the reader processes frames in wire order, so this
        // happens-before the response gets processed.
        let notification = session.transport.next_incoming().await.unwrap();
        session.apply_incoming(notification).unwrap();
        assert_eq!(session.snapshot().phase, SessionPhase::Completed);

        let turn_id = start_turn_task.await.unwrap().unwrap();
        assert_eq!(turn_id, "turn-1", "start_turn still reports success to its own caller");

        let snapshot = session.snapshot();
        assert_eq!(
            snapshot.phase,
            SessionPhase::Completed,
            "a turn/completed that landed first must not be overwritten back to Running"
        );
        assert!(snapshot.turn_id.is_none());

        server.await.unwrap();
    }

    /// ReAgent P1, PR #3210: `resume_thread` used to unconditionally clear
    /// turn_id/phase/items, silently dropping a turn the 0.154.0 contract
    /// reports as still `inProgress` in `ThreadResumeResponse.thread.turns`.
    #[tokio::test]
    async fn resume_thread_derives_an_in_progress_turn_instead_of_dropping_it() {
        let (session, server_reader, mut server_writer) = session();
        let server = tokio::spawn(async move {
            let mut lines = BufReader::new(server_reader).lines();
            let _request: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            server_writer
                .write_all(
                    br#"{"id":1,"result":{"thread":{"id":"thread-1","turns":[{"id":"turn-old","status":"completed"},{"id":"turn-1","status":"inProgress"}]}}}
"#,
                )
                .await
                .unwrap();
        });

        session
            .resume_thread("thread-1".to_string(), None, None, Duration::from_secs(1))
            .await
            .unwrap();

        let snapshot = session.snapshot();
        assert_eq!(snapshot.phase, SessionPhase::Running);
        assert_eq!(snapshot.turn_id.as_deref(), Some("turn-1"));

        server.await.unwrap();
    }

    #[tokio::test]
    async fn resume_thread_with_no_in_progress_turn_still_resets_to_idle() {
        let (session, server_reader, mut server_writer) = session();
        let server = tokio::spawn(async move {
            let mut lines = BufReader::new(server_reader).lines();
            let _request: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            server_writer
                .write_all(
                    br#"{"id":1,"result":{"thread":{"id":"thread-1","turns":[{"id":"turn-old","status":"completed"}]}}}
"#,
                )
                .await
                .unwrap();
        });

        session
            .resume_thread("thread-1".to_string(), None, None, Duration::from_secs(1))
            .await
            .unwrap();

        let snapshot = session.snapshot();
        assert_eq!(snapshot.phase, SessionPhase::Idle);
        assert!(snapshot.turn_id.is_none());

        server.await.unwrap();
    }

    /// ReAgent P1, PR #3210: a delayed/redundant `thread/started` for a
    /// thread that's already loaded (and already has an active turn) must
    /// not wipe that turn state — only a genuinely NEW thread id should
    /// reset it.
    #[tokio::test]
    async fn a_redundant_thread_started_for_the_same_thread_does_not_wipe_an_active_turn() {
        let (session, server_reader, mut server_writer) = session();
        let server = tokio::spawn(async move {
            let mut lines = BufReader::new(server_reader).lines();
            let _start: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            server_writer
                .write_all(br#"{"id":1,"result":{"thread":{"id":"thread-1"}}}
"#)
                .await
                .unwrap();
            let _turn: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            server_writer
                .write_all(br#"{"id":2,"result":{"turn":{"id":"turn-1"}}}
"#)
                .await
                .unwrap();
        });

        session
            .start_thread(ThreadStartOptions::default(), Duration::from_secs(1))
            .await
            .unwrap();
        session
            .start_turn("hello".to_string(), Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(session.snapshot().phase, SessionPhase::Running);

        session
            .apply_incoming(AppServerIncoming::Notification {
                method: "thread/started".to_string(),
                params: json!({"thread": {"id": "thread-1"}}),
            })
            .unwrap();

        let snapshot = session.snapshot();
        assert_eq!(
            snapshot.phase,
            SessionPhase::Running,
            "a redundant thread/started for the SAME thread must not wipe the active turn"
        );
        assert_eq!(snapshot.turn_id.as_deref(), Some("turn-1"));

        server.await.unwrap();
    }

    #[tokio::test]
    async fn a_thread_started_for_a_genuinely_new_thread_still_resets_turn_state() {
        let (session, _reader, _writer) = session();
        {
            let mut state = session.state.lock().unwrap();
            state.thread_id = Some("thread-1".to_string());
            state.turn_id = Some("turn-1".to_string());
            state.phase = SessionPhase::Running;
        }

        session
            .apply_incoming(AppServerIncoming::Notification {
                method: "thread/started".to_string(),
                params: json!({"thread": {"id": "thread-2"}}),
            })
            .unwrap();

        let snapshot = session.snapshot();
        assert_eq!(snapshot.thread_id.as_deref(), Some("thread-2"));
        assert_eq!(snapshot.phase, SessionPhase::Idle);
        assert!(snapshot.turn_id.is_none());
    }

    /// ReAgent P1, PR #3210 (2nd review): collapsing "interrupted" (user
    /// cancellation) into the same `Failed` phase as a genuine
    /// provider/execution error hid an expected outcome behind an error
    /// signal.
    #[tokio::test]
    async fn turn_completed_with_interrupted_status_maps_to_its_own_phase() {
        let (session, _reader, _writer) = session();
        {
            let mut state = session.state.lock().unwrap();
            state.thread_id = Some("thread-1".to_string());
            state.turn_id = Some("turn-1".to_string());
            state.phase = SessionPhase::Running;
        }

        session
            .apply_incoming(AppServerIncoming::Notification {
                method: "turn/completed".to_string(),
                params: json!({
                    "threadId": "thread-1",
                    "turn": {"id": "turn-1", "status": "interrupted"},
                }),
            })
            .unwrap();

        let snapshot = session.snapshot();
        assert_eq!(
            snapshot.phase,
            SessionPhase::Interrupted,
            "an interrupted (cancelled) turn must not read the same as a genuine failure"
        );
        assert!(snapshot.turn_id.is_none());
    }

    #[tokio::test]
    async fn turn_completed_with_a_genuine_failure_still_maps_to_failed() {
        let (session, _reader, _writer) = session();
        {
            let mut state = session.state.lock().unwrap();
            state.thread_id = Some("thread-1".to_string());
            state.turn_id = Some("turn-1".to_string());
            state.phase = SessionPhase::Running;
        }

        session
            .apply_incoming(AppServerIncoming::Notification {
                method: "turn/completed".to_string(),
                params: json!({
                    "threadId": "thread-1",
                    "turn": {"id": "turn-1", "status": "failed"},
                }),
            })
            .unwrap();

        assert_eq!(session.snapshot().phase, SessionPhase::Failed);
    }

    /// ReAgent P1, PR #3210 (2nd review): start_turn's check-then-act
    /// (read turn_id, release the lock, THEN await the request) wasn't
    /// atomic — nothing was reserved under the lock, so two concurrent
    /// calls before either got a response could both observe turn_id ==
    /// None and both send turn/start. `tokio::join!` polls its futures in
    /// argument order on their first poll; neither future reaches an
    /// `.await` point before the synchronous turn_starting check/set, so
    /// this is deterministic, not timing-dependent: the first call's
    /// synchronous prefix runs to completion (reserving the slot) before
    /// the second call's synchronous prefix ever runs.
    #[tokio::test]
    async fn concurrent_start_turn_calls_do_not_both_send_turn_start() {
        let (session, server_reader, mut server_writer) = session();
        let server = tokio::spawn(async move {
            let mut lines = BufReader::new(server_reader).lines();
            let _start: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            server_writer
                .write_all(br#"{"id":1,"result":{"thread":{"id":"thread-1"}}}
"#)
                .await
                .unwrap();

            // Exactly one turn/start line — if the fix regressed, a second
            // concurrent call would send a second one here.
            let turn: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(turn["method"], "turn/start");
            server_writer
                .write_all(br#"{"id":2,"result":{"turn":{"id":"turn-1"}}}
"#)
                .await
                .unwrap();

            // Confirm nothing further arrives — a regression would send a
            // second turn/start request here.
            let extra = tokio::time::timeout(Duration::from_millis(200), lines.next_line()).await;
            assert!(
                extra.is_err(),
                "a second concurrent start_turn call must be rejected locally, not also sent over the wire"
            );
        });

        session
            .start_thread(ThreadStartOptions::default(), Duration::from_secs(1))
            .await
            .unwrap();

        let session_a = session.clone();
        let session_b = session.clone();
        let (result_a, result_b) = tokio::join!(
            session_a.start_turn("hello".to_string(), Duration::from_secs(1)),
            session_b.start_turn("world".to_string(), Duration::from_secs(1)),
        );

        let results = [result_a, result_b];
        let ok_count = results.iter().filter(|r| r.is_ok()).count();
        let rejected_count = results
            .iter()
            .filter(|r| matches!(r, Err(CodexAppServerProtocolError::TurnAlreadyActive(_))))
            .count();
        assert_eq!(ok_count, 1, "exactly one of the two concurrent calls should succeed");
        assert_eq!(
            rejected_count, 1,
            "the other must be rejected with TurnAlreadyActive, not also sent over the wire"
        );

        server.await.unwrap();
    }

    #[tokio::test]
    async fn routes_supported_requests_without_auto_approval_and_scopes_them() {
        let (session, _reader, _writer) = session();
        {
            let mut state = session.state.lock().unwrap();
            state.thread_id = Some("thread-1".to_string());
            state.turn_id = Some("turn-1".to_string());
        }
        for (id, method) in [
            ("command", "item/commandExecution/requestApproval"),
            ("file", "item/fileChange/requestApproval"),
            ("permissions", "item/permissions/requestApproval"),
            ("input", "item/tool/requestUserInput"),
        ] {
            let pending = session
                .accept_server_request(AppServerIncoming::Request {
                    id: RpcId::String(id.to_string()),
                    method: method.to_string(),
                    params: json!({"threadId":"thread-1","turnId":"turn-1","itemId":id}),
                })
                .unwrap();
            assert_eq!(pending.method, method);
        }
        let mcp = session
            .accept_server_request(AppServerIncoming::Request {
                id: RpcId::String("mcp".to_string()),
                method: "mcpServer/elicitation/request".to_string(),
                params: json!({"message":"confirm","mode":"form","requestedSchema":{}}),
            })
            .unwrap();
        assert_eq!(mcp.kind, ServerRequestKind::McpElicitation);
        assert_eq!(session.pending_server_requests().len(), 5);
        assert!(matches!(
            session.accept_server_request(AppServerIncoming::Request {
                id: RpcId::String("unknown".to_string()),
                method: "future/request".to_string(),
                params: json!({})
            }),
            Err(CodexAppServerProtocolError::UnsupportedServerRequest(_))
        ));
        assert!(matches!(
            session.accept_server_request(AppServerIncoming::Request {
                id: RpcId::String("cross".to_string()),
                method: "item/tool/requestUserInput".to_string(),
                params: json!({"threadId":"thread-2","turnId":"turn-1","itemId":"x"})
            }),
            Err(CodexAppServerProtocolError::ThreadScopeMismatch { .. })
        ));
    }

    #[tokio::test]
    async fn rejects_duplicate_or_unknown_decisions_and_clears_on_turn_completion() {
        let (session, _reader, _writer) = session();
        {
            let mut state = session.state.lock().unwrap();
            state.thread_id = Some("thread-1".to_string());
            state.turn_id = Some("turn-1".to_string());
        }
        let id = RpcId::String("approval".to_string());
        let incoming = AppServerIncoming::Request {
            id: id.clone(),
            method: "item/tool/requestUserInput".to_string(),
            params: json!({"threadId":"thread-1","turnId":"turn-1","itemId":"item-1"}),
        };
        session.accept_server_request(incoming.clone()).unwrap();
        assert!(matches!(
            session.accept_server_request(incoming),
            Err(CodexAppServerProtocolError::DuplicateServerRequest)
        ));
        assert!(matches!(
            session
                .respond_server_request(
                    &RpcId::String("missing".to_string()),
                    Ok(json!({})),
                    Duration::from_millis(10)
                )
                .await,
            Err(CodexAppServerProtocolError::UnknownServerRequest)
        ));
        session
            .apply_incoming(AppServerIncoming::Notification {
                method: "turn/completed".to_string(),
                params: json!({"threadId":"thread-1","turn":{"id":"turn-1","status":"completed"}}),
            })
            .unwrap();
        assert!(session.pending_server_requests().is_empty());
    }

    /// ReAgent P1, PR #3212 (2nd review): item-scoped kinds (everything
    /// except McpElicitation) must require threadId/turnId rather than
    /// silently treating an omitted field as "no scope claimed" -- that
    /// contradicted this API's own "scoped to active thread/turn"
    /// fail-closed claim.
    #[tokio::test]
    async fn an_item_scoped_request_missing_thread_id_is_rejected_not_silently_queued() {
        let (session, _reader, _writer) = session();
        {
            let mut state = session.state.lock().unwrap();
            state.thread_id = Some("thread-1".to_string());
            state.turn_id = Some("turn-1".to_string());
        }
        assert!(matches!(
            session.accept_server_request(AppServerIncoming::Request {
                id: RpcId::String("no-scope".to_string()),
                method: "item/tool/requestUserInput".to_string(),
                params: json!({"itemId": "item-1"}),
            }),
            Err(CodexAppServerProtocolError::MissingStringField("threadId"))
        ));
        assert!(session.pending_server_requests().is_empty());

        // McpElicitation alone stays exempt -- it genuinely has no
        // thread/turn context.
        let mcp = session
            .accept_server_request(AppServerIncoming::Request {
                id: RpcId::String("mcp".to_string()),
                method: "mcpServer/elicitation/request".to_string(),
                params: json!({"message": "confirm"}),
            })
            .unwrap();
        assert!(mcp.thread_id.is_none());
    }

    /// ReAgent P1, PR #3212 (2nd review): a pending approval queued under a
    /// prior thread/turn must not remain answerable after that thread has
    /// been replaced -- only turn/completed cleared pending_requests
    /// before; start_thread, resume_thread, and a genuinely-new
    /// thread/started notification did not.
    #[tokio::test]
    async fn thread_transitions_clear_stale_pending_requests() {
        let (session, server_reader, mut server_writer) = session();
        let server = tokio::spawn(async move {
            let mut lines = BufReader::new(server_reader).lines();
            let _start: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            server_writer
                .write_all(br#"{"id":1,"result":{"thread":{"id":"thread-2"}}}
"#)
                .await
                .unwrap();
        });

        {
            let mut state = session.state.lock().unwrap();
            state.thread_id = Some("thread-1".to_string());
        }
        session
            .accept_server_request(AppServerIncoming::Request {
                id: RpcId::String("stale".to_string()),
                method: "mcpServer/elicitation/request".to_string(),
                params: json!({"message": "confirm"}),
            })
            .unwrap();
        assert_eq!(session.pending_server_requests().len(), 1);

        session
            .start_thread(ThreadStartOptions::default(), Duration::from_secs(1))
            .await
            .unwrap();
        assert!(
            session.pending_server_requests().is_empty(),
            "start_thread must clear pending requests queued under the previous thread"
        );

        server.await.unwrap();
    }

    /// ReAgent P2, PR #3212 (2nd review): respond_server_request checked
    /// pending_requests membership, then awaited the transport write,
    /// THEN removed the entry -- two concurrent calls for the same id
    /// could both pass the membership check before either removed it, so
    /// both would send a response and both report success. Now removed
    /// atomically up front; deterministic via tokio::join! the same way
    /// the concurrent-start_turn test is (neither future reaches an
    /// await point before the synchronous remove).
    #[tokio::test]
    async fn concurrent_respond_calls_for_the_same_id_do_not_both_succeed() {
        let (session, server_reader, mut server_writer) = session();
        let server = tokio::spawn(async move {
            let mut lines = BufReader::new(server_reader).lines();
            // Exactly one respond frame should be written.
            let _line = lines.next_line().await.unwrap().unwrap();
            server_writer.write_all(b"ignored\n").await.ok();
            let extra = tokio::time::timeout(Duration::from_millis(200), lines.next_line()).await;
            assert!(extra.is_err(), "a second concurrent respond call must not also write a response");
        });

        {
            let mut state = session.state.lock().unwrap();
            state.thread_id = Some("thread-1".to_string());
        }
        let id = RpcId::String("approval".to_string());
        session
            .accept_server_request(AppServerIncoming::Request {
                id: id.clone(),
                method: "mcpServer/elicitation/request".to_string(),
                params: json!({"message": "confirm"}),
            })
            .unwrap();

        let session_a = session.clone();
        let session_b = session.clone();
        let id_a = id.clone();
        let id_b = id.clone();
        let (result_a, result_b) = tokio::join!(
            session_a.respond_server_request(&id_a, Ok(json!({"approved": true})), Duration::from_secs(1)),
            session_b.respond_server_request(&id_b, Ok(json!({"approved": true})), Duration::from_secs(1)),
        );
        let results = [result_a, result_b];
        let ok_count = results.iter().filter(|r| r.is_ok()).count();
        let unknown_count = results
            .iter()
            .filter(|r| matches!(r, Err(CodexAppServerProtocolError::UnknownServerRequest)))
            .count();
        assert_eq!(ok_count, 1, "exactly one of the two concurrent respond calls should succeed");
        assert_eq!(unknown_count, 1, "the other must see it already gone, not also send a response");

        server.abort();
    }

    /// ReAgent P1, PR #3212 (3rd review): before any turn had started (or
    /// after turn/completed cleared turn_id back to None), a request
    /// claiming an arbitrary/stale turnId passed the old ensure_scopes
    /// check trivially, since that helper only compares when state.turn_id
    /// is Some. accept_server_request must fail closed when there is no
    /// active thread/turn to scope the request to, not merely skip the
    /// check.
    #[tokio::test]
    async fn a_request_claiming_a_thread_or_turn_is_rejected_when_none_is_active() {
        let (session, _reader, _writer) = session();
        assert!(matches!(
            session.accept_server_request(AppServerIncoming::Request {
                id: RpcId::String("no-thread".to_string()),
                method: "item/tool/requestUserInput".to_string(),
                params: json!({"threadId": "thread-1", "turnId": "turn-1", "itemId": "x"}),
            }),
            Err(CodexAppServerProtocolError::ThreadNotLoaded)
        ));

        {
            let mut state = session.state.lock().unwrap();
            state.thread_id = Some("thread-1".to_string());
        }
        assert!(matches!(
            session.accept_server_request(AppServerIncoming::Request {
                id: RpcId::String("no-turn".to_string()),
                method: "item/tool/requestUserInput".to_string(),
                params: json!({"threadId": "thread-1", "turnId": "turn-1", "itemId": "x"}),
            }),
            Err(CodexAppServerProtocolError::TurnNotActive)
        ));
        assert!(session.pending_server_requests().is_empty());
    }

    /// ReAgent P1, PR #3212 (3rd review): respond_server_request's
    /// failure-path reinsert didn't check whether the thread/turn context
    /// was still current, so a stale approval could be resurrected into a
    /// brand-new session/turn after a transition raced the in-flight
    /// transport write.
    #[tokio::test]
    async fn a_failed_respond_is_not_resurrected_after_the_turn_it_was_queued_under_ends() {
        let (session, server_reader, server_writer) = session();

        {
            let mut state = session.state.lock().unwrap();
            state.thread_id = Some("thread-1".to_string());
            state.turn_id = Some("turn-1".to_string());
        }
        let id = RpcId::String("approval".to_string());
        session
            .accept_server_request(AppServerIncoming::Request {
                id: id.clone(),
                method: "item/tool/requestUserInput".to_string(),
                params: json!({"threadId": "thread-1", "turnId": "turn-1", "itemId": "x"}),
            })
            .unwrap();

        // Drop the entire peer side of the transport so the write inside
        // respond_server_request fails, standing in for "the transport
        // write raced a state transition and lost" without a timing-
        // dependent sleep.
        drop(server_reader);
        drop(server_writer);

        // Simulate the turn ending while that write is in flight: clear
        // pending_requests and the active turn the same way turn/completed
        // does.
        session.state.lock().unwrap().pending_requests.clear();
        session.state.lock().unwrap().turn_id = None;

        let result = session
            .respond_server_request(&id, Ok(json!({"approved": true})), Duration::from_secs(1))
            .await;
        assert!(result.is_err(), "the write should fail once the transport is closed");
        assert!(
            session.pending_server_requests().is_empty(),
            "a stale approval must not be resurrected once its turn has ended"
        );
    }

    /// ReAgent P2, PR #3212 (3rd review): pending_requests had no size cap,
    /// so a buggy or adversarial App Server could grow it unboundedly by
    /// sending validly-scoped approval requests.
    #[tokio::test]
    async fn accept_server_request_rejects_once_the_pending_cap_is_reached() {
        let (session, _reader, _writer) = session();
        {
            let mut state = session.state.lock().unwrap();
            state.thread_id = Some("thread-1".to_string());
        }
        for i in 0..MAX_PENDING_SERVER_REQUESTS {
            session
                .accept_server_request(AppServerIncoming::Request {
                    id: RpcId::String(format!("mcp-{i}")),
                    method: "mcpServer/elicitation/request".to_string(),
                    params: json!({"message": "confirm"}),
                })
                .unwrap();
        }
        assert!(matches!(
            session.accept_server_request(AppServerIncoming::Request {
                id: RpcId::String("one-too-many".to_string()),
                method: "mcpServer/elicitation/request".to_string(),
                params: json!({"message": "confirm"}),
            }),
            Err(CodexAppServerProtocolError::TooManyPendingRequests(
                MAX_PENDING_SERVER_REQUESTS
            ))
        ));
    }

    /// ReAgent P1, PR #3212 (4th review): a request carrying turnId but no
    /// threadId (McpElicitation may supply either field alone) fell through
    /// both `if let Some(thread) = &thread_id` and the `(Some, Some)` tuple
    /// match untouched, so a claimed-but-stale turnId was never validated
    /// and the request was queued unconditionally. Same asymmetry mirrored
    /// in respond_server_request's reinsert-on-failure check.
    #[tokio::test]
    async fn a_turn_only_request_is_still_scoped_to_the_active_turn() {
        let (session, _reader, _writer) = session();
        assert!(matches!(
            session.accept_server_request(AppServerIncoming::Request {
                id: RpcId::String("turn-only-no-turn".to_string()),
                method: "mcpServer/elicitation/request".to_string(),
                params: json!({"turnId": "turn-1"}),
            }),
            Err(CodexAppServerProtocolError::TurnNotActive)
        ));

        {
            let mut state = session.state.lock().unwrap();
            state.turn_id = Some("turn-1".to_string());
        }
        assert!(matches!(
            session.accept_server_request(AppServerIncoming::Request {
                id: RpcId::String("turn-only-stale".to_string()),
                method: "mcpServer/elicitation/request".to_string(),
                params: json!({"turnId": "turn-stale"}),
            }),
            Err(CodexAppServerProtocolError::TurnScopeMismatch { .. })
        ));
        assert!(session.pending_server_requests().is_empty());

        let pending = session
            .accept_server_request(AppServerIncoming::Request {
                id: RpcId::String("turn-only-current".to_string()),
                method: "mcpServer/elicitation/request".to_string(),
                params: json!({"turnId": "turn-1"}),
            })
            .unwrap();
        assert_eq!(pending.turn_id.as_deref(), Some("turn-1"));
    }

    /// See a_turn_only_request_is_still_scoped_to_the_active_turn -- the
    /// same asymmetry existed on the reinsert side: a pending entry with
    /// turn_id but no thread_id was unconditionally resurrected on a
    /// transport-write failure (`(None, _) => true`) regardless of
    /// whether that turn was still active.
    #[tokio::test]
    async fn a_failed_turn_only_respond_is_not_resurrected_after_its_turn_ends() {
        let (session, server_reader, server_writer) = session();
        {
            let mut state = session.state.lock().unwrap();
            state.turn_id = Some("turn-1".to_string());
        }
        let id = RpcId::String("turn-only".to_string());
        session
            .accept_server_request(AppServerIncoming::Request {
                id: id.clone(),
                method: "mcpServer/elicitation/request".to_string(),
                params: json!({"turnId": "turn-1"}),
            })
            .unwrap();

        drop(server_reader);
        drop(server_writer);
        session.state.lock().unwrap().pending_requests.clear();
        session.state.lock().unwrap().turn_id = None;

        let result = session
            .respond_server_request(&id, Ok(json!({"approved": true})), Duration::from_secs(1))
            .await;
        assert!(result.is_err(), "the write should fail once the transport is closed");
        assert!(
            session.pending_server_requests().is_empty(),
            "a stale turn-only approval must not be resurrected once its turn has ended"
        );
    }

    /// ReAgent P1, PR #3212 (5th review): respond_server_request removed the
    /// pending entry synchronously, then awaited the transport write. If
    /// the calling future were dropped/cancelled while suspended at that
    /// await (a caller-side select!/timeout racing the write), Rust never
    /// runs code after an await point that's never resumed -- neither the
    /// success path nor the old failure-path reinsert would run, silently
    /// stranding the entry with no response ever sent. A 1-byte duplex
    /// buffer with nothing reading the other side guarantees the write
    /// stays suspended, so aborting the task deterministically exercises
    /// the drop-mid-await path rather than racing a timing window.
    #[tokio::test]
    async fn a_cancelled_respond_future_reinserts_the_pending_entry() {
        let (client, _server) = duplex(1);
        let (client_reader, client_writer) = tokio::io::split(client);
        let transport = Arc::new(AppServerTransport::new(
            client_reader,
            client_writer,
            Default::default(),
        ));
        let session = Arc::new(CodexAppServerSession::new(transport));
        {
            let mut state = session.state.lock().unwrap();
            state.thread_id = Some("thread-1".to_string());
            state.turn_id = Some("turn-1".to_string());
        }
        let id = RpcId::String("approval".to_string());
        session
            .accept_server_request(AppServerIncoming::Request {
                id: id.clone(),
                method: "item/tool/requestUserInput".to_string(),
                params: json!({"threadId": "thread-1", "turnId": "turn-1", "itemId": "x"}),
            })
            .unwrap();

        let task_session = session.clone();
        let task_id = id.clone();
        let handle = tokio::spawn(async move {
            task_session
                .respond_server_request(
                    &task_id,
                    Ok(json!({"approved": true})),
                    Duration::from_secs(30),
                )
                .await
        });

        // The synchronous remove() runs on the task's first poll, before
        // it ever reaches the write's await point -- give it a bounded
        // number of scheduling turns to get there rather than assuming
        // one yield is always enough.
        for _ in 0..100 {
            if session.pending_server_requests().is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(
            session.pending_server_requests().is_empty(),
            "the entry should already be removed while the write is in flight"
        );

        handle.abort();
        let _ = handle.await;

        assert_eq!(
            session.pending_server_requests().len(),
            1,
            "cancelling the respond future must reinsert the pending entry, not lose it silently"
        );
    }
}
