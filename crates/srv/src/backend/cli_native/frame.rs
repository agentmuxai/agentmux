// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The stream-json frames that carry a CLI-native message, read as typed values.
//! The shapes are the pinned CLI's (`providers/claude.rs` `pinned_version`);
//! `tests` holds frames captured from it.

use serde_json::Value;

/// A message from another session, as the receiving CLI reported it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerFrame {
    /// The frame's uuid; the `command_uuid` of the lifecycle lines about it.
    pub uuid: String,
    /// The CLI's id for the message; the sender's tool result carries the same one.
    pub msg_id: String,
    /// The pid the CLI verified for the sender's connection.
    pub pid: Option<u32>,
    pub proc_start: Option<String>,
    /// What the sender called itself. A claim.
    pub claimed_name: Option<String>,
    pub from_mode: Option<String>,
    pub body: String,
}

/// A `user` frame the CLI replays under `--replay-user-messages`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Replay {
    Peer(PeerFrame),
    /// The human's own prompt or any other replayed turn.
    Other,
}

/// `Some` for a replayed user frame, `None` for any other frame.
pub fn replay(v: &Value) -> Option<Replay> {
    if v.get("type")?.as_str()? != "user" || v.get("isReplay")?.as_bool()? != true {
        return None;
    }
    let origin = v.get("origin").filter(|o| o.get("kind").and_then(Value::as_str) == Some("peer"));
    let (Some(origin), Some(uuid)) = (origin, v.get("uuid").and_then(Value::as_str)) else { return Some(Replay::Other) };
    let text = |k: &str| origin.get(k).and_then(Value::as_str).map(str::to_string);
    let Some(msg_id) = text("msg_id") else { return Some(Replay::Other) };
    Some(Replay::Peer(PeerFrame {
        uuid: uuid.to_string(),
        msg_id,
        pid: origin.get("verifiedPeerPid").and_then(Value::as_u64).and_then(|p| u32::try_from(p).ok()),
        proc_start: origin.get("verifiedPeerProcStart").map(|p| p.as_str().map(str::to_string).unwrap_or_else(|| p.to_string())),
        claimed_name: text("name"),
        from_mode: text("fromMode"),
        body: text("body").unwrap_or_default(),
    }))
}

/// The receiving CLI kept a message from reaching its model, as in a different
/// permission mode from the sender's. It says nothing of who sent it but a name the
/// sender gave, and nothing of what it said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerHold {
    /// The held message's uuid (not the sender's `msg_id`).
    pub message_uuid: String,
    pub from_name: Option<String>,
    pub cause: String,
}

/// `system/peer_message_hold` frame.
pub fn peer_hold(v: &Value) -> Option<PeerHold> {
    if v.get("type")?.as_str()? != "system" || v.get("subtype")?.as_str()? != "peer_message_hold" || v.get("state")?.as_str()? != "held" {
        return None;
    }
    Some(PeerHold {
        message_uuid: v.get("message_uuid")?.as_str()?.to_string(),
        from_name: v.get("from_name").and_then(Value::as_str).map(str::to_string),
        cause: v.get("cause").and_then(Value::as_str).unwrap_or("unknown").to_string(),
    })
}

/// What the CLI says about a message turn it began for its own reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lifecycle {
    Started,
    Completed,
}

/// `command_lifecycle` frame: (command uuid, state).
pub fn lifecycle(v: &Value) -> Option<(&str, Lifecycle)> {
    if v.get("type")?.as_str()? != "command_lifecycle" {
        return None;
    }
    let state = match v.get("state")?.as_str()? {
        "started" => Lifecycle::Started,
        "completed" => Lifecycle::Completed,
        _ => return None,
    };
    Some((v.get("command_uuid")?.as_str()?, state))
}

/// The CLI's own `SendMessage` tool called by this agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendCall {
    pub tool_use_id: String,
    pub to: String,
    pub text: String,
}

/// The `SendMessage` calls in an assistant frame. Only the CLI's own tool, by its
/// bare name: `mcp__<server>__SendMessage` tools are other tools.
pub fn send_calls(v: &Value) -> Vec<SendCall> {
    if v.get("type").and_then(Value::as_str) != Some("assistant") {
        return Vec::new();
    }
    let blocks = v.pointer("/message/content").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    blocks
        .iter()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_use") && b.get("name").and_then(Value::as_str) == Some("SendMessage"))
        .filter_map(|b| {
            let input = b.get("input")?;
            Some(SendCall {
                tool_use_id: b.get("id")?.as_str()?.to_string(),
                to: input.get("to")?.as_str()?.to_string(),
                text: input.get("message").and_then(Value::as_str).unwrap_or_default().to_string(),
            })
        })
        .collect()
}

/// What the CLI answered a `SendMessage` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendResult {
    pub tool_use_id: String,
    /// The CLI accepted it into the other session's inbox.
    pub accepted: bool,
    pub msg_id: Option<String>,
}

