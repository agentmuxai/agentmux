// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The spawn gate for a Claude agent on a model server of its own (#31509).

use super::*;

#[test]
fn an_unlinked_claude_agent_on_its_own_endpoint_is_not_refused_for_the_account() {
    // #31509: the account's login is discarded for an agent on a model server of its own, so an agent with
    // no linked Claude account launches; the same agent without an endpoint is refused (the canary below).
    let store = make_store();
    let mut def = gate_def(0);
    store.agent_def_insert(&mut def).unwrap();
    insert_block_for_agent(&store, "block-own-1", "def-1");
    store.instance_create(&make_instance("block-own-1", "id-own")).unwrap();

    let mut env: HashMap<String, String> = HashMap::new();
    let res = inject_identity_env_for_spawn(store.clone(), store.clone(), store.clone(), None, "block-own-1", &mut env, true);
    assert_eq!(res, Ok(()));
    assert!(env.is_empty(), "no account's login was put in");

    let mut env: HashMap<String, String> = HashMap::new();
    let res = inject_identity_env_for_spawn(store.clone(), store.clone(), store, None, "block-own-1", &mut env, false);
    assert_eq!(res, Err(SpawnGateError::MissingCredentials { provider: "claude".to_string(), linked: false }));
}

#[test]
fn a_claude_agent_on_its_own_endpoint_is_not_refused_for_an_account_that_no_longer_resolves() {
    let store = make_store();
    let mut def = gate_def(0);
    store.agent_def_insert(&mut def).unwrap();
    let claude = make_account("acct-own-gone", "claude", SecretRef::OAuthConfigDir { dir: test_config_dir("id-own2") });
    store.identity_upsert(&claude).unwrap();
    store.agent_identity_link("def-1", "acct-own-gone", "claude").unwrap();
    delete_account_row_keep_link(&store, "acct-own-gone");
    insert_block_for_agent(&store, "block-own-2", "def-1");
    store.instance_create(&make_instance("block-own-2", "id-own2")).unwrap();

    let mut env: HashMap<String, String> = HashMap::new();
    let res = inject_identity_env_for_spawn(store.clone(), store.clone(), store.clone(), None, "block-own-2", &mut env, true);
    assert_eq!(res, Ok(()));
    assert!(env.is_empty());

    let mut env: HashMap<String, String> = HashMap::new();
    let res = inject_identity_env_for_spawn(store.clone(), store.clone(), store, None, "block-own-2", &mut env, false);
    assert_eq!(res, Err(SpawnGateError::MissingCredentials { provider: "claude".to_string(), linked: true }));
}

#[test]
fn a_deleted_agent_is_refused_even_on_its_own_endpoint() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store_with_def_registry(&tmp);
    let mut def = crate::backend::storage::agents::test_agent_def("agent-own-deleted", "Gone", "claude", "agent", 1, "");
    store.agent_def_insert(&mut def).unwrap();
    assert!(store.agent_def_delete("agent-own-deleted").unwrap());
    insert_block_for_agent(&store, "block-own-3", "agent-own-deleted");

    let mut env = HashMap::new();
    let res = inject_identity_env_for_spawn(store.clone(), store.clone(), store, None, "block-own-3", &mut env, true);
    assert_eq!(res, Err(SpawnGateError::AgentDeleted { agent_id: "agent-own-deleted".to_string() }));
}
