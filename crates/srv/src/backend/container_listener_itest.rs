//! Live check of the container listener against a real Docker daemon:
//!   cargo test -p agentmux-srv --bins -- --ignored --nocapture itest_container_listener
//!
//! A container on `agentmux-agents` reaches the real router through the network's gateway with
//! its container token, and is refused with anything else.

use std::sync::OnceLock;

use futures_util::StreamExt as _;

use crate::backend::container::{ContainerManager, ContainerMountSpec};
use crate::backend::container_credential::token_for_block;
use crate::backend::container_listener;

/// What the Docker tests share: they use the process-wide router and listener, whose server task
/// lives on the runtime that bound it. So they run one at a time on one state, and each starts
/// from an unbound listener and unbinds again before its runtime ends.
pub(super) struct Shared {
    pub instance_key: String,
    pub block_id: String,
}

pub(super) async fn begin() -> (tokio::sync::MutexGuard<'static, ()>, &'static Shared) {
    static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    static SHARED: OnceLock<Shared> = OnceLock::new();
    let guard = SERIAL.lock().await;
    let shared = SHARED.get_or_init(|| {
        let state = crate::server::tests::test_state();
        let instance_key = state.auth_key.clone();
        let tab = state.mstore.get_all::<crate::backend::obj::Tab>().unwrap().into_iter().next().expect("seeded tab");
        let block_id = tab.blockids.first().expect("seeded agent block").clone();
        container_listener::set_router(crate::server::build_routers(state).container);
        Shared { instance_key, block_id }
    });
    container_listener::unbind();
    (guard, shared)
}

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
    let (_serial, shared) = begin().await;
    let instance_key = shared.instance_key.clone();

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
    // An environment built before the container started is repointed once it has.
    let mut stale = vec![("AGENTMUX_LOCAL_URL".to_string(), "http://host.docker.internal:1".to_string())];
    container_listener::point_env_at_listener(&mut stale);
    assert_eq!(stale[0].1, base);
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
    container_listener::unbind();

    assert_eq!(got[0].1, "200", "{}", got[0].0);
    assert_eq!(got[1].1, "403", "{}", got[1].0);
    assert_eq!(got[2].1, "401", "{}", got[2].0);
    assert_eq!(got[3].1, "401", "{}", got[3].0);
    assert_eq!(got[4].1, "401", "{}", got[4].0);
    assert!(loopback.is_err(), "the listener must not answer on loopback");
}
