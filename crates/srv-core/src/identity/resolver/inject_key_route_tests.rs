// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The spawn gate for a Claude agent on an Anthropic API key account: it is pointed at the gateway's key route and
//! never holds the key (docs/specs/SPEC_AGENT_PANE_API_KEY_ACCOUNT_SWITCH_2026_11_16.md).

use super::*;
use crate::backend::model_gateway::{catalog, token};

/// The token registry is process-wide and keyed by agent definition and route, and these tests all use one definition: run them one at a time.
fn serial() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// A made-up key of the shape the redaction code knows, built from pieces so no key-shaped literal sits in the file.
fn made_up_key() -> String {
    ["sk-ant", "-api03-", "MADEUPMADEUPMADEUPMADEUPMADEUP0123456789"].concat()
}

fn key_account(id: &str, kind: &str) -> IdentityAccount {
    let mut account = make_account(id, "anthropic", SecretRef::Env { env_var: format!("AMX_TEST_KEY_ROUTE_{id}") });
    account.kind = kind.to_string();
    account.display_name = "Work key".to_string();
    account
}

fn spawn(store: &Arc<Store>, block: &str, own_endpoint: bool) -> (Result<(), SpawnGateError>, HashMap<String, String>) {
    spawn_with(store, store, store, block, own_endpoint)
}

/// The spawn with the model, account and link stores given apart, as srv opens them.
fn spawn_with(mstore: &Arc<Store>, id_store: &Arc<Store>, identity_store: &Arc<Store>, block: &str, own_endpoint: bool) -> (Result<(), SpawnGateError>, HashMap<String, String>) {
    std::env::set_var("AGENTMUX_LOCAL_URL", "http://127.0.0.1:4000");
    let mut env = HashMap::new();
    let res = inject_identity_env_for_spawn(mstore.clone(), id_store.clone(), identity_store.clone(), None, block, &mut env, own_endpoint);
    (res, env)
}

fn agent_on(store: &Arc<Store>, account: &IdentityAccount, block: &str) {
    let mut def = gate_def(0);
    store.agent_def_insert(&mut def).unwrap();
    store.identity_upsert(account).unwrap();
    store.agent_identity_link("def-1", &account.id, "anthropic").unwrap();
    insert_block_for_agent(store, block, "def-1");
    store.instance_create(&make_instance(block, "id-key")).unwrap();
}

#[test]
fn a_claude_agent_on_an_api_key_is_given_the_gateway_and_a_token_and_never_the_key() {
    let key = made_up_key();
    std::env::set_var("AMX_TEST_KEY_ROUTE_acct-key-1", &key);
    let _serial = serial();
    let store = make_store();
    agent_on(&store, &key_account("acct-key-1", "api_key"), "block-key-1");

    let (res, env) = spawn(&store, "block-key-1", false);

    assert_eq!(res, Ok(()), "the key link satisfies the Claude agent's account gate");
    assert_eq!(env["ANTHROPIC_BASE_URL"], "http://127.0.0.1:4000/api/v1/model/anthropic");
    assert!(env["ANTHROPIC_AUTH_TOKEN"].starts_with(token::TOKEN_PREFIX));
    assert_eq!(env["ANTHROPIC_API_KEY"], "", "no key of any kind in its place");
    assert!(!env.values().any(|v| v.contains(&key) || v.contains("MADEUPMADEUP")), "the key is nowhere in the agent's environment: {env:?}");
    let grant = token::grant_for(&env["ANTHROPIC_AUTH_TOKEN"]).unwrap();
    assert_eq!((grant.route.as_str(), grant.account.as_str(), grant.def_id.as_str()), (catalog::KEY_ROUTE_ID, "acct-key-1", "def-1"));
}

#[test]
fn the_token_is_for_the_key_route_only_and_follows_the_account_the_agent_is_on() {
    let _serial = serial();
    let store = make_store();
    agent_on(&store, &key_account("acct-key-2", "api_key"), "block-key-2");
    let (_, env) = spawn(&store, "block-key-2", false);
    let first = env["ANTHROPIC_AUTH_TOKEN"].clone();

    store.identity_upsert(&key_account("acct-key-3", "api_key")).unwrap();
    store.agent_identity_unlink("def-1", "anthropic").unwrap();
    store.agent_identity_link("def-1", "acct-key-3", "anthropic").unwrap();
    let (_, env) = spawn(&store, "block-key-2", false);

    assert_eq!(env["ANTHROPIC_AUTH_TOKEN"], first, "the same token, now spending the other account");
    assert_eq!(token::grant_for(&first).unwrap().account, "acct-key-3");
    assert_ne!(token::grant_for(&first).unwrap().route, "openrouter", "never a model route's token");
}

