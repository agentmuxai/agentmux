// The steps `agent.open` runs, called one at a time (agent_open_steps.rs). Nothing here installs a CLI,
// starts a controller, touches the real auth directory or reaches the network.

use super::agent_open_steps::*;
use super::*;
use crate::backend::storage::agents::test_agent_def;

fn claude() -> &'static providers::ProviderConfig {
    providers::get_provider("claude").expect("the claude provider")
}

fn store_with(def: &mut AgentDefinition) -> Store {
    let store = Store::open_in_memory().unwrap();
    store.agent_def_insert(def).unwrap();
    store
}

fn set_content(store: &Store, agent_id: &str, kind: &str, content: &str) {
    store
        .agent_content_set(&AgentContent { agent_id: agent_id.to_string(), content_type: kind.to_string(), content: content.to_string(), updated_at: 0 })
        .unwrap();
}

#[test]
fn launch_env_has_the_agents_identity_and_auth_and_the_definitions_variables_never_override_auth() {
    let mut def = test_agent_def("agent-env-1", "Env One", "claude", "host", 1, "");
    let store = store_with(&mut def);
    let auth_var = claude().auth_config_dir_env_var;
    set_content(&store, "agent-env-1", "env", &format!("FOO=bar\n{auth_var}=/evil\n"));

    let env = launch_env(&store, &def, claude(), "/auth/dir", "env-one", "persistent").unwrap();

    assert_eq!(env["AGENTMUX_AGENT_ID"], "env-one");
    assert_eq!(env["MUXBUS_AGENT_ID"], "env-one");
    assert_eq!(env["FOO"], "bar");
    if !auth_var.is_empty() {
        assert_eq!(env[auth_var], "/auth/dir");
    }
    assert!(!env.contains_key("CLAUDE_CODE_EXIT_AFTER_STOP_DELAY"), "a persistent controller has no exit delay");
}

#[test]
fn a_subprocess_controller_gets_the_exit_delay() {
    let mut def = test_agent_def("agent-env-2", "Env Two", "claude", "host", 1, "");
    let store = store_with(&mut def);
    let env = launch_env(&store, &def, claude(), "/auth/dir", "env-two", "subprocess").unwrap();
    assert_eq!(env["CLAUDE_CODE_EXIT_AFTER_STOP_DELAY"], "30000");
}

#[test]
fn pane_identity_meta_names_the_agent_and_keeps_one_colour_across_opens() {
    let mut def = test_agent_def("agent-meta-1", "Meta One", "claude", "host", 1, "");
    def.container_image = "img:1".to_string();
    def.container_volumes = r#"[{"host":"/a","container":"/b"}]"#.to_string();
    let store = store_with(&mut def);
    set_content(&store, "agent-meta-1", "ui:zoom", "1.5");

    let first = pane_identity_meta(&store, &def, claude());
    assert_eq!(first["view"], "agent");
    assert_eq!(first["agentId"], "agent-meta-1");
    assert_eq!(first["agentName"], "Meta One");
    assert_eq!(first["agentIcon"], "wand-magic-sparkles", "the default icon");
    assert_eq!(first["agent:container_image"], "img:1");
    assert_eq!(first["agent:container_volumes"], def.container_volumes);
    assert_eq!(first["term:zoom"], 1.5);
    let color = first["frame:activebordercolor"].as_str().unwrap().to_string();
    assert_eq!(store.agent_content_get("agent-meta-1", "ui:color").unwrap().unwrap().content, color, "the colour is stored");

    let second = pane_identity_meta(&store, &def, claude());
    assert_eq!(second["frame:activebordercolor"], color.as_str());
}

#[test]
fn add_launch_meta_sets_the_command_and_lets_the_callers_keys_win() {
    let mut meta = MetaMapType::new();
    meta.insert("view".to_string(), json!("agent"));
    let mut extra = MetaMapType::new();
    extra.insert("cmd:cwd".to_string(), json!("/from/the/caller"));
    let launch = LaunchMeta {
        controller_type: "persistent",
        cli_path: "/bin/claude",
        seeded: crate::backend::agent_runtime::seed_launch("claude", vec!["--x".to_string()], ""),
        pane_flags: "",
        work_dir: "/work/launch-1",
        env_vars: serde_json::Map::new(),
        resume_session_id: Some("sess-9"),
        extra_meta: extra,
    };
    add_launch_meta(&mut meta, claude(), launch);
    assert_eq!(meta["controller"], "persistent");
    assert_eq!(meta["cmd"], "/bin/claude");
    assert_eq!(meta["agent:sessionid"], "sess-9");
    assert_eq!(meta["cmd:cwd"], "/from/the/caller");
}

#[test]
fn routing_id_prefers_the_stored_slug_and_work_dir_the_set_directory() {
    let mut def = test_agent_def("agent-route-1", "Route One", "claude", "host", 1, "");
    def.slug = "route-one-stable".to_string();
    assert_eq!(routing_id(&def), "route-one-stable");
    def.slug.clear();
    assert_eq!(routing_id(&def), agentmux_common::slug::path_slug("Route One"));

    def.working_directory = "/work/route-1".to_string();
    assert_eq!(agent_work_dir(&def), "/work/route-1");
    def.working_directory.clear();
    assert_eq!(agent_work_dir(&def), crate::backend::storage::agents::default_agent_working_dir("Route One"));
}

#[test]
fn a_host_agent_opens_persistent_and_a_container_agent_opens_a_subprocess() {
    let host = test_agent_def("agent-ct-1", "Host", "claude", "host", 1, "");
    assert_eq!(open_controller_type(&host, claude()).unwrap(), "persistent");
    let container = test_agent_def("agent-ct-2", "Boxed", "claude", "container", 1, "");
    assert_eq!(open_controller_type(&container, claude()).unwrap(), "subprocess");
}

#[tokio::test]
async fn nothing_is_resumed_when_there_is_no_shared_registry() {
    let mut def = test_agent_def("agent-resume-1", "Resume One", "claude", "host", 1, "");
    let store = store_with(&mut def);
    assert_eq!(resume_session_for(&store, &def).await, None);
}

#[tokio::test]
async fn filling_a_pane_that_is_missing_or_not_an_empty_agent_pane_is_refused() {
    let state = crate::server::tests::test_state();
    let missing = uuid::Uuid::new_v4().to_string();
    let err = fill_existing_pane(&state, &missing, json!({})).await.unwrap_err();
    assert_eq!(err, format!("agent.open: no such pane: {missing}"));

    let mut term = Block { oid: uuid::Uuid::new_v4().to_string(), ..Default::default() };
    term.meta.insert("view".to_string(), json!("term"));
    state.mstore.insert(&mut term).unwrap();
    let err = fill_existing_pane(&state, &term.oid, json!({})).await.unwrap_err();
    assert_eq!(err, "agent.open: the target pane is not an empty agent pane");
}

#[test]
fn a_block_with_no_controller_has_status_init() {
    assert_eq!(launch_status("no-such-block-open"), "init");
}
