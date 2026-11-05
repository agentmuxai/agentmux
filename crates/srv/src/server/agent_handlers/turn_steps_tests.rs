// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The steps of an agent turn, called without a controller.

use super::*;
use crate::backend::obj::MetaMapType;
use crate::server::tests::test_state;
use serde_json::json;

fn block_with(meta: &[(&str, serde_json::Value)]) -> Block {
    let mut block = Block { oid: uuid::Uuid::new_v4().to_string(), ..Default::default() };
    let mut m = MetaMapType::new();
    for (k, v) in meta {
        m.insert(k.to_string(), v.clone());
    }
    block.meta = m;
    block
}

#[test]
fn a_pane_with_no_command_runs_claude_with_the_stream_json_args() {
    let base = spawn_base(&block_with(&[]));
    assert_eq!(base.cli_command, "claude");
    assert_eq!(base.working_dir, "");
    assert_eq!(base.cli_args, ["-p", "--input-format", "stream-json", "--output-format", "stream-json"]);
}

#[test]
fn a_pane_runs_the_command_args_and_directory_its_meta_names() {
    let block = block_with(&[("cmd", json!("/bin/agent")), ("cmd:args", json!(["--one", "--two"])), ("cmd:cwd", json!("/work/x"))]);
    let base = spawn_base(&block);
    assert_eq!(base.cli_command, "/bin/agent");
    assert_eq!(base.cli_args, ["--one", "--two"]);
    assert_eq!(base.working_dir, "/work/x");
}

#[tokio::test]
async fn a_refusal_before_any_process_is_recorded_as_the_blocks_last_failure() {
    let state = test_state();
    let deps = AgentTurnDeps::from_state(&state);
    let mut block = block_with(&[]);
    state.mstore.insert(&mut block).unwrap();

    surface_refusal(&deps, &block.oid, "Attachments aren't supported for container agents yet.");

    let stored: Block = state.mstore.must_get(&block.oid).unwrap();
    let failure = stored.meta.get(crate::backend::blockcontroller::core::META_LAST_FAILURE).expect("a last failure");
    assert!(failure.to_string().contains("Attachments aren't supported"), "{failure}");
}

#[tokio::test]
async fn a_turn_for_a_block_with_no_controller_is_refused_before_anything_is_read() {
    let state = test_state();
    let deps = AgentTurnDeps::from_state(&state);
    let origin = crate::backend::message_origin::MessageOrigin::broadcast();
    let err = super::super::input::run_agent_turn_joining(
        &deps,
        "blk-turn-no-controller".to_string(),
        "hi".to_string(),
        None,
        super::super::input::TurnRegistration::Skip,
        origin,
        Vec::new(),
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(err, "no controller for block blk-turn-no-controller");
}
