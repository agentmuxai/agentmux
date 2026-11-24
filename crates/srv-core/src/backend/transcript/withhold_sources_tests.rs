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

fn file_store() {
    static DIR: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    let dir = DIR.get_or_init(|| tempfile::tempdir().unwrap());
    let _ = crate::identity::secret_store::use_file_store(dir.path().join("secrets"));
    assert_eq!(crate::identity::secret_store::backend_name(), "file", "these tests never touch the OS keychain");
}

#[test]
fn a_keychain_secret_is_withheld_once_srv_writes_it_and_the_account_fields_around_it_are_not() {
    file_store();
    let (access, refresh, client) = ("keychain-access-7731-QwErTy", "keychain-refresh-7732-AsDfGh", "keychain-client-7733-ZxCvBn");
    let blob = json!({ "access_token": access, "refresh_token": refresh, "expires_at": 1, "client_secret": client, "email": "person@example.com" });
    crate::identity::secret_store::put("withhold-account-blob", &blob.to_string()).unwrap();
    for secret in [access, refresh, client] {
        withheld(secret);
    }
    assert_eq!(withhold::text("person@example.com"), "person@example.com", "a field that is not a credential is left alone");

    let pat = "keychain-pat-7734-PoIuYt";
    crate::identity::secret_store::put("withhold-account-pat", pat).unwrap();
    withheld(pat);
}

#[test]
fn a_keychain_secret_is_withheld_when_srv_reads_it() {
    file_store();
    let secret = "keychain-read-7741-MnBvCx";
    crate::identity::secret_store::put("withhold-account-read", secret).unwrap();
    // As if this process had not written it: another srv, or an earlier run, did.
    withhold::release_all("keychain:withhold-account-read");
    assert!(shown(secret).contains(secret), "not known until it is read");
    let read = crate::identity::secret_store::get_optional("withhold-account-read").unwrap().unwrap();
    assert_eq!(read.as_str(), secret);
    withheld(secret);
}

#[test]
fn the_cloud_sign_in_is_withheld_whole() {
    let (access, refresh, id) = ("cloud-access-7751-LkJhGf", "cloud-refresh-7752-DsAqWe", "cloud-id-7753-RtYuIo");
    super::withhold_sources::cloud_sign_in(access, refresh, id, agentmux_common::time::now_secs() + 3600);
    for secret in [access, refresh, id] {
        withheld(secret);
    }
}

#[test]
fn an_agents_relay_credential_is_withheld_when_saved_and_when_loaded() {
    let store = Store::open_in_memory().unwrap();
    let (secret, token) = ("relay-secret-7761-ZaXsCd", "relay-access-7762-VfBgNh");
    let agent = "withhold-relay-agent";
    store.agent_credential_save(agent, "client-id-1", secret, "https://example.invalid/token").unwrap();
    withheld(secret);
    store.agent_credential_save_token(agent, token, agentmux_common::time::now_secs() + 3600).unwrap();
    withheld(token);

    // As if this process had not saved them: an earlier run did.
    withhold::release_all(&format!("relay-credential:{agent}:secret"));
    withhold::release_all(&format!("relay-credential:{agent}:access"));
    withhold::release_prefix(&format!("relay-credential:{agent}:"));
    assert!(shown(secret).contains(secret) && shown(token).contains(token), "not known until it is loaded");
    store.agent_credential_load(agent).unwrap().unwrap();
    withheld(secret);
    withheld(token);
}

#[test]
fn a_bridge_token_a_relay_token_and_a_key_written_in_an_agents_environment_are_withheld() {
    super::withhold_sources::bridge_token("discord", "bridge-token-7771-QaWsEd");
    withheld("bridge-token-7771-QaWsEd");
    super::withhold_sources::relay_token("relay-token-7772-RfTgYh");
    withheld("relay-token-7772-RfTgYh");

    super::withhold_sources::env_credential("withhold-env-agent", "GITHUB_TOKEN", "env-token-7773-UjIkOl");
    withheld("env-token-7773-UjIkOl");
    for (name, value) in [
        ("SERVICE_API_KEY", "vault:entry-7774-abcdef"),
        ("OPENAI_API_KEY", "ollama"),
        ("EDITOR_NAME", "env-plain-7775-PlMkNj"),
        ("EMPTY_TOKEN", ""),
    ] {
        super::withhold_sources::env_credential("withhold-env-agent", name, value);
    }
    for ordinary in ["vault:entry-7774-abcdef", "env-plain-7775-PlMkNj"] {
        assert_eq!(withhold::text(ordinary), ordinary, "{ordinary}: a reference, a placeholder and a value under an ordinary name hold no secret");
    }
}
