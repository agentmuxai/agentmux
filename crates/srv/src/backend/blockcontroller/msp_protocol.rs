// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The Muse Session Protocol (MSP) messages AgentMux sends to `muse serve`,
//! and the MCP servers it passes in them. Checked against Muse Code
//! 1.4.7-R6101.1 (`muse schema generate-ts`), SPEC_NINE_NEW_HARNESSES_2026_10_14.md §7.7.

use serde_json::{json, Map, Value};

/// The stable-surface fingerprint of the MSP schema this client was written
/// against, as `initialize` reports it in `schema.fingerprint`. Another one is
/// logged, not refused: MSP is pre-1.0, and additions are additive.
pub(super) const MSP_SCHEMA_FINGERPRINT: &str =
    "sha256:6de32db08bbbe0deb478d2bfcaba001e9fb490986bc80db3aceb4d996abf7df9";

/// `initialize`. `clientInfo.name` must match `[a-z0-9_]+`. The one capability
/// asked for lets `session/start` carry MCP servers; `userInputDialogs: false`
/// tells the host the pane can't show its question dialogs, so it doesn't ask.
pub(super) fn initialize_params() -> Value {
    json!({
        "clientInfo": { "name": "agentmux", "title": "AgentMux", "version": env!("CARGO_PKG_VERSION") },
        "capabilities": { "requestedCapabilities": ["sessionMcp"], "userInputDialogs": false },
    })
}

/// A client-minted UUIDv7, which every MSP command carries as its `commandId`.
pub(super) fn command_id() -> String {
    let random = uuid::Uuid::new_v4().into_bytes();
    let mut tail = [0u8; 10];
    tail.copy_from_slice(&random[6..]);
    uuid::Builder::from_unix_timestamp_millis(agentmux_common::time::now_ms_u64(), &tail)
        .into_uuid()
        .to_string()
}

/// `session/start` in `cwd` with `mcp_servers`, or `session/resume` of
/// `resume`. A resume asks for no history: the pane already shows it.
pub(super) fn session_request(resume: Option<&str>, cwd: &str, mcp_servers: &Map<String, Value>) -> (&'static str, Value) {
    let mut params = json!({ "commandId": command_id() });
    if !mcp_servers.is_empty() {
        params["config"] = json!({ "mcpServers": mcp_servers });
    }
    match resume {
        Some(sid) => {
            params["sessionId"] = json!(sid);
            params["excludeItems"] = json!(true);
            ("session/resume", params)
        }
        None => {
            params["workspaceRoot"] = json!(cwd);
            ("session/start", params)
        }
    }
}

/// `turn/start` with the user's text. A message sent while a turn runs joins
/// that turn (`steer`), as it does in Claude's pane, instead of waiting in a
/// queue behind it.
pub(super) fn turn_start_params(session_id: &str, text: &str) -> Value {
    json!({
        "sessionId": session_id,
        "commandId": command_id(),
        "input": [{ "type": "text", "text": text }],
        "ifBusy": "steer",
    })
}

/// `turn/interrupt` of the session's running turn.
pub(super) fn turn_interrupt_params(session_id: &str) -> Value {
    json!({ "sessionId": session_id, "commandId": command_id() })
}

/// The choice AgentMux makes for an approval, as it does for every harness
/// (ACP's `allow_always`): approved for the session, else approved once. A
/// choice that writes a lasting rule into the user's own config is never
/// picked. With no approving choice, the denying one, so the turn goes on.
pub(super) fn approval_choice(params: &Value) -> Option<String> {
    let choices = params.get("availableChoices")?.as_array()?;
    let with = |decision: &str| {
        choices
            .iter()
            .find(|c| c.get("decision").and_then(Value::as_str) == Some(decision))
            .and_then(|c| c.get("choiceId").and_then(Value::as_str))
    };
    with("approvedForSession")
        .or_else(|| with("approved"))
        .or_else(|| with("denied"))
        .map(str::to_string)
}

/// `approval/decide` for an `approval/request`'s params, or `None` when it
/// names no approval or offers no usable choice.
pub(super) fn approval_decide_params(params: &Value) -> Option<Value> {
    Some(json!({
        "sessionId": params.get("sessionId")?.as_str()?,
        "approvalId": params.get("approvalId")?.as_str()?,
        "choiceId": approval_choice(params)?,
        "requirementId": params.get("currentRequirementId")?.clone(),
        "commandId": command_id(),
    }))
}

/// `userInput/cancel` for a `userInput/request`: the pane can't show a
/// question, and an unanswered one would hold the turn.
pub(super) fn user_input_cancel_params(params: &Value) -> Option<Value> {
    Some(json!({
        "sessionId": params.get("sessionId")?.as_str()?,
        "userInputId": params.get("userInputId")?.as_str()?,
        "commandId": command_id(),
        "reason": "AgentMux can't show this question",
    }))
}

