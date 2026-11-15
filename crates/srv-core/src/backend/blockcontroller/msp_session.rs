// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The MSP client's state: the handshake, the session, which turns are still
//! running, and what each line from `muse serve` asks of the controller. No
//! I/O, so the controller's whole reaction to a line is testable without a
//! process (`MspController` in msp.rs does the I/O).
//!
//! Handshake: `initialize` → `initialized` → `session/start` (or
//! `session/resume` of the pane's previous session, falling back to a new
//! one). A turn is a `turn/start` command whose ack names its turn; the turn
//! ends with the `turn/completed` notification.

use std::collections::HashSet;

use serde_json::{Map, Value};

use super::msp_protocol as proto;

/// What one line from the host asks of the controller.
#[derive(Debug, Default, PartialEq)]
pub(super) struct Effects {
    /// Lines to write to the host's stdin, in order.
    pub send: Vec<String>,
    /// Keep the line out of the transcript: an error the client recovered
    /// from, or the answer to a command the pane didn't send.
    pub hide: bool,
    /// Lines to write to the transcript after this one.
    pub append: Vec<String>,
    /// A session the controller now drives, to persist as `agent:sessionid`.
    pub session: Option<String>,
    /// The turn started (`true`) or every turn ended (`false`).
    pub turn_active: Option<bool>,
}

/// The MSP client of one `muse serve` process.
pub(super) struct MspSession {
    next_id: u64,
    cwd: String,
    mcp_servers: Map<String, Value>,
    /// The session to resume instead of starting one.
    resume: Option<String>,
    init_id: Option<u64>,
    /// The `session/start` or `session/resume` in flight, and whether it is a
    /// resume.
    session_req: Option<(u64, bool)>,
    session_id: Option<String>,
    /// The session couldn't open: nothing runs until the agent restarts.
    failed: bool,
    /// Messages sent before the session opened, joined.
    pending: Option<String>,
    /// `turn/start`s sent and not answered yet.
    turn_requests: HashSet<u64>,
    /// Turns the host started (or queued) that haven't completed.
    open_turns: HashSet<String>,
    /// Turns that completed while a `turn/start` was unanswered: its late ack
    /// may name one, which must not count as running again.
    ended_turns: HashSet<String>,
    /// Commands srv sent on its own (decisions, cancels, interrupts): an
    /// error answering one is logged, not shown.
    quiet: HashSet<u64>,
    /// Approvals already decided, by approval and stage.
    decided: HashSet<String>,
}

impl MspSession {
    pub(super) fn new() -> Self {
        Self {
            next_id: 1,
            cwd: String::new(),
            mcp_servers: Map::new(),
            resume: None,
            init_id: None,
            session_req: None,
            session_id: None,
            failed: false,
            pending: None,
            turn_requests: HashSet::new(),
            open_turns: HashSet::new(),
            ended_turns: HashSet::new(),
            quiet: HashSet::new(),
            decided: HashSet::new(),
        }
    }

    fn id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// A new process: the handshake starts over, in `cwd`, with these MCP
    /// servers, resuming `resume` when given. A message still waiting for a
    /// session waits for the new one. Returns the `initialize` request.
    pub(super) fn begin(&mut self, cwd: String, mcp_servers: Map<String, Value>, resume: Option<String>) -> String {
        let pending = self.pending.take();
        *self = Self { next_id: self.next_id, ..Self::new() };
        self.pending = pending;
        self.cwd = cwd;
        self.mcp_servers = mcp_servers;
        self.resume = resume;
        let id = self.id();
        self.init_id = Some(id);
        proto::request(id, "initialize", proto::initialize_params())
    }

    #[cfg(test)]
    pub(super) fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// A turn is running, or a message waits to start one.
    pub(super) fn turn_active(&self) -> bool {
        !self.turn_requests.is_empty() || !self.open_turns.is_empty() || self.pending.is_some()
    }

    /// Work the process was doing when it exited: a turn, or a message
    /// waiting for a session. Forgets it; the caller tells the pane.
    pub(super) fn process_ended(&mut self) -> bool {
        let had_work = self.turn_active();
        self.pending = None;
        self.turn_requests.clear();
        self.open_turns.clear();
        self.ended_turns.clear();
        self.session_id = None;
        had_work
    }

