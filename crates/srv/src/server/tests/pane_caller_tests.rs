// The window commands `setmeta`, `controllerresync`, `agentstop`, `createsubblock`, `deletesubblock` and the `agentinput` pre-start, through the real
// registered handlers, from the four callers: the agent that runs the pane, another agent, a connection nothing proved,
// and the window.

use std::sync::Arc;

use crate::backend::obj::Block;
use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{RpcContext, RpcMessage};
use crate::backend::storage::agents::test_agent_def;
use crate::server::tests::test_state;
use crate::server::AppState;

/// A state with two agents, a pane each, and a terminal no agent runs. The agents' slugs are the ones the store gave
/// them; `[mine, theirs, persons]` are the panes.
struct Fleet {
    state: AppState,
    owner: String,
    panes: [String; 3],
}

fn fleet() -> Fleet {
    let state = test_state();
    let mut agents = Vec::new();
    for n in 0..2 {
        let id = format!("pc-agent-{n}-{}", uuid::Uuid::new_v4().simple());
        let mut def = test_agent_def(&id, &id, "claude", "agent", 1, "");
        def.slug = id.clone();
        state.mstore.agent_def_insert(&mut def).unwrap();
        agents.push((def.id.clone(), def.slug.clone()));
    }
    let mut pane = |agent: Option<&str>| {
        let mut block = Block { oid: uuid::Uuid::new_v4().to_string(), ..Default::default() };
        if let Some(agent) = agent {
            block.meta.insert("agentId".to_string(), serde_json::json!(agent));
        }
        state.mstore.insert(&mut block).unwrap();
        block.oid
    };
    let panes = [pane(Some(&agents[0].0)), pane(Some(&agents[1].0)), pane(None)];
    Fleet { state, owner: agents[0].1.clone(), panes }
}

async fn call(state: &AppState, ctx: RpcContext, command: &str, data: serde_json::Value) -> RpcMessage {
    let (engine, mut output_rx) = WshRpcEngine::new();
    crate::server::ws_register::register_handlers(&engine, state.clone(), "pane-caller-test".to_string());
    engine.set_rpc_context(ctx);
    let engine: Arc<WshRpcEngine> = engine;
    engine.handle_message(RpcMessage {
        command: command.to_string(),
        reqid: uuid::Uuid::new_v4().to_string(),
        data: Some(data),
        ..Default::default()
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), output_rx.recv())
        .await
        .expect("the handler answers within 5s")
        .expect("the response channel stays open")
}

fn agent(slug: &str) -> RpcContext {
    RpcContext { agent_id: slug.to_string(), ..Default::default() }
}

/// What every refused call says: that it is refused, why, and what the agent can do instead (ask the other agent, or the person).
fn assert_refused(resp: &RpcMessage, why: &str) {
    assert!(resp.error.starts_with("FORBIDDEN") && resp.error.contains(why) && (resp.error.contains("Message send") || resp.error.contains("ask the person")), "{}", resp.error);
}

fn assert_reached(resp: &RpcMessage) {
    assert!(!resp.error.starts_with("FORBIDDEN"), "refused: {}", resp.error);
}

#[tokio::test]
async fn setmeta_writes_the_pane_of_the_agent_that_runs_it_and_of_the_window_and_refuses_another_agent() {
    let Fleet { state, owner, panes: [mine, theirs, persons], .. } = fleet();
    let write = |block: &str, n: i64| serde_json::json!({ "oref": format!("block:{block}"), "meta": { "term:agentlockuntil": n } });
    let lock = |block: &str| state.mstore.must_get::<Block>(block).unwrap().meta.get("term:agentlockuntil").cloned();

    assert_reached(&call(&state, agent(&owner), "setmeta", write(&mine, 1)).await);
    assert_eq!(lock(&mine), Some(serde_json::json!(1)), "an agent writes the meta of its own pane");
    for (ctx, n) in [(RpcContext::operator_for_tests(), 2), (RpcContext::default(), 3)] {
        assert_reached(&call(&state, ctx, "setmeta", write(&theirs, n)).await);
        assert_eq!(lock(&theirs), Some(serde_json::json!(n)), "the window and an unproven connection write as before");
    }

    assert_refused(&call(&state, agent(&owner), "setmeta", write(&theirs, 9)).await, "another agent's pane");
    assert_eq!(lock(&theirs), Some(serde_json::json!(3)), "a refused write changes nothing");
    assert_refused(&call(&state, agent(&owner), "setmeta", write(&persons, 9)).await, "no agent runs");
    assert_eq!(lock(&persons), None);
}

