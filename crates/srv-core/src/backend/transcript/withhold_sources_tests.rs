// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Each long-lived value srv puts in an agent's process or lays in its files is withheld from transcripts from the
//! moment srv makes or loads it: the value, made by the real code, goes through [`withhold`] and a stored transcript.

use std::sync::Arc;

use base64::Engine as _;
use serde_json::json;

use super::withhold::{self, MARK};
use crate::backend::storage::filestore::FileStore;
use crate::backend::storage::store::Store;

fn withheld(value: &str) {
    let line = format!("env says {value} and {}", base64::engine::general_purpose::STANDARD.encode(value));
    let shown = withhold::text(&line);
    assert!(shown.contains(MARK) && !shown.contains(value), "{shown}");
}

#[test]
fn an_agents_signing_keys_are_withheld_once_srv_hands_them_out() {
    let store = Store::open_in_memory().unwrap();
    let mut def = crate::backend::storage::agents::test_agent_def("withhold-uid-1", "withhold-slug", "claude", "agent", 1, "");
    def.slug = "withhold-slug".to_string();
    store.agent_def_insert(&mut def).unwrap();

    let jekt = base64::engine::general_purpose::STANDARD.encode(store.agent_jekt_key_ensure("withhold-slug").unwrap());
    withheld(&jekt);
    withheld(&store.agent_lan_key_ensure("withhold-slug").unwrap().private_key);
    withheld(&store.agent_lan_key_load("withhold-slug").unwrap().unwrap().private_key);
    let (uid_keys, _) = store.agent_uid_keys_ensure("withhold-uid-1").unwrap().expect("the agent has a row");
    withheld(&uid_keys.lan.private_key);
    withheld(&uid_keys.wan.private_key);
    withheld(&store.agent_wan_key_ensure("withhold-slug").unwrap().private_key);
    withheld(&store.agent_wan_key_load("withhold-slug").unwrap().unwrap().private_key);
}

#[test]
fn a_container_token_a_gateway_token_and_a_device_token_are_withheld_when_made() {
    withheld(&crate::backend::container::credential::token_for_block("withhold-block", None));
    withheld(&crate::backend::model_gateway::token::token_for("withhold-agent", "withhold-def", "withhold-route"));
    withheld(&crate::backend::viewer::pairing::new_token());
}

fn shown(value: &str) -> String {
    withhold::text(&format!("TOKEN={value}")).into_owned()
}

#[test]
fn a_container_token_is_withheld_until_its_block_is_revoked() {
    let token = crate::backend::container::credential::token_for_block("withhold-revoke-block", None);
    assert_eq!(shown(&token), format!("TOKEN={MARK}"));
    crate::backend::container::credential::revoke_block("withhold-revoke-block");
    assert_eq!(shown(&token), format!("TOKEN={token}"), "a revoked token no longer works, so it is no longer held");
    let next = crate::backend::container::credential::token_for_block("withhold-revoke-block", None);
    assert_ne!(next, token);
    assert_eq!(shown(&next), format!("TOKEN={MARK}"));
}

#[test]
fn a_gateway_token_is_withheld_until_it_is_replaced_or_ended() {
    use crate::backend::model_gateway::token;
    let first = token::token_for_account("withhold-gw-agent", "withhold-gw-def", "withhold-gw-route", "acct");
    assert_eq!(shown(&first), format!("TOKEN={MARK}"));
    let renewed = token::renew_for_account("withhold-gw-agent", "withhold-gw-def", "withhold-gw-route", "other", None);
    assert_ne!(first, renewed);
    assert_eq!(shown(&renewed), format!("TOKEN={MARK}"));
    assert_eq!(shown(&first), format!("TOKEN={first}"), "the token it replaced is dropped");
    token::revoke("withhold-gw-def", "withhold-gw-route");
    assert_eq!(shown(&renewed), format!("TOKEN={renewed}"), "a route that was changed ends its token, and its hold");
}

#[test]
fn a_devices_token_is_withheld_until_the_device_is_unpaired() {
    let store = Store::open_in_memory().unwrap();
    let token = crate::backend::viewer::pairing::new_token();
    store.viewer_device_insert("withhold-device", &crate::backend::viewer::pairing::token_hash(&token), "phone", "", 1).unwrap();
    assert_eq!(shown(&token), format!("TOKEN={MARK}"));
    store.viewer_device_delete("withhold-device").unwrap();
    assert_eq!(shown(&token), format!("TOKEN={token}"));
}

#[test]
fn a_stored_transcript_never_holds_a_token_an_agent_printed() {
    let token = crate::backend::container::credential::token_for_block("withhold-store-block", None);
    let own = Arc::new(FileStore::open_in_memory().unwrap());
    let broker = crate::backend::mps::Broker::new();
    let line = json!({ "type": "user", "message": { "role": "user", "content": [{ "type": "tool_result", "tool_use_id": "t1", "content": format!("TOKEN={token}") }] } });
    super::append::append_agent_output_line(
        &broker,
        "withhold-store-block",
        &line.to_string(),
        Some(&own),
        Some("agent:withhold-store:current"),
        super::origin_sig::WriterRuntime::Host,
    );
    let stored = String::from_utf8(own.read_file("withhold-store-block", "output").unwrap().unwrap_or_default()).unwrap();
    assert!(stored.contains(MARK) && !stored.contains(&token), "{stored}");
}

#[test]
fn only_the_secret_values_of_a_connector_config_are_withheld() {
    let config = json!({
        "command": "npx",
        "env": { "SERVICE_API_KEY": "connector-secret-4471-AbCdEf", "MODE": "production-mode" },
        "headers": { "Authorization": "Bearer connector-bearer-9920-GhIjKl", "Accept": "application/json" }
    });
    crate::backend::storage::mcp_servers::withhold_secrets_of(&config.to_string());
    for secret in ["connector-secret-4471-AbCdEf", "Bearer connector-bearer-9920-GhIjKl", "connector-bearer-9920-GhIjKl"] {
        assert!(!withhold::text(&format!("seen {secret}")).contains(secret), "{secret}");
    }
    for ordinary in ["production-mode", "application/json"] {
        assert_eq!(withhold::text(ordinary), ordinary, "ordinary text is left alone");
    }
}