/// A JSON-RPC request line.
pub(super) fn request(id: u64, method: &str, params: Value) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }).to_string()
}

/// A JSON-RPC notification line.
pub(super) fn notification(method: &str) -> String {
    json!({ "jsonrpc": "2.0", "method": method }).to_string()
}

/// The reply to a request from the host: the empty receipt for the two it
/// defines (the decision travels as a command), "method not found" for
/// anything else, so the host never waits on us.
pub(super) fn reply_to_host_request(id: &Value, method: &str) -> String {
    if method == "approval/request" || method == "userInput/request" {
        return json!({ "jsonrpc": "2.0", "id": id, "result": {} }).to_string();
    }
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": -32601, "message": format!("AgentMux does not offer {method}") },
    })
    .to_string()
}

/// Written to the output after a turn ended without the host saying so (its
/// `turn/start` was refused, the session never opened, the process exited):
/// the pane ends a turn on `turn/completed`.
pub(super) const TURN_ENDED_BY_SRV: &str =
    r#"{"jsonrpc":"2.0","method":"turn/completed","params":{"turnId":"","terminal":"failed"}}"#;

/// An error line for the pane when `muse serve` exited while it had work:
/// its exit code and the last of what it wrote to stderr, which is where it
/// says why (a broken config file, a missing credential store).
pub(super) fn process_exit_error(exit_code: i32, stderr_tail: &[String]) -> String {
    let mut message = format!("Muse Code exited (code {exit_code})");
    let tail = stderr_tail.join("\n");
    if !tail.trim().is_empty() {
        message.push_str(": ");
        message.push_str(tail.trim());
    }
    json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32603, "message": message } }).to_string()
}

