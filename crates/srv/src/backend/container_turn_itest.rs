//! Live check of a container agent's turn on Linux, against a real Docker daemon:
//!   cargo test -p agentmux-srv --bins -- --ignored --nocapture itest_container_turn
//!
//! The provider CLI is a stub: a shell script that reads the message from stdin, announces a
//! session like a provider's init event, and calls a MuxAPI route over `$AGENTMUX_LOCAL_URL`
//! with the token it was given. What the stub saw is read back out of the container.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt as _;

use crate::backend::blockcontroller::subprocess::{SubprocessController, SubprocessSpawnConfig};
use crate::backend::container::{ContainerManager, ContainerMountSpec};
use crate::backend::container_listener;

/// What the stub CLI does with its turn.
const STUB: &str = r#"
read -r message
echo '{"type":"system","subtype":"init","session_id":"stub-session"}'
wget -q -O /tmp/stub-self.json --header "X-AuthKey: $AGENTMUX_AUTH_KEY" "$AGENTMUX_LOCAL_URL/api/v1/self?block_id=$AGENTMUX_BLOCK_ID"
echo "wget=$?" > /tmp/stub-result
echo "key=$AGENTMUX_AUTH_KEY" >> /tmp/stub-result
echo "url=$AGENTMUX_LOCAL_URL" >> /tmp/stub-result
echo "heard=$message" >> /tmp/stub-result
echo '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"pong"}]}}'
echo '{"type":"result","subtype":"success","is_error":false,"result":"pong"}'
touch /tmp/stub-done
"#;

async fn in_container(cm: &ContainerManager, name: &str, script: &str) -> String {
    // Empty while the turn's first step has not created the container yet.
    let Ok(session) = cm.exec(name, &["sh".into(), "-c".into(), script.into()], None, &[], false).await else {
        return String::new();
    };
    drop(session.input);
    let mut out = String::new();
    let mut stream = session.output;
    while let Some(Ok(chunk)) = stream.next().await {
        out.push_str(&String::from_utf8_lossy(&chunk.into_bytes()));
    }
    out
}

#[tokio::test]
#[ignore]
async fn itest_container_turn_answers_and_calls_a_muxapi_tool() {
    let state = crate::server::tests::test_state();
    let instance_key = state.auth_key.clone();
    let tab = state.mstore.get_all::<crate::backend::obj::Tab>().unwrap().into_iter().next().expect("seeded tab");
    let block_id = tab.blockids.first().expect("seeded agent block").clone();
    container_listener::set_router(crate::server::build_routers(state).container);

    let cm = ContainerManager::connect().expect("connect to docker (set DOCKER_HOST)");
    cm.check_available().await.expect("docker daemon must be reachable");
    let name = "agentmux-itest-turn";
    let _ = cm.remove(name, true).await;
    assert!(container_listener::url().is_none(), "nothing is listening before the first container starts");

    // What a host agent's launch hands the container turn: srv's loopback address and the instance
    // key, which the turn swaps for a container token and a reachable address.
    let env_vars: HashMap<String, String> = [
        ("AGENTMUX_AUTH_KEY", instance_key.as_str()),
        ("AGENTMUX_LOCAL_URL", "http://127.0.0.1:1"),
        ("AGENTMUX_BLOCK_ID", block_id.as_str()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    let config = SubprocessSpawnConfig {
        session_id_field: "session_id".into(),
        message: "ping".into(),
        env_vars,
        ..Default::default()
    };
    let controller = SubprocessController::new("tab".into(), block_id.clone(), None, None, None, None, None, Arc::from("itest"));
    // The container starts as the turn's first step, as for a first message after srv restarts.
    let starter = cm.clone();
    controller.set_container_startup(Arc::new(move || {
        let cm = starter.clone();
        Box::pin(async move {
            cm.ensure_running_observed("agentmux-itest-turn", "nginx:alpine", &[], &[], &ContainerMountSpec::default(), &|_| {})
                .await
                .map_err(|e| e.to_string())
        })
    }));
    controller
        .spawn_container_turn(cm.clone(), name.to_string(), vec!["sh".into(), "-c".into(), STUB.into(), "stub".into()], config)
        .expect("spawn_container_turn");

    let mut done = false;
    for _ in 0..60 {
        if in_container(&cm, name, "test -e /tmp/stub-done && echo yes").await.contains("yes") {
            done = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let result = in_container(&cm, name, "cat /tmp/stub-result").await;
    let self_json = in_container(&cm, name, "cat /tmp/stub-self.json").await;
    println!("{result}\n{self_json}");
    let session = controller.session_id();
    let listener_url = container_listener::url().expect("the listener binds the agents network's gateway on a Linux Docker host");
    let _ = cm.remove(name, true).await;

    assert!(done, "the stub CLI finished its turn");
    assert!(result.contains("wget=0"), "the MuxAPI call succeeded: {result}");
    assert!(result.contains("heard=ping"), "the message reached the CLI on stdin: {result}");
    assert!(result.contains(&format!("url={listener_url}")), "the CLI was pointed at the listener: {result}");
    let key = result.lines().find_map(|l| l.strip_prefix("key=")).unwrap_or_default();
    assert!(key.starts_with("amxc_") && key != instance_key, "the CLI holds a container token, not the instance key");
    assert!(self_json.contains(&block_id), "the tool answered for this agent's block: {self_json}");
    assert_eq!(session.as_deref(), Some("stub-session"), "srv read the session out of the container's output");
}
