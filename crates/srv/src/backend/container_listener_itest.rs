//! Live check of the container listener against a real Docker daemon:
//!   cargo test -p agentmux-srv --bins -- --ignored --nocapture itest_container_listener
//!
//! A container on `agentmux-agents` reaches the real router through the network's gateway with
//! its container token, and is refused with anything else.

use futures_util::StreamExt as _;

use crate::backend::container::{ContainerManager, ContainerMountSpec};
use crate::backend::container_credential::token_for_block;
use crate::backend::container_listener;

/// The HTTP status `wget` inside the container got, or what it said when it got none.
async fn status(cm: &ContainerManager, name: &str, url: &str, key: Option<&str>) -> String {
    let script = r#"wget -S -O /dev/null --header "X-AuthKey: $KEY" "$URL" 2>&1 | grep -m1 -E 'HTTP/|wget:'"#;
    let env = [("URL".to_string(), url.to_string()), ("KEY".to_string(), key.unwrap_or("").to_string())];
    let session = cm.exec(name, &["sh".into(), "-c".into(), script.into()], None, &env, false).await.expect("exec");
    drop(session.input);
    let mut out = String::new();
    let mut stream = session.output;
    while let Some(Ok(chunk)) = stream.next().await {
        out.push_str(&String::from_utf8_lossy(&chunk.into_bytes()));
    }
    let line = out.trim().to_string();
    match line.split("HTTP/").nth(1).and_then(|rest| rest.split_whitespace().nth(1)) {
        Some(code) => code.to_string(),
        None => line,
    }
}

#[tokio::test]
#[ignore]
async fn itest_container_listener_serves_the_agents_network_only() {
    let state = crate::server::tests::test_state();
    let instance_key = state.auth_key.clone();
    container_listener::set_router(crate::server::build_routers(state).container);

    let cm = ContainerManager::connect().expect("connect to docker (set DOCKER_HOST)");
    cm.check_available().await.expect("docker daemon must be reachable");
    let name = "agentmux-itest-listener";
    let _ = cm.remove(name, true).await;
    cm.ensure_running_observed(name, "nginx:alpine", &[], &[], &ContainerMountSpec::default(), &|_| {}).await.expect("ensure_running");

    // A second start with the same network reuses the listener (it once deadlocked here).
    let first = container_listener::url();
    tokio::time::timeout(std::time::Duration::from_secs(30), cm.ensure_running_observed(name, "nginx:alpine", &[], &[], &ContainerMountSpec::default(), &|_| {}))
        .await
        .expect("a second ensure_running must not hang")
        .expect("ensure_running (reuse)");
    assert_eq!(container_listener::url(), first, "the listener is reused, not rebound");

    let base = container_listener::url().expect("the listener binds the agents network's gateway on a Linux Docker host");
    let token = token_for_block(name, None);
    let got = [
        ("a container token on an agent route", status(&cm, name, &format!("{base}/agentmux/discovery"), Some(&token)).await),
        ("a container token on an operator route", status(&cm, name, &format!("{base}/agentmux/service"), Some(&token)).await),
        ("a wrong token", status(&cm, name, &format!("{base}/agentmux/discovery"), Some("amxc_wrong")).await),
        ("the instance key", status(&cm, name, &format!("{base}/agentmux/discovery"), Some(&instance_key)).await),
        ("no key", status(&cm, name, &format!("{base}/agentmux/discovery"), None).await),
    ];
    for (what, code) in &got {
        println!("{what}: {code}");
    }
    let port = base.rsplit(':').next().unwrap().to_string();
    let loopback = tokio::net::TcpStream::connect(format!("127.0.0.1:{port}")).await;
    println!("loopback: {loopback:?}");
    let _ = cm.remove(name, true).await;

    assert_eq!(got[0].1, "200", "{}", got[0].0);
    assert_eq!(got[1].1, "403", "{}", got[1].0);
    assert_eq!(got[2].1, "401", "{}", got[2].0);
    assert_eq!(got[3].1, "401", "{}", got[3].0);
    assert_eq!(got[4].1, "401", "{}", got[4].0);
    assert!(loopback.is_err(), "the listener must not answer on loopback");
}