#[tokio::test]
async fn setmeta_never_lets_an_agent_take_over_a_browser_pane_and_lets_the_window() {
    let Fleet { state, owner, panes: [mine, ..], .. } = fleet();
    let take_over = serde_json::json!({ "oref": format!("block:{mine}"), "meta": { "browser:owner_agent": null } });
    let refused = call(&state, agent(&owner), "setmeta", take_over.clone()).await;
    assert!(refused.error.starts_with("FORBIDDEN") && refused.error.contains("Take over") && refused.error.contains("person"), "{}", refused.error);
    assert_reached(&call(&state, RpcContext::operator_for_tests(), "setmeta", take_over).await);
}

#[tokio::test]
async fn controllerresync_restarts_only_the_pane_of_the_agent_that_asks() {
    let Fleet { state, owner, panes: [mine, theirs, persons], .. } = fleet();
    let resync = |block: &str| serde_json::json!({ "blockid": block, "tabid": "t" });
    for ctx in [agent(&owner), RpcContext::operator_for_tests(), RpcContext::default()] {
        assert_reached(&call(&state, ctx, "controllerresync", resync(&mine)).await);
    }
    assert_refused(&call(&state, agent(&owner), "controllerresync", resync(&theirs)).await, "another agent's pane");
    assert_refused(&call(&state, agent(&owner), "controllerresync", resync(&persons)).await, "no agent runs");
    for ctx in [RpcContext::operator_for_tests(), RpcContext::default()] {
        assert_reached(&call(&state, ctx, "controllerresync", resync(&theirs)).await);
    }
}

#[tokio::test]
async fn agentstop_stops_the_agent_that_asks_and_the_window_and_refuses_one_agent_stopping_another() {
    let Fleet { state, owner, panes: [mine, theirs, _], .. } = fleet();
    let stop = |block: &str| serde_json::json!({ "blockid": block });
    for ctx in [agent(&owner), RpcContext::operator_for_tests(), RpcContext::default()] {
        assert_reached(&call(&state, ctx, "agentstop", stop(&mine)).await);
    }
    let refused = call(&state, agent(&owner), "agentstop", stop(&theirs)).await;
    assert_refused(&refused, "another agent's pane");
    assert!(refused.error.contains("another agent"), "{}", refused.error);
    for ctx in [RpcContext::operator_for_tests(), RpcContext::default()] {
        assert_reached(&call(&state, ctx, "agentstop", stop(&theirs)).await);
    }
}

#[tokio::test]
async fn the_agentinput_pre_start_is_checked_before_it_runs_and_a_typed_message_is_still_the_windows_alone() {
    let Fleet { state, owner, panes: [mine, theirs, _], .. } = fleet();
    let prestart = |block: &str| serde_json::json!({ "blockid": block, "message": "", "prestart": true });
    for ctx in [agent(&owner), RpcContext::operator_for_tests(), RpcContext::default()] {
        assert_reached(&call(&state, ctx, "agentinput", prestart(&mine)).await);
    }
    assert_refused(&call(&state, agent(&owner), "agentinput", prestart(&theirs)).await, "another agent's pane");

    // Not a pre-start: a message typed into the composer is the person's, so an agent connection is refused whichever pane it names.
    let typed = call(&state, agent(&owner), "agentinput", serde_json::json!({ "blockid": mine, "message": "hello" })).await;
    assert!(typed.error.starts_with("FORBIDDEN: only the AgentMux window"), "{}", typed.error);
}