#[test]
fn an_account_that_is_not_an_api_key_is_not_carried_by_the_gateway() {
    let _serial = serial();
    let store = make_store();
    agent_on(&store, &key_account("acct-key-4", "pat"), "block-key-4");
    let (res, env) = spawn(&store, "block-key-4", false);
    assert_eq!(res, Err(SpawnGateError::MissingCredentials { provider: "claude".to_string(), linked: false }));
    assert!(!env.contains_key("ANTHROPIC_AUTH_TOKEN"));
}

#[test]
fn an_agent_with_a_key_link_whose_account_is_gone_is_refused_not_left_on_a_login() {
    let _serial = serial();
    let store = make_store();
    agent_on(&store, &key_account("acct-key-5", "api_key"), "block-key-5");
    delete_account_row_keep_link(&store, "acct-key-5");
    let (res, env) = spawn(&store, "block-key-5", false);
    assert!(matches!(res, Err(SpawnGateError::MissingCredentials { .. })), "{res:?}");
    assert!(env.is_empty());
}

#[test]
fn an_agent_on_its_own_endpoint_does_not_use_the_key_route() {
    let _serial = serial();
    let store = make_store();
    agent_on(&store, &key_account("acct-key-6", "api_key"), "block-key-6");
    let (res, env) = spawn(&store, "block-key-6", true);
    assert_eq!(res, Ok(()));
    assert!(!env.contains_key("ANTHROPIC_AUTH_TOKEN"), "its own endpoint's authentication is put in after this step");
}

#[test]
fn a_container_agent_with_a_key_link_is_refused_and_given_neither_the_key_nor_the_gateway() {
    let _serial = serial();
    let store = make_store();
    agent_on(&store, &key_account("acct-key-7", "api_key"), "block-key-7");
    let mut block = store.get::<crate::backend::obj::Block>("block-key-7").unwrap().unwrap();
    block.meta.insert("agentMode".to_string(), serde_json::json!("container"));
    store.update(&mut block).unwrap();

    let (res, env) = spawn(&store, "block-key-7", false);

    assert_eq!(res, Err(SpawnGateError::MissingCredentials { provider: "claude".to_string(), linked: false }));
    assert!(env.is_empty(), "nothing of the key or the gateway reaches a container: {env:?}");
}

#[test]
fn a_key_link_inherited_from_a_template_is_not_carried_by_the_gateway() {
    let _serial = serial();
    let store = make_store();
    let mut tpl = gate_def(0);
    tpl.id = "tpl-1".to_string();
    tpl.is_seeded = 1;
    store.agent_def_insert(&mut tpl).unwrap();
    let mut def = gate_def(0);
    def.parent_id = "tpl-1".to_string();
    store.agent_def_insert(&mut def).unwrap();
    store.identity_upsert(&key_account("acct-key-9", "api_key")).unwrap();
    store.agent_identity_link("tpl-1", "acct-key-9", "anthropic").unwrap();
    insert_block_for_agent(&store, "block-key-9", "def-1");
    store.instance_create(&make_instance("block-key-9", "id-key")).unwrap();

    let (res, env) = spawn(&store, "block-key-9", false);

    assert!(res.is_err(), "the gate is not satisfied by a key the gateway would refuse: {res:?}");
    assert!(!env.contains_key("ANTHROPIC_AUTH_TOKEN"), "{env:?}");
}

#[test]
fn a_key_linked_in_the_link_store_is_carried_when_the_stores_are_apart() {
    let _serial = serial();
    let (mstore, id_store, link_store) = (make_store(), make_store(), make_store());
    let account = key_account("acct-key-10", "api_key");
    let mut def = gate_def(0);
    mstore.agent_def_insert(&mut def).unwrap();
    insert_block_for_agent(&mstore, "block-key-10", "def-1");
    mstore.instance_create(&make_instance("block-key-10", "id-key")).unwrap();
    id_store.identity_upsert(&account).unwrap();
    link_store.identity_upsert(&account).unwrap();
    link_store.agent_def_insert(&mut gate_def(0)).unwrap();
    link_store.agent_identity_link("def-1", &account.id, "anthropic").unwrap();

    let (res, env) = spawn_with(&mstore, &id_store, &link_store, "block-key-10", false);

    assert_eq!(res, Ok(()), "the link is read where srv writes it");
    assert!(env["ANTHROPIC_AUTH_TOKEN"].starts_with(token::TOKEN_PREFIX));
}
