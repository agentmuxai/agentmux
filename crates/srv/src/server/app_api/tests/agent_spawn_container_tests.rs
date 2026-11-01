// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Where a new agent runs: `Agents spawn` and `Agents create` with `runtime`, `container_image`, `memory_mb` and `cpus`.

use super::*;
use crate::backend::container_image::{DEFAULT_AGENT_IMAGE, DEFAULT_DEVELOPER_IMAGE};
use crate::backend::storage::agents::test_agent_def;
use crate::server::tests::test_state;

fn ask(runtime: Option<&str>, image: Option<&str>, memory: Option<serde_json::Value>, cpus: Option<serde_json::Value>) -> ContainerAsk {
    ContainerAsk { runtime: runtime.map(str::to_string), container_image: image.map(str::to_string), memory_mb: memory, cpus }
}

fn refused(r: Result<Placement, SpawnError>) -> (StatusCode, String) {
    let e = r.unwrap_err();
    (e.status, e.message)
}

#[test]
fn nothing_asked_leaves_the_new_agent_as_its_source() {
    assert_eq!(decide(&ContainerAsk::default(), false, false, "", Ok(())).unwrap(), Placement::AsSource);
    assert_eq!(decide(&ContainerAsk::default(), true, true, "x/y:1", Ok(())).unwrap(), Placement::AsSource, "a container leader's worker, as before");
}

#[test]
fn a_host_agent_can_start_a_container_agent_with_an_image_and_limits() {
    let got = decide(&ask(Some("container"), Some(DEFAULT_DEVELOPER_IMAGE), Some(json!(8192)), Some(json!(2.5))), false, false, "", Ok(())).unwrap();
    assert_eq!(
        got,
        Placement::Container(ContainerPlan { image: DEFAULT_DEVELOPER_IMAGE.to_string(), memory_mb: Some("8192".into()), cpus: Some("2.5".into()) })
    );
    let auto = decide(&ask(Some("container"), None, Some(json!("auto")), Some(json!(0))), false, false, "", Ok(())).unwrap();
    assert_eq!(
        auto,
        Placement::Container(ContainerPlan { image: DEFAULT_AGENT_IMAGE.to_string(), memory_mb: Some(String::new()), cpus: Some("0".into()) }),
        "the base image when nothing names one"
    );
}

#[test]
fn a_container_source_is_kept_unless_the_runtime_says_native() {
    // A template that is a container type, and only limits asked: it stays a container with the template's image.
    let got = decide(&ask(None, None, Some(json!(4096)), None), false, true, "tpl/image:2", Ok(())).unwrap();
    assert_eq!(got, Placement::Container(ContainerPlan { image: "tpl/image:2".into(), memory_mb: Some("4096".into()), cpus: None }));
    assert_eq!(decide(&ask(Some("native"), None, None, None), false, true, "tpl/image:2", Ok(())).unwrap(), Placement::Native);
    assert_eq!(decide(&ask(Some("host"), None, None, None), false, true, "", Ok(())).unwrap(), Placement::Native, "host is native");
}

#[test]
fn an_image_or_limits_for_a_native_agent_are_refused() {
    for a in [ask(Some("native"), Some(DEFAULT_AGENT_IMAGE), None, None), ask(None, None, Some(json!(1024)), None), ask(Some("native"), None, None, Some(json!(1)))] {
        let (status, why) = refused(decide(&a, false, false, "", Ok(())));
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(why.contains("add runtime: container"), "{why}");
    }
}

#[test]
fn an_unknown_runtime_image_or_limit_is_refused() {
    let (status, why) = refused(decide(&ask(Some("vm"), None, None, None), false, false, "", Ok(())));
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(why.contains("native or container"), "{why}");
    let (_, why) = refused(decide(&ask(Some("container"), Some("my/own:1"), None, None), false, false, "", Ok(())));
    assert!(why.contains("container_image must be"), "{why}");
    let (_, why) = refused(decide(&ask(Some("container"), None, Some(json!("lots")), None), false, false, "", Ok(())));
    assert!(why.contains("memory_mb"), "{why}");
    let (_, why) = refused(decide(&ask(Some("container"), None, None, Some(json!(-1))), false, false, "", Ok(())));
    assert!(why.contains("cpus"), "{why}");
}