    fn session_request(&mut self, resume: bool) -> String {
        let id = self.id();
        self.session_req = Some((id, resume));
        let resume_id = if resume { self.resume.clone() } else { None };
        let (method, params) = proto::session_request(resume_id.as_deref(), &self.cwd, &self.mcp_servers);
        proto::request(id, method, params)
    }

    fn turn_start(&mut self, session_id: &str, text: &str) -> (u64, String) {
        let id = self.id();
        self.turn_requests.insert(id);
        (id, proto::request(id, "turn/start", proto::turn_start_params(session_id, text)))
    }

    /// The user's message. `Ok(Some((id, line)))`: send it (the turn is now
    /// active). `Ok(None)`: it waits for the session, which a closed session
    /// is reopened for (the returned line). `Err`: the session can't open.
    pub(super) fn prompt(&mut self, text: &str) -> Result<PromptOutcome, String> {
        if let Some(sid) = self.session_id.clone() {
            let (id, line) = self.turn_start(&sid, text);
            return Ok(PromptOutcome::Send { id, line });
        }
        if self.failed {
            return Err("Muse Code couldn't open a session (see the error above). Fix that, then restart the agent.".to_string());
        }
        self.pending = Some(match self.pending.take() {
            Some(earlier) => format!("{earlier}\n\n{text}"),
            None => text.to_string(),
        });
        // The host unloaded the session (`session/closed`): load it again.
        let reopen = (self.init_id.is_none() && self.session_req.is_none() && self.resume.is_some())
            .then(|| self.session_request(true));
        Ok(PromptOutcome::Queued { reopen })
    }

    /// A `turn/start` that never reached the host: forgotten. True when no
    /// turn is left running.
    pub(super) fn prompt_unsent(&mut self, id: u64) -> bool {
        self.turn_requests.remove(&id);
        !self.turn_active()
    }

    /// `turn/interrupt` for the running turn, when there is one to stop.
    pub(super) fn interrupt(&mut self) -> Option<String> {
        let sid = self.session_id.clone()?;
        if self.turn_requests.is_empty() && self.open_turns.is_empty() {
            return None;
        }
        let id = self.id();
        self.quiet.insert(id);
        Some(proto::request(id, "turn/interrupt", proto::turn_interrupt_params(&sid)))
    }

    /// React to one line from the host.
    pub(super) fn on_message(&mut self, msg: &Value) -> Effects {
        let mut fx = Effects::default();
        let method = msg.get("method").and_then(Value::as_str);
        match (method, msg.get("id")) {
            (Some(method), Some(id)) => self.on_host_request(method, id, msg.get("params"), &mut fx),
            (Some(method), None) => self.on_notification(method, msg.get("params"), &mut fx),
            (None, Some(id)) => match id.as_u64() {
                Some(id) => self.on_response(id, msg, &mut fx),
                None => {}
            },
            (None, None) => {}
        }
        fx
    }

    fn on_host_request(&mut self, method: &str, id: &Value, params: Option<&Value>, fx: &mut Effects) {
        fx.send.push(proto::reply_to_host_request(id, method));
        let Some(params) = params else { return };
        let command = match method {
            "approval/request" => {
                let key = format!("{}:{}", params["approvalId"], params["currentRequirementId"]);
                if !self.decided.insert(key) {
                    return;
                }
                proto::approval_decide_params(params).map(|p| ("approval/decide", p))
            }
            "userInput/request" => proto::user_input_cancel_params(params).map(|p| ("userInput/cancel", p)),
            _ => None,
        };
        if let Some((method, p)) = command {
            let id = self.id();
            self.quiet.insert(id);
            fx.send.push(proto::request(id, method, p));
        }
    }

    fn on_notification(&mut self, method: &str, params: Option<&Value>, fx: &mut Effects) {
        let turn_id = params.and_then(|p| p.get("turnId")).and_then(Value::as_str).filter(|t| !t.is_empty());
        let ours = |p: Option<&Value>, sid: &Option<String>| {
            sid.is_some() && p.and_then(|p| p.get("sessionId")).and_then(Value::as_str) == sid.as_deref()
        };
        match method {
            "turn/started" => {
                if let Some(t) = turn_id {
                    if !self.ended_turns.contains(t) && self.open_turns.insert(t.to_string()) {
                        fx.turn_active = Some(true);
                    }
                }
            }
            "turn/completed" => {
                if let Some(t) = turn_id {
                    self.open_turns.remove(t);
                    if !self.turn_requests.is_empty() {
                        self.ended_turns.insert(t.to_string());
                    }
                }
                self.settle(fx);
            }
            "session/closed" if ours(params, &self.session_id) => {
                // Unloaded while idle: the next message loads it again.
                self.resume = self.session_id.take();
                self.open_turns.clear();
            }
            _ => {}
        }
    }