/// `.mcp.json`'s servers as `session/start`'s `config.mcpServers`, built from
/// the ACP list (`acp_mcp`, which expands placeholders and resolves commands).
/// MSP takes stdio and streamable-HTTP servers, keyed by name. Each is
/// `optional`: one that fails to start leaves the agent without its tools
/// rather than failing every turn.
pub(super) fn msp_mcp_servers(
    mcp_json: &str,
    resolve: impl Fn(&str) -> Option<String>,
    lookup: impl Fn(&str) -> Option<String>,
) -> Map<String, Value> {
    let pairs = |v: &Value| -> Map<String, Value> {
        v.as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|p| Some((p.get("name")?.as_str()?.to_string(), p.get("value")?.clone())))
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut out = Map::new();
    for s in super::acp_mcp::acp_mcp_servers(mcp_json, true, false, resolve, lookup) {
        let Some(name) = s.get("name").and_then(Value::as_str) else { continue };
        let server = if s.get("type").and_then(Value::as_str) == Some("http") {
            json!({ "transport": "streamableHttp", "url": s["url"], "headers": pairs(&s["headers"]), "mode": "optional" })
        } else {
            json!({ "transport": "stdio", "command": s["command"], "args": s["args"], "env": pairs(&s["env"]), "mode": "optional" })
        };
        out.insert(name.to_string(), server);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize_names_the_client_as_msp_requires_and_asks_for_session_mcp() {
        let p = initialize_params();
        let name = p["clientInfo"]["name"].as_str().unwrap();
        assert!(name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'), "{name}");
        assert_eq!(p["capabilities"]["requestedCapabilities"], json!(["sessionMcp"]));
        assert_eq!(p["capabilities"]["userInputDialogs"], false);
        assert!(p["capabilities"].get("experimentalApi").is_none(), "the stable surface only");
    }

    #[test]
    fn command_ids_are_distinct_uuid_v7s() {
        let (a, b) = (command_id(), command_id());
        assert_ne!(a, b);
        for id in [a, b] {
            let u = uuid::Uuid::parse_str(&id).unwrap();
            assert_eq!(u.get_version_num(), 7, "{id}");
        }
    }

    #[test]
    fn a_session_starts_in_the_cwd_or_resumes_without_history() {
        let mut mcp = Map::new();
        mcp.insert("agentmux".into(), json!({"transport": "stdio", "command": "x"}));
        let (method, p) = session_request(None, "C:/agents/a", &mcp);
        assert_eq!(method, "session/start");
        assert_eq!(p["workspaceRoot"], "C:/agents/a");
        assert_eq!(p["config"]["mcpServers"]["agentmux"]["command"], "x");
        assert!(p["commandId"].is_string());
        let (method, p) = session_request(Some("s1"), "C:/agents/a", &Map::new());
        assert_eq!(method, "session/resume");
        assert_eq!(p["sessionId"], "s1");
        assert_eq!(p["excludeItems"], true);
        assert!(p.get("config").is_none(), "no servers, no config");
        assert!(p.get("workspaceRoot").is_none());
    }

    #[test]
    fn a_turn_is_text_input_that_steers_a_running_turn() {
        let p = turn_start_params("s1", "hi");
        assert_eq!(p["sessionId"], "s1");
        assert_eq!(p["input"], json!([{"type": "text", "text": "hi"}]));
        assert_eq!(p["ifBusy"], "steer");
    }

    fn approval(choices: Value) -> Value {
        json!({
            "approvalId": "a1", "sessionId": "s1", "currentRequirementId": {"stage": 1},
            "availableChoices": choices,
        })
    }

    #[test]
    fn approvals_prefer_the_session_then_once_and_never_a_lasting_rule() {
        let all = approval(json!([
            {"choiceId": "rule", "decision": "approvedPolicyAmendment"},
            {"choiceId": "once", "decision": "approved"},
            {"choiceId": "sess", "decision": "approvedForSession"},
            {"choiceId": "no", "decision": "denied"},
        ]));
        assert_eq!(approval_choice(&all).as_deref(), Some("sess"));
        let once = approval(json!([{"choiceId": "rule", "decision": "approvedPolicyAmendment"}, {"choiceId": "once", "decision": "approved"}]));
        assert_eq!(approval_choice(&once).as_deref(), Some("once"));
        let deny = approval(json!([{"choiceId": "rule", "decision": "approvedPolicyAmendment"}, {"choiceId": "no", "decision": "denied"}]));
        assert_eq!(approval_choice(&deny).as_deref(), Some("no"));
        assert_eq!(approval_choice(&approval(json!([]))), None);
    }

    #[test]
    fn a_decision_names_the_approval_and_its_current_stage() {
        let p = approval_decide_params(&approval(json!([{"choiceId": "once", "decision": "approved"}]))).unwrap();
        assert_eq!(p["approvalId"], "a1");
        assert_eq!(p["sessionId"], "s1");
        assert_eq!(p["choiceId"], "once");
        assert_eq!(p["requirementId"], json!({"stage": 1}));
        assert!(p["commandId"].is_string());
        assert!(approval_decide_params(&json!({"approvalId": "a1"})).is_none());
    }

    #[test]
    fn host_requests_get_a_receipt_or_method_not_found() {
        let r: Value = serde_json::from_str(&reply_to_host_request(&json!(7), "approval/request")).unwrap();
        assert_eq!(r, json!({"jsonrpc": "2.0", "id": 7, "result": {}}));
        let r: Value = serde_json::from_str(&reply_to_host_request(&json!("x"), "fs/read")).unwrap();
        assert_eq!(r["id"], "x");
        assert_eq!(r["error"]["code"], -32601);
    }

    #[test]
    fn a_question_is_declined() {
        let p = user_input_cancel_params(&json!({"sessionId": "s1", "userInputId": "q1"})).unwrap();
        assert_eq!(p["userInputId"], "q1");
        assert!(user_input_cancel_params(&json!({"sessionId": "s1"})).is_none());
    }

    #[test]
    fn an_exit_error_carries_the_code_and_stderr() {
        let line: Value = serde_json::from_str(&process_exit_error(3, &["malformed auth file".to_string()])).unwrap();
        assert_eq!(line["error"]["message"], "Muse Code exited (code 3): malformed auth file");
        assert!(line["id"].is_null());
        let line: Value = serde_json::from_str(&process_exit_error(1, &[])).unwrap();
        assert_eq!(line["error"]["message"], "Muse Code exited (code 1)");
    }

    #[test]
    fn mcp_servers_are_keyed_by_name_with_env_and_headers_as_objects() {
        let raw = r#"{"mcpServers":{
            "agentmux":{"type":"stdio","command":"agentmux-mcp","args":[],"env":{"AGENTMUX_AGENT_ID":"aria"}},
            "docs":{"type":"http","url":"https://example.com/mcp","headers":{"Authorization":"Bearer ${T}"}},
            "feed":{"type":"sse","url":"https://example.com/sse"}
        }}"#;
        let resolve = |c: &str| (c == "agentmux-mcp").then(|| "C:/AgentMux/agentmux-mcp.exe".to_string());
        let lookup = |k: &str| (k == "T").then(|| "t0k".to_string());
        let s = msp_mcp_servers(raw, resolve, lookup);
        assert_eq!(
            s["agentmux"],
            json!({"transport": "stdio", "command": "C:/AgentMux/agentmux-mcp.exe", "args": [],
                   "env": {"AGENTMUX_AGENT_ID": "aria"}, "mode": "optional"})
        );
        assert_eq!(
            s["docs"],
            json!({"transport": "streamableHttp", "url": "https://example.com/mcp",
                   "headers": {"Authorization": "Bearer t0k"}, "mode": "optional"})
        );
        assert!(s.get("feed").is_none(), "MSP has no SSE transport");
        assert!(msp_mcp_servers("not json", |_| None, |_| None).is_empty());
    }
}