#[test]
fn a_container_without_docker_is_refused_with_the_reason() {
    let (status, why) = refused(decide(&ask(Some("container"), None, None, None), false, false, "", Err("the Docker daemon is not running".into())));
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(why.starts_with("DOCKER_UNAVAILABLE") && why.contains("the Docker daemon is not running") && why.contains("runtime: native"), "{why}");
    // A native agent never needs it.
    assert_eq!(decide(&ask(Some("native"), None, None, None), false, false, "", Err("no".into())).unwrap(), Placement::Native);
}

#[test]
fn a_container_agent_starts_only_container_agents_and_sets_no_limits() {
    let (status, why) = refused(decide(&ask(Some("native"), None, None, None), true, true, "x/y:1", Ok(())));
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(why.starts_with("CONTAINER_AGENT"), "{why}");
    // Nothing asked but the template is a host one: a container agent does not get a host agent that way either.
    let (status, why) = refused(decide(&ContainerAsk::default(), true, false, "", Ok(())));
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(why.starts_with("CONTAINER_AGENT"), "{why}");
    let (status, why) = refused(decide(&ask(None, None, Some(json!(65536)), None), true, true, "x/y:1", Ok(())));
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(why.contains("cannot set a container's limits"), "{why}");
    // It may choose the image among the two AgentMux ships, and make a container agent from a host template.
    let got = decide(&ask(Some("container"), Some(DEFAULT_DEVELOPER_IMAGE), None, None), true, false, "", Ok(())).unwrap();
    assert_eq!(got, Placement::Container(ContainerPlan { image: DEFAULT_DEVELOPER_IMAGE.into(), memory_mb: None, cpus: None }));
}

#[test]
fn the_approval_prompt_says_where_it_runs() {
    assert_eq!(Placement::AsSource.describe(), "");
    assert_eq!(Placement::Native.describe(), " on this computer");
    let c = Placement::Container(ContainerPlan { image: DEFAULT_DEVELOPER_IMAGE.into(), memory_mb: Some("8192".into()), cpus: Some("0".into()) });
    let d = c.describe();
    assert!(d.contains("in a container") && d.contains(DEFAULT_DEVELOPER_IMAGE) && d.contains("memory 8192 MB") && d.contains("CPUs unlimited"), "{d}");
}

#[tokio::test]
async fn applying_a_container_placement_sets_the_type_the_image_and_the_limits() {
    let state = test_state();
    let mut def = test_agent_def("new-agent", "New", "claude", "host", 1, "");
    state.mstore.agent_def_insert(&mut def).unwrap();
    let plan = Placement::Container(ContainerPlan { image: DEFAULT_DEVELOPER_IMAGE.into(), memory_mb: Some("8192".into()), cpus: None });
    apply(&state, &mut def, &plan).unwrap();
    let stored = state.mstore.agent_def_get(&def.id).unwrap().unwrap();
    assert_eq!((stored.agent_type.as_str(), stored.environment.as_str(), stored.container_image.as_str()), ("container", "docker", DEFAULT_DEVELOPER_IMAGE));
    assert_eq!(state.mstore.container_limits_get(&def.id).unwrap(), Some(("8192".to_string(), String::new())), "cpus stay automatic");

    apply(&state, &mut def, &Placement::Native).unwrap();
    let stored = state.mstore.agent_def_get(&def.id).unwrap().unwrap();
    assert_eq!((stored.agent_type.as_str(), stored.environment.as_str(), stored.container_image.as_str()), ("host", "local", ""));
}

#[tokio::test]
async fn nothing_asked_applies_nothing() {
    let state = test_state();
    let mut def = test_agent_def("kept", "Kept", "claude", "container", 1, "");
    def.container_image = "tpl/image:2".into();
    state.mstore.agent_def_insert(&mut def).unwrap();
    apply(&state, &mut def, &Placement::AsSource).unwrap();
    let stored = state.mstore.agent_def_get(&def.id).unwrap().unwrap();
    assert_eq!((stored.agent_type.as_str(), stored.container_image.as_str()), ("container", "tpl/image:2"));
    assert_eq!(state.mstore.container_limits_get(&def.id).unwrap(), None);
}
