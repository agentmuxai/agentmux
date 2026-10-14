// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::storage::agents::test_agent_def;

/// Every file under the shared registry root except the lease tree, with
/// its bytes — what "nothing shared was written" is measured against.
fn snapshot(root: &std::path::Path) -> Vec<(std::path::PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.file_name().is_some_and(|n| n == "leases") {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push((path.clone(), std::fs::read(&path).unwrap_or_default()));
            }
        }
    }
    out.sort();
    out
}

/// SPEC_AGENT_SINGLE_LIVE_INSTANCE_2026_09_24 §7 test 9 (I9): opening an
/// agent another AgentMux instance on this host is running is refused
/// before the handler writes anything — no block, no registry record.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn opening_an_agent_live_in_another_instance_writes_nothing() {
    let state = crate::server::tests::test_state();
    let tmp = tempfile::tempdir().unwrap();
    let registry = Arc::new(crate::registry::Registry::open(tmp.path().join("registry")).unwrap());
    state.mstore.set_registry(Arc::clone(&registry));
    let uid = uuid::Uuid::new_v4().to_string();
    let mut def = test_agent_def(&uid, "Agent3", "claude", "host", 1, "");
    state.mstore.agent_def_insert(&mut def).unwrap();

    let lease_store =
        crate::backend::agent_admission::lease_store_for(Some(Arc::clone(&registry))).unwrap();
    let _other = crate::backend::agent_admission::acquire(
        &lease_store,
        &uid,
        "Agent3",
        &Arc::from("another-srv-boot"),
        "other-block",
        None,
        |_| {},
    )
    .unwrap();

    let blocks_before = state.mstore.get_all::<Block>().unwrap().len();
    let registry_before = snapshot(registry.root());

    let err = open_agent_impl(
        &state,
        CommandAgentOpenData {
            agent_id: uid.clone(),
            tab_id: None,
            split_direction: None,
            split_reference_block_id: None,
            focus: None,
        },
    )
    .await
    .err()
    .expect("the open must be refused");

    assert!(err.starts_with("AGENT_LIVE_ELSEWHERE: Agent3 is already running"), "{err}");
    assert_eq!(state.mstore.get_all::<Block>().unwrap().len(), blocks_before, "no block created");
    assert_eq!(snapshot(registry.root()), registry_before, "shared registry untouched");
}
