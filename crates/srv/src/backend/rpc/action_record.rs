// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The record of what each connection did: one line in the audit log for every
//! call of a state-changing websocket RPC, whether it was done or refused, saying
//! who made it ([`super::actor`]), what it acted on, and how it went.
//!
//! It is made in one place, the engine's dispatch, so a handler cannot forget it
//! and a new command is recorded until someone decides it is not
//! ([`super::action_commands`], with a test that every registered command is
//! decided). It names the actor, the command, the agent or pane it targets and
//! the identifier it acts on; it never holds a request's contents.

use serde_json::Value;

use super::actor::{actor_of, Actor};
use crate::backend::rpc_types::RpcContext;

/// What a call acted on, read from the request's identifying fields only.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Target {
    pub agent: Option<String>,
    pub block_id: Option<String>,
    pub subject: Option<String>,
}

/// One recorded call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionRecord {
    pub actor: Actor,
    pub command: String,
    pub target: Target,
    pub request_id: String,
    /// `None` when it was done, else why it was not.
    pub error: Option<String>,
}

/// The longest identifier kept.
const MAX_FIELD: usize = 200;

/// Fields that name an agent, a pane, or the thing acted on, with spellings folded
/// (`blockId`, `block_id` and `blockid` are one). Only these are read: contents, keys,
/// tokens, secrets and message text are not among them.
const AGENT_KEYS: &[&str] = &["agentid", "agentname", "agent", "agentslug", "slug"];
const BLOCK_KEYS: &[&str] = &["blockid", "block"];
const SUBJECT_KEYS: &[&str] = &[
    "id", "widgetid", "accountid", "definitionid", "providerid", "provider", "name", "templateid", "bundleid", "skillid",
    "droneid", "jobid", "groupid", "connname", "host", "path", "oldpath", "newpath", "pid", "signal",
];

fn fold(key: &str) -> String {
    key.chars().filter(|c| *c != '_' && *c != '-').collect::<String>().to_lowercase()
}

fn text_of(v: &Value) -> Option<String> {
    let s = match v {
        Value::String(s) => s.trim().to_string(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    (!s.is_empty()).then(|| s.chars().take(MAX_FIELD).collect())
}

/// What `data` (a request's top-level fields) names.
pub fn target_of(data: &Value) -> Target {
    let Some(map) = data.as_object() else { return Target::default() };
    let find = |keys: &[&str]| -> Option<String> {
        keys.iter().find_map(|want| map.iter().find(|(k, _)| fold(k) == *want).and_then(|(_, v)| text_of(v)))
    };
    let subjects: Vec<String> = SUBJECT_KEYS
        .iter()
        .filter_map(|want| map.iter().find(|(k, _)| fold(k) == *want).and_then(|(k, v)| text_of(v).map(|t| format!("{}={t}", fold(k)))))
        .collect();
    Target {
        agent: find(AGENT_KEYS),
        block_id: find(BLOCK_KEYS),
        subject: (!subjects.is_empty()).then(|| subjects.join(" ").chars().take(MAX_FIELD * 2).collect()),
    }
}

/// Commands that answer `{ "ok": false }` when they did nothing.
const ANSWERS_OK_FLAG: &[&str] = &["agent.kill-process", "agent.kill-tree"];

/// How a call went, from what the handler answered: a command that answers
/// `ok: false` did not do what was asked.
pub fn outcome_of_response(command: &str, response: Option<&Value>) -> Result<(), String> {
    let refused = ANSWERS_OK_FLAG.contains(&command)
        && response.and_then(|r| r.get("ok")).and_then(Value::as_bool) == Some(false);
    if refused { Err("not done: the command answered ok=false".to_string()) } else { Ok(()) }
}

/// Records a finished call of `command`, if it is one that changes state.
pub fn after_call(command: &str, data: &Value, ctx: &RpcContext, request_id: &str, outcome: &Result<(), String>) {
    let actor = actor_of(ctx);
    if !super::action_commands::is_recorded(command, actor == Actor::Operator) || super::action_commands::is_a_read(command, data) {
        return;
    }
    let record = ActionRecord {
        actor,
        command: command.to_string(),
        target: target_of(data),
        request_id: request_id.to_string(),
        error: outcome.as_ref().err().map(|e| e.chars().take(MAX_FIELD).collect()),
    };
    crate::backend::reactive::get_global_handler().audit_action(&record);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_target_is_read_from_identifying_fields_in_any_spelling() {
        let t = target_of(&json!({"blockId": "b1", "pid": 42, "agent_id": "opaz", "signal": "SIGKILL"}));
        assert_eq!((t.agent.as_deref(), t.block_id.as_deref()), (Some("opaz"), Some("b1")));
        assert_eq!(t.subject.as_deref(), Some("pid=42 signal=SIGKILL"));
        let t = target_of(&json!({"block_id": "b2", "id": "acme.widget"}));
        assert_eq!((t.block_id.as_deref(), t.subject.as_deref()), (Some("b2"), Some("id=acme.widget")));
    }

    #[test]
    fn a_kill_that_answers_not_ok_is_recorded_as_not_done() {
        let no = json!({"ok": false});
        assert!(outcome_of_response("agent.kill-tree", Some(&no)).is_err());
        assert!(outcome_of_response("agent.kill-process", Some(&no)).is_err());
        assert!(outcome_of_response("agent.kill-tree", Some(&json!({"ok": true}))).is_ok());
        assert!(outcome_of_response("agent.kill-tree", None).is_ok());
        assert!(outcome_of_response("agent.send", Some(&no)).is_ok());
    }

    #[test]
    fn contents_keys_and_tokens_are_never_read() {
        let t = target_of(&json!({
            "id": "acct-1", "clientSecret": "s3cret", "token": "t0ken", "apiKey": "k", "password": "p",
            "content": "file body", "message": "hello", "data": "x", "env": {"A": "b"}, "value": "v"
        }));
        let all = format!("{t:?}");
        for secret in ["s3cret", "t0ken", "file body", "hello", "\"k\"", "\"p\""] {
            assert!(!all.contains(secret), "{secret} leaked into {all}");
        }
        assert_eq!(t.subject.as_deref(), Some("id=acct-1"));
    }

    #[test]
    fn a_long_or_empty_field_is_cut_or_dropped() {
        let t = target_of(&json!({"id": "x".repeat(900), "agentId": "  ", "name": 7}));
        assert_eq!(t.agent, None);
        assert!(t.subject.unwrap().len() <= MAX_FIELD * 2);
        assert_eq!(target_of(&Value::Null), Target::default());
        assert_eq!(target_of(&json!("a string")), Target::default());
    }
}