#[tokio::test]
async fn createsubblock_is_for_a_sub_block_under_the_pane_of_the_agent_that_asks() {
    let Fleet { state, owner, panes: [mine, theirs, persons], .. } = fleet();
    let create = |parent: &str| serde_json::json!({ "parentblockid": parent, "blockdef": { "meta": { "view": "term" } } });
    for ctx in [agent(&owner), RpcContext::operator_for_tests(), RpcContext::default()] {
        assert_reached(&call(&state, ctx, "createsubblock", create(&mine)).await);
    }
    assert_refused(&call(&state, agent(&owner), "createsubblock", create(&theirs)).await, "another agent's pane");
    assert_refused(&call(&state, agent(&owner), "createsubblock", create(&persons)).await, "no agent runs");
    let under = |block: &str| state.mstore.must_get::<Block>(block).unwrap().subblockids.unwrap_or_default().len();
    assert_eq!(under(&mine), 3, "the three allowed calls each made one");
    assert_eq!(under(&theirs), 0, "a refused call made nothing");
    for ctx in [RpcContext::operator_for_tests(), RpcContext::default()] {
        assert_reached(&call(&state, ctx, "createsubblock", create(&theirs)).await);
    }
}

#[tokio::test]
async fn deletesubblock_removes_only_a_sub_block_of_the_pane_of_the_agent_that_asks() {
    let Fleet { state, owner, panes: [mine, theirs, persons], .. } = fleet();
    let sub = |parent: &str| {
        let mut block = Block { oid: uuid::Uuid::new_v4().to_string(), parentoref: format!("block:{parent}"), ..Default::default() };
        state.mstore.insert(&mut block).unwrap();
        block.oid
    };
    let delete = |block: &str| serde_json::json!({ "blockid": block });
    let exists = |block: &str| state.mstore.get::<Block>(block).ok().flatten().is_some();

    let own = sub(&mine);
    assert_reached(&call(&state, agent(&owner), "deletesubblock", delete(&own)).await);
    assert!(!exists(&own), "an agent deletes a sub-block of its own pane");

    let (theirs_sub, persons_sub) = (sub(&theirs), sub(&persons));
    assert_refused(&call(&state, agent(&owner), "deletesubblock", delete(&theirs_sub)).await, "another agent's pane");
    assert_refused(&call(&state, agent(&owner), "deletesubblock", delete(&persons_sub)).await, "no agent runs");
    assert!(exists(&theirs_sub) && exists(&persons_sub), "a refused call deletes nothing");

    // The pane itself is not a sub-block: an agent does not delete its own pane through this call, and is told what to use.
    let refused = call(&state, agent(&owner), "deletesubblock", delete(&mine)).await;
    assert!(refused.error.starts_with("FORBIDDEN") && refused.error.contains("Terminal close") && refused.error.contains("Pane close"), "{}", refused.error);
    assert!(exists(&mine));

    for ctx in [RpcContext::operator_for_tests(), RpcContext::default()] {
        let other = sub(&theirs);
        assert_reached(&call(&state, ctx, "deletesubblock", delete(&other)).await);
        assert!(!exists(&other), "the window and an unproven connection delete as before");
    }
}

#[tokio::test]
async fn setmeta_on_a_tab_window_or_workspace_is_the_persons_and_an_agent_is_told_what_to_use() {
    let Fleet { state, owner, .. } = fleet();
    for kind in ["tab", "window", "workspace", "client"] {
        let oref = format!("{kind}:{}", uuid::Uuid::new_v4());
        let write = serde_json::json!({ "oref": oref, "meta": { "name": "renamed" } });
        let refused = call(&state, agent(&owner), "setmeta", write.clone()).await;
        assert!(
            refused.error.starts_with("FORBIDDEN") && refused.error.contains(kind) && refused.error.contains("Pane and UI tools") && refused.error.contains("ask the person"),
            "{oref}: {}",
            refused.error
        );
        for ctx in [RpcContext::operator_for_tests(), RpcContext::default()] {
            assert_reached(&call(&state, ctx, "setmeta", write.clone()).await);
        }
    }
}