    fn on_response(&mut self, id: u64, msg: &Value, fx: &mut Effects) {
        let error = msg.get("error");
        let result = msg.get("result");
        if self.init_id == Some(id) {
            self.init_id = None;
            match result {
                Some(r) => self.on_initialized(r, fx),
                None => self.fail(fx),
            }
        } else if self.session_req.map(|(sid, _)| sid) == Some(id) {
            let (_, was_resume) = self.session_req.take().unwrap_or_default();
            let opened = result.and_then(|r| r.pointer("/session/sessionId")).and_then(Value::as_str);
            match (opened, error) {
                (Some(sid), _) => {
                    self.session_id = Some(sid.to_string());
                    self.resume = None;
                    fx.session = Some(sid.to_string());
                    if let Some(text) = self.pending.take() {
                        let sid = sid.to_string();
                        let (_, line) = self.turn_start(&sid, &text);
                        fx.send.push(line);
                        fx.turn_active = Some(true);
                    }
                }
                (None, Some(_)) if was_resume => {
                    // The previous session is gone: a new one instead.
                    tracing::info!(error = %msg, "[msp] session/resume refused; starting a new session");
                    fx.hide = true;
                    self.resume = None;
                    let line = self.session_request(false);
                    fx.send.push(line);
                }
                _ => self.fail(fx),
            }
        } else if self.turn_requests.remove(&id) {
            if let Some(r) = result {
                let disposition = r.get("disposition").and_then(Value::as_str).unwrap_or("started");
                let turn = r.get("turnId").and_then(Value::as_str).filter(|t| !t.is_empty());
                if let Some(t) = turn.filter(|_| matches!(disposition, "started" | "queued" | "steered")) {
                    if !self.ended_turns.contains(t) {
                        self.open_turns.insert(t.to_string());
                    }
                }
            }
            if !self.turn_active() && error.is_some() {
                // Refused: no `turn/completed` will come for it.
                fx.append.push(proto::TURN_ENDED_BY_SRV.to_string());
            }
            self.settle(fx);
        } else if self.quiet.remove(&id) {
            if error.is_some() {
                tracing::info!(error = %msg, "[msp] a command srv sent was refused");
            }
            fx.hide = true;
        }
    }

    /// `initialize` answered: check the schema, say `initialized`, open the
    /// session.
    fn on_initialized(&mut self, result: &Value, fx: &mut Effects) {
        let fingerprint = result.pointer("/schema/fingerprint").and_then(Value::as_str).unwrap_or("");
        if fingerprint != proto::MSP_SCHEMA_FINGERPRINT {
            tracing::warn!(fingerprint, expected = proto::MSP_SCHEMA_FINGERPRINT, "[msp] the host's schema differs from the one AgentMux was written against");
        }
        let granted = result
            .get("grantedCapabilities")
            .and_then(Value::as_array)
            .is_some_and(|caps| caps.iter().any(|c| c == "sessionMcp"));
        if !granted && !self.mcp_servers.is_empty() {
            tracing::warn!("[msp] the host didn't grant sessionMcp; starting the session without AgentMux's MCP servers");
            self.mcp_servers.clear();
        }
        fx.send.push(proto::notification("initialized"));
        let resume = self.resume.is_some();
        let line = self.session_request(resume);
        fx.send.push(line);
    }

    /// The handshake failed: no session until the agent restarts. A message
    /// that was waiting for it ends its turn.
    fn fail(&mut self, fx: &mut Effects) {
        self.failed = true;
        if self.pending.take().is_some() {
            fx.append.push(proto::TURN_ENDED_BY_SRV.to_string());
        }
        self.settle(fx);
    }

    /// Report the end of the last turn once nothing is left running.
    fn settle(&mut self, fx: &mut Effects) {
        if !self.turn_active() {
            self.ended_turns.clear();
            fx.turn_active = Some(false);
        }
    }
}

/// What `MspSession::prompt` decided.
#[derive(Debug, PartialEq)]
pub(super) enum PromptOutcome {
    /// Write `line`, the `turn/start` request `id`.
    Send { id: u64, line: String },
    /// The message waits for the session; write `reopen` first when given.
    Queued { reopen: Option<String> },
}
