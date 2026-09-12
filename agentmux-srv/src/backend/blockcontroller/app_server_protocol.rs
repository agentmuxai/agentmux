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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionPhase {
    Idle,
    Running,
    Completed,
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
        let mut state = self.state.lock().unwrap();
        state.thread_id = Some(resumed_id.clone());
        state.turn_id = None;
        state.phase = SessionPhase::Idle;
        state.items.clear();
        state.pending_requests.clear();
        Ok(resumed_id)
    }

    pub async fn start_turn(
        &self,
        text: String,
        timeout: Duration,
    ) -> Result<String, CodexAppServerProtocolError> {
        let thread_id = {
            let state = self.state.lock().unwrap();
            if let Some(turn_id) = &state.turn_id {
                return Err(CodexAppServerProtocolError::TurnAlreadyActive(
                    turn_id.clone(),
                ));
            }
            state
                .thread_id
                .clone()
                .ok_or(CodexAppServerProtocolError::ThreadNotLoaded)?
        };
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
        let turn_id = string_field(turn.get("id"), "turn.id")?;
        let mut state = self.state.lock().unwrap();
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
        let thread_id = params
            .get("threadId")
            .and_then(Value::as_str)
            .map(str::to_string);
        let turn_id = params
            .get("turnId")
            .and_then(Value::as_str)
            .map(str::to_string);
        let item_id = params
            .get("itemId")
            .and_then(Value::as_str)
            .map(str::to_string);
        let mut state = self.state.lock().unwrap();
        if state.pending_requests.contains_key(&id) {
            return Err(CodexAppServerProtocolError::DuplicateServerRequest);
        }
        if let Some(thread) = &thread_id {
            ensure_thread_scope(&state, thread)?;
        }
        if let (Some(thread), Some(turn)) = (&thread_id, &turn_id) {
            ensure_scopes(&state, thread, turn)?;
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
        let pending = self
            .state
            .lock()
            .unwrap()
            .pending_requests
            .remove(id)
            .ok_or(CodexAppServerProtocolError::UnknownServerRequest)?;
        if let Err(error) = self.transport.respond(id.clone(), result, timeout).await {
            self.state
                .lock()
                .unwrap()
                .pending_requests
                .insert(id.clone(), pending);
            return Err(error.into());
        }
        Ok(())
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
                state.thread_id = Some(thread_id.clone());
                state.turn_id = None;
                state.phase = SessionPhase::Idle;
                state.items.clear();
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
                state.phase = if status == "completed" {
                    SessionPhase::Completed
                } else {
                    SessionPhase::Failed
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
}
