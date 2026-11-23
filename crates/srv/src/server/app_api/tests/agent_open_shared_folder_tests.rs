// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Opening a host agent whose folder a container agent can write (`agent.open`, a layout's placeholder, an import):
//! it is refused with the marked refusal until the person has gone ahead with that pair, and the pane stays an empty
//! agent pane the window can ask in. A container agent is not asked.

use super::*;
use crate::backend::folder_overlap as overlap;
use crate::backend::storage::agents::test_agent_def;
use crate::sagas::park_agents_tests::{seed_block, seed_workspace};

/// One folder for every test's choices; each test names its own agents.
fn choices() {
    static DIR: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    let dir = DIR.get_or_init(|| tempfile::tempdir().unwrap());
    overlap::set_choices_dir_for_test(Some(dir.path().to_path_buf()));
}

fn save(state: &AppState, tag: &str, kind: &str, folder: &str) -> String {
    let id = format!("{tag}-{}", uuid::Uuid::new_v4().simple());
    let mut def = test_agent_def(&id, tag, "claude", kind, 1, "");
    def.working_directory = folder.to_string();
    state.mstore.agent_def_insert(&mut def).unwrap();
    id
}

struct Fleet {
    state: AppState,
    host: String,
    container: String,
    _root: tempfile::TempDir,
}

/// Hosty works in a folder inside the workspace of Aria, a container agent.
fn fleet() -> Fleet {
    choices();
    let state = crate::server::tests::test_state();
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("shared").to_string_lossy().to_string();
    let inside = root.path().join("shared/work").to_string_lossy().to_string();
    let container = save(&state, "Aria", "container", &workspace);
    let host = save(&state, "Hosty", "host", &inside);
    Fleet { state, host, container, _root: root }
}

fn open_cmd(agent: &str, into_block_id: Option<&str>) -> CommandAgentOpenData {
    CommandAgentOpenData {
        agent_id: agent.to_string(),
        tab_id: None,
        split_direction: None,
        split_reference_block_id: None,
        focus: Some(false),
        into_block_id: into_block_id.map(str::to_string),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_host_agent_that_shares_a_folder_is_not_opened_until_the_person_has_gone_ahead() {
    let f = fleet();
    let blocks_before = f.state.mstore.get_all::<Block>().unwrap().len();

    let err = open_agent_impl(&f.state, open_cmd(&f.host, None)).await.err().expect("the open must be refused");

    assert!(err.starts_with(&format!("{}: Hosty (host) and Aria (container) share a folder", overlap::OPEN_REFUSAL_CODE)), "{err}");
    assert!(err.contains("Only the person can go ahead"), "{err}");
    assert_eq!(f.state.mstore.get_all::<Block>().unwrap().len(), blocks_before, "no pane was made");

    overlap::accept(&f.host, &f.container).unwrap();
    let agent = f.state.mstore.agent_def_get(&f.host).unwrap().unwrap();
    agent_open_steps::admit_open(&f.state, &agent).await.expect("the person went ahead with that pair");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_container_agent_is_not_asked_when_it_is_opened() {
    let f = fleet();
    let agent = f.state.mstore.agent_def_get(&f.container).unwrap().unwrap();
    agent_open_steps::admit_open(&f.state, &agent).await.expect("a container agent is asked when its folders are saved");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_layouts_placeholder_that_is_refused_stays_an_empty_agent_pane_that_still_names_the_agent() {
    let f = fleet();
    let (_ws, tabs) = seed_workspace(&f.state, 1).await;
    let placeholder = seed_block(&f.state, &tabs[0], serde_json::json!({ "view": "agent", "layout:agent": f.host })).await;

    let err = agent_open::open_agent_into_block(&f.state, &f.host, &tabs[0], &placeholder).await.err().expect("refused");

    assert!(err.starts_with(overlap::OPEN_REFUSAL_CODE), "{err}");
    let stored = f.state.mstore.get::<Block>(&placeholder).unwrap().unwrap();
    assert_eq!(obj::meta_get_string(&stored.meta, "agentId", ""), "", "still an empty agent pane");
    assert_eq!(obj::meta_get_string(&stored.meta, "layout:agent", ""), f.host, "and it still says which agent goes here");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_open_into_a_pane_is_refused_the_same_way() {
    let f = fleet();
    let (_ws, tabs) = seed_workspace(&f.state, 1).await;
    let pane = seed_block(&f.state, &tabs[0], serde_json::json!({ "view": "agent" })).await;

    let err = open_agent_impl(&f.state, open_cmd(&f.host, Some(&pane))).await.err().expect("refused");

    assert!(err.starts_with(overlap::OPEN_REFUSAL_CODE), "{err}");
}

#[test]
fn layout_open_says_a_held_agents_pane_asks_and_reports_any_other_failure_as_before() {
    let held = layout::not_started_note("Hosty", &format!("{}: Hosty (host) and Aria (container) share a folder", overlap::OPEN_REFUSAL_CODE));
    assert_eq!(held, "Hosty works in a folder a container agent can write, so its pane asks you before it opens.");
    let other = layout::not_started_note("Hosty", "INVALID_PROVIDER: unknown provider 'x'");
    assert_eq!(other, "An agent (Hosty) didn't start, so its pane shows the agent picker: INVALID_PROVIDER: unknown provider 'x'");
}
