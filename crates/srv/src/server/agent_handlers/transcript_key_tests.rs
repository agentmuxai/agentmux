// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The segment log and the transcript are keyed by the same string for a user
//! agent, and by different ones for a template launch (Kanban #187).

use super::input::persisted_agent_identity;
use crate::backend::storage::agents::test_agent_def;
use crate::backend::storage::store::Store;

fn block_with_agent_id(store: &Store, block_id: &str, agent_id: &str) {
    let mut block = crate::backend::obj::Block {
        oid: block_id.to_string(),
        meta: [("view", "agent"), ("agentId", agent_id)]
            .into_iter()
            .map(|(k, v)| (k.to_string(), serde_json::json!(v)))
            .collect(),
        ..Default::default()
    };
    store.insert(&mut block).unwrap();
}

/// A user agent's segment log and transcript are one key. The segment log
/// is `agent-uid:<uid>:segments`, with the uid the spawn env carries (the
/// block's resolved row id); the transcript is `agent:<agentId>:current`,
/// with the id the block's meta names. The resolver reads the row whose id
/// is the block's `agentId`, so for any non-template row they are the same
/// string: a fork whose slug is a pre-fix `loap-loap-2-loap-3`, and the
/// same agent after a rename or an account change (none of which touch the
/// row id; the slug migration rewrites only the slug).
#[test]
fn segment_log_uid_and_transcript_zone_key_are_one_string_for_a_user_agent() {
    use crate::backend::agent_session::{agent_current_zone, agent_zone_for_block_meta};
    use crate::backend::continuity_segments::zone_for;

    let store = Store::open_in_memory().unwrap();
    let mut plain = test_agent_def("uid-plain", "Plain", "claude", "host", 1, "");
    store.agent_def_insert(&mut plain).unwrap();
    let mut fork = test_agent_def("uid-loap-3", "Loap-3", "claude", "host", 1, "");
    fork.slug = "loap-loap-2-loap-3".to_string();
    fork.parent_id = "uid-plain".to_string();
    fork.branch_label = "loap-3".to_string();
    store.agent_def_insert(&mut fork).unwrap();
    block_with_agent_id(&store, "block-plain", "uid-plain");
    block_with_agent_id(&store, "block-fork", "uid-loap-3");

    let keys = |store: &Store, block_id: &str| {
        let uid = persisted_agent_identity(store, block_id).expect("the block resolves to a row").uid;
        let meta = store.get::<crate::backend::obj::Block>(block_id).unwrap().unwrap().meta;
        let zone = agent_zone_for_block_meta(&meta).expect("the block names an agent");
        (uid, zone)
    };
    let assert_one_key = |store: &Store, block_id: &str, id: &str| {
        let (uid, zone) = keys(store, block_id);
        assert_eq!(uid, id, "{block_id}: the spawn's uid is the row id");
        assert_eq!(zone, agent_current_zone(&uid), "{block_id}: the transcript zone carries the uid");
        assert_eq!(
            zone_for(&uid).as_deref(),
            Some(format!("agent-uid:{id}:segments").as_str()),
            "{block_id}: so does the segment log"
        );
    };
    assert_one_key(&store, "block-plain", "uid-plain");
    assert_one_key(&store, "block-fork", "uid-loap-3");

    // A rename and an account change rewrite the row, not its id.
    plain.name = "Renamed".to_string();
    plain.accounts = r#"{"claude":"acct-b"}"#.to_string();
    store.agent_def_update(&mut plain).unwrap();
    assert_one_key(&store, "block-plain", "uid-plain");
}

/// The one place the two keys part, pinned so it stays deliberate: a block
/// that names a template (a launch from one) shares the template's
/// transcript `agent:<templateId>:current`, while the spawn's uid, and so
/// its segment log, is the launched row's own id (a template launch is a
/// different agent from its siblings).
#[test]
fn a_template_launch_shares_the_templates_transcript_but_keeps_its_own_segment_log() {
    use crate::backend::agent_session::agent_zone_for_block_meta;
    use crate::backend::storage::store::{AgentInstance, InstanceStatus};

    let store = Store::open_in_memory().unwrap();
    let mut template = test_agent_def("tpl-claude", "Claude", "claude", "host", 1, "");
    template.is_seeded = 1;
    store.agent_def_insert(&mut template).unwrap();
    // The block names the template and carries the launched row's id, as the frontend stamps it.
    let mut block = crate::backend::obj::Block {
        oid: "block-launch".to_string(),
        meta: [("view", "agent"), ("agentId", "tpl-claude"), ("agentInstanceId", "launch-1")]
            .into_iter()
            .map(|(k, v)| (k.to_string(), serde_json::json!(v)))
            .collect(),
        ..Default::default()
    };
    store.insert(&mut block).unwrap();
    let launch = |id: &str| AgentInstance {
        id: id.to_string(),
        definition_id: "tpl-claude".to_string(),
        parent_instance_id: String::new(),
        block_id: "block-launch".to_string(),
        session_id: String::new(),
        status: InstanceStatus::Running.as_str().to_string(),
        github_context: String::new(),
        started_at: 1,
        ended_at: 0,
        created_at: 1,
        identity_id: String::new(),
        memory_id: String::new(),
        instance_name: String::new(),
        working_directory: String::new(),
        display_hidden: false,
    };
    let created = store.instance_create(&launch("launch-1")).unwrap();
    assert_eq!(created.id, "launch-1", "a fresh template launch gets a row of its own");

    let uid = persisted_agent_identity(&store, "block-launch").expect("the launch resolves").uid;
    let meta = store.get::<crate::backend::obj::Block>("block-launch").unwrap().unwrap().meta;
    assert_eq!(uid, "launch-1");
    assert_eq!(agent_zone_for_block_meta(&meta).as_deref(), Some("agent:tpl-claude:current"));
    assert_ne!(
        crate::backend::continuity_segments::zone_for(&uid).unwrap(),
        "agent-uid:tpl-claude:segments",
        "the launch's segments are not the template's"
    );
}