/// The tool results in a non-replayed user frame, read as `SendMessage` results.
/// A result that is not the tool's JSON answer reads as not accepted.
pub fn send_results(v: &Value) -> Vec<SendResult> {
    if v.get("type").and_then(Value::as_str) != Some("user") {
        return Vec::new();
    }
    let blocks = v.pointer("/message/content").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    blocks
        .iter()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_result"))
        .filter_map(|b| {
            let id = b.get("tool_use_id")?.as_str()?.to_string();
            let content = b.get("content")?;
            let text = match content {
                Value::String(s) => s.clone(),
                Value::Array(parts) => parts.iter().filter_map(|p| p.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join(""),
                _ => String::new(),
            };
            let answer: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            Some(SendResult {
                tool_use_id: id,
                accepted: answer.get("success").and_then(Value::as_bool) == Some(true),
                msg_id: answer.get("msg_id").and_then(Value::as_str).map(str::to_string),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // Captured from claude 2.1.288 with --replay-user-messages (the receiver, mid-turn).
    fn peer_frame() -> Value {
        json!({"type":"user","message":{"role":"user","content":"<cross-session-message from=\"uds:/tmp/cc-socks/4120954.sock\" from-name=\"send-36\" from-mode=\"bypass\">\nXS-TEST2 mid-turn hello.\n</cross-session-message>"},
            "session_id":"s","parent_tool_use_id":null,"uuid":"df2a326a-967c-49d0-87cc-f80823165a7f","isReplay":true,"isSynthetic":true,
            "origin":{"kind":"peer","from":"uds:/tmp/cc-socks/4120954.sock","verifiedPeerPid":4120954,"verifiedPeerProcStart":"215691469",
                "msg_id":"58220516-3da1-4679-8d7c-5040d05f1cfe","name":"send-36","fromMode":"bypass","body":"XS-TEST2 mid-turn hello."}})
    }

    #[test]
    fn a_peer_frame_is_read_from_its_typed_origin() {
        let Some(Replay::Peer(p)) = replay(&peer_frame()) else { panic!("not a peer frame") };
        assert_eq!(p.msg_id, "58220516-3da1-4679-8d7c-5040d05f1cfe");
        assert_eq!(p.uuid, "df2a326a-967c-49d0-87cc-f80823165a7f");
        assert_eq!((p.pid, p.proc_start.as_deref()), (Some(4120954), Some("215691469")));
        assert_eq!((p.claimed_name.as_deref(), p.from_mode.as_deref()), (Some("send-36"), Some("bypass")));
        assert_eq!(p.body, "XS-TEST2 mid-turn hello.");
    }

    #[test]
    fn the_text_of_the_message_never_makes_a_peer_frame() {
        // The wrapper text in a replayed frame with no typed origin is not a peer message.
        let mut f = peer_frame();
        f.as_object_mut().unwrap().remove("origin");
        assert_eq!(replay(&f), Some(Replay::Other));
        let mut f = peer_frame();
        f["origin"]["kind"] = json!("task-notification");
        assert_eq!(replay(&f), Some(Replay::Other));
    }

    #[test]
    fn the_humans_replayed_prompt_and_ordinary_frames() {
        let typed = json!({"type":"user","message":{"role":"user","content":"hello"},"uuid":"u","isReplay":true});
        assert_eq!(replay(&typed), Some(Replay::Other));
        let tool_result = json!({"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t","content":"x"}]},"uuid":"u"});
        assert_eq!(replay(&tool_result), None);
        assert_eq!(replay(&json!({"type":"assistant","isReplay":true})), None);
    }

    #[test]
    fn a_held_message_is_read_from_the_clis_hold_line() {
        // Captured from claude 2.1.288: a bypass-mode session messaged a default-mode one.
        let held = json!({"type":"system","subtype":"peer_message_hold","state":"held","message_uuid":"25198c47","lane":"socket",
            "from":"uds:/tmp/cc-socks/54771.sock","from_name":"send-02","cause":"mode-mismatch"});
        assert_eq!(peer_hold(&held), Some(PeerHold { message_uuid: "25198c47".into(), from_name: Some("send-02".into()), cause: "mode-mismatch".into() }));
        assert_eq!(peer_hold(&json!({"type":"system","subtype":"init"})), None);
    }

    #[test]
    fn lifecycle_lines_carry_the_message_uuid() {
        let started = json!({"type":"command_lifecycle","command_uuid":"df2a","state":"started","uuid":"x","session_id":"s"});
        assert_eq!(lifecycle(&started), Some(("df2a", Lifecycle::Started)));
        let done = json!({"type":"command_lifecycle","command_uuid":"df2a","state":"completed"});
        assert_eq!(lifecycle(&done), Some(("df2a", Lifecycle::Completed)));
        assert_eq!(lifecycle(&json!({"type":"command_lifecycle","command_uuid":"d","state":"queued"})), None);
    }

    #[test]
    fn the_senders_call_and_the_clis_answer() {
        let call = json!({"type":"assistant","message":{"content":[
            {"type":"text","text":"sending"},
            {"type":"tool_use","id":"toolu_1","name":"SendMessage","input":{"to":"recv-82","message":"XS-TEST2 hello","type":"message"}},
            {"type":"tool_use","id":"toolu_2","name":"mcp__agentmux__SendMessage","input":{"to":"x","message":"not the CLI's tool"}}]}});
        assert_eq!(send_calls(&call), vec![SendCall { tool_use_id: "toolu_1".into(), to: "recv-82".into(), text: "XS-TEST2 hello".into() }]);

        let answer = json!({"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_1",
            "content":[{"type":"text","text":"{\"success\":true,\"message\":\"sent\",\"msg_id\":\"58220516-3da1-4679-8d7c-5040d05f1cfe\"}"}]}]}});
        assert_eq!(
            send_results(&answer),
            vec![SendResult { tool_use_id: "toolu_1".into(), accepted: true, msg_id: Some("58220516-3da1-4679-8d7c-5040d05f1cfe".into()) }]
        );
        let refused = json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"Error: no such session","is_error":true}]}});
        assert_eq!(send_results(&refused), vec![SendResult { tool_use_id: "toolu_1".into(), accepted: false, msg_id: None }]);
    }
}
