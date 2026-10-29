//! `skill.catalog.set_opt_out`: one agent's exception to a skill that reaches everyone.

use super::*;
use serde_json::{json, Value};

async fn call(state: &AppState, command: &str, data: Value) -> Result<Value, String> {
    let (engine, mut rx) = WshRpcEngine::new();
    register(&engine, state);
    engine.handle_message(RpcMessage { command: command.to_string(), reqid: "req-1".to_string(), data: Some(data), ..Default::default() });
    let resp = tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv()).await.unwrap().unwrap();
    if resp.error.is_empty() { Ok(resp.data.unwrap_or(Value::Null)) } else { Err(resp.error) }
}

fn for_agent(list: &Value, skill_id: &str) -> Value {
    list.as_array().unwrap().iter().find(|s| s["id"] == skill_id).unwrap().clone()
}

#[tokio::test]
async fn an_opt_out_shows_in_the_agents_list_and_is_reversible_and_checked() {
    let state = crate::server::tests::test_state();
    let made = call(&state, COMMAND_SKILL_CATALOG_UPSERT, json!({"name": "optout-rpc-t", "skill_type": "agent-skill", "content": "c"})).await.unwrap();
    let skill_id = made["id"].as_str().unwrap();
    let agent_id = "optout-agent-t";
    state
        .mstore
        .conn()
        .lock()
        .unwrap()
        .execute("INSERT INTO db_agents (id, name, slug, provider, created_at) VALUES (?1, ?1, ?1, 'claude', 1)", [agent_id])
        .unwrap();

    call(&state, COMMAND_SKILL_CATALOG_SET_REACH, json!({"skill_id": skill_id, "all_agents": true, "new_agents": true})).await.unwrap();
    let listed = call(&state, COMMAND_SKILL_CATALOG_LIST_FOR_AGENT, json!({"agent_id": agent_id})).await.unwrap();
    assert_eq!((for_agent(&listed, skill_id)["reached"].as_bool(), for_agent(&listed, skill_id)["opted_out"].as_bool()), (Some(true), Some(false)));

    call(&state, COMMAND_SKILL_CATALOG_SET_OPT_OUT, json!({"skill_id": skill_id, "agent_id": agent_id, "opted_out": true})).await.unwrap();
    let listed = call(&state, COMMAND_SKILL_CATALOG_LIST_FOR_AGENT, json!({"agent_id": agent_id})).await.unwrap();
    assert_eq!((for_agent(&listed, skill_id)["reached"].as_bool(), for_agent(&listed, skill_id)["opted_out"].as_bool()), (Some(false), Some(true)));

    call(&state, COMMAND_SKILL_CATALOG_SET_OPT_OUT, json!({"skill_id": skill_id, "agent_id": agent_id, "opted_out": false})).await.unwrap();
    let listed = call(&state, COMMAND_SKILL_CATALOG_LIST_FOR_AGENT, json!({"agent_id": agent_id})).await.unwrap();
    assert_eq!(for_agent(&listed, skill_id)["reached"].as_bool(), Some(true));

    let e = call(&state, COMMAND_SKILL_CATALOG_SET_OPT_OUT, json!({"skill_id": "no-such", "agent_id": agent_id, "opted_out": true})).await.unwrap_err();
    assert!(e.contains("not found"), "{e}");
}
