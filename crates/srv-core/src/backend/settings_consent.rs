// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The settings that waive or widen what a person has agreed to: which programs may run, who may be reached, what is
//! shared with other devices, and how freely agents start agents. An agent may change any other setting, recorded as
//! that agent; these it never changes, because a change would be counted as the person's own choice. Only the
//! AgentMux window, proven through its host, passes (`RpcContext::operator_only`).

use serde_json::{Map, Value};

use super::rpc_types::RpcContext;
use super::supervisor::settings::KEYS as SUPERVISOR_KEYS;

/// Every setting that is the person's alone to change, besides the Supervisor's own keys.
pub const PERSON_ONLY: &[&str] = &[
    "agent:maystartagents",
    "agents:start_per_minute",
    "desktop:enabled",
    "desktop:allowlist",
    "desktop:blocklist",
    "desktop:grantminutes",
    "network:lan_discovery",
    "network:remote_ops",
    "term:localshellpath",
    "term:localshellopts",
    "cmd:env",
    "conn:helper",
    "audit:sharewithpaired",
    "tower:sharewithpaired",
    "viewer:shareterminals",
    "swarm:sharesummaries",
    "messaging:telegram:allowed_chats",
    "messaging:discord:token",
    "messaging:telegram:token",
    "messaging:slack:bot_token",
    "messaging:slack:app_token",
    "messaging:whatsapp:access_token",
    "messaging:whatsapp:webhook_verify_token",
];

/// Is `key` one only the person changes.
pub fn is_person_only(key: &str) -> bool {
    PERSON_ONLY.contains(&key) || SUPERVISOR_KEYS.contains(&key)
}

/// The keys of a settings write that only the person changes, in the order given.
pub fn person_only_keys(keys: &Map<String, Value>) -> Vec<&str> {
    keys.keys().map(String::as_str).filter(|k| is_person_only(k)).collect()
}

/// `Err` unless the connection is the person's window when the write sets a key only the person changes.
pub fn refuse_unless_person(ctx: &RpcContext, keys: &Map<String, Value>) -> Result<(), String> {
    let held = person_only_keys(keys);
    if held.is_empty() {
        return Ok(());
    }
    ctx.operator_only(&format!("only the person changes {}", held.join(", ")))
}

/// The names of the keys a settings write sets (never their values), for the record of who changed what.
pub fn key_names(keys: &Map<String, Value>) -> String {
    keys.keys().map(String::as_str).collect::<Vec<_>>().join(",")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_listed_key_is_a_setting_the_schema_knows() {
        let schema: Value = serde_json::from_str(include_str!("../../../../schema/settings.json")).unwrap();
        let known = |key: &str| schema.to_string().contains(&format!("\"{key}\""));
        for key in PERSON_ONLY.iter().chain(SUPERVISOR_KEYS.iter()) {
            assert!(known(key), "{key} is not in schema/settings.json: a renamed setting would drop out of the list unnoticed");
        }
    }

    #[test]
    fn the_person_only_keys_of_a_write_are_picked_out_and_the_rest_are_left() {
        let write = json!({"term:fontsize": 12, "agent:maystartagents": true, "supervisor:enabled": true, "app:startatlogin": false});
        let write = write.as_object().unwrap();
        let mut found = person_only_keys(write);
        found.sort_unstable();
        assert_eq!(found, vec!["agent:maystartagents", "supervisor:enabled"]);
        assert!(person_only_keys(json!({"term:fontsize": 12}).as_object().unwrap()).is_empty());
    }

    #[test]
    fn only_the_window_passes_a_write_that_sets_a_key_only_the_person_changes() {
        let write = json!({"term:fontsize": 12, "agent:maystartagents": true});
        let write = write.as_object().unwrap();
        let agent = RpcContext { agent_id: "korp".into(), operator_proven: true, ..Default::default() };
        let error = refuse_unless_person(&agent, write).unwrap_err();
        assert!(error.starts_with("FORBIDDEN: only the person changes agent:maystartagents"), "{error}");
        assert!(!error.contains("term:fontsize"), "{error}");
        assert!(refuse_unless_person(&RpcContext::default(), write).unwrap_err().contains("isn't the AgentMux window"));
        assert!(refuse_unless_person(&RpcContext::operator_for_tests(), write).is_ok());
    }

    #[test]
    fn an_agent_still_changes_any_other_setting() {
        let agent = RpcContext { agent_id: "korp".into(), ..Default::default() };
        assert!(refuse_unless_person(&agent, json!({"term:fontsize": 12, "app:startatlogin": false}).as_object().unwrap()).is_ok());
    }

    #[test]
    fn the_record_names_keys_and_never_values() {
        let write = json!({"messaging:telegram:token": "s3cret-value", "term:fontsize": 12});
        let names = key_names(write.as_object().unwrap());
        assert!(names.contains("messaging:telegram:token") && names.contains("term:fontsize"));
        assert!(!names.contains("s3cret-value") && !names.contains("12"));
    }
}
