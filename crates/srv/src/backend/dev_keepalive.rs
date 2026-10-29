// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Dev servers that outlive their container.
//!
//! A container agent's dev server is a process in its container, so it goes when the container is recreated
//! (an AgentMux upgrade, an image update). An agent that registers one with the command that starts it
//! (`RegisterDevServer` with `command`) has AgentMux start it and keep it up: when the container next runs a turn
//! and the server is not listening, srv runs the command again, detached, in the same folder, and registers the
//! route with the container's new address. What is remembered is the command the agent itself gave, and it only
//! ever runs in the container it was registered from.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::backend::container::ContainerManager;
use crate::backend::dev_proxy::Target;

/// The folder dev servers start in, and the container's name for it.
const WORKSPACE: &str = "/workspace";
/// What one container may keep up.
const MAX_PER_CONTAINER: usize = 8;
/// A command is a line or a short script.
const MAX_COMMAND_BYTES: usize = 4096;
/// How long a freshly started server gets to begin listening.
const STARTUP_WAIT: Duration = Duration::from_secs(90);

/// One dev server to keep up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spec {
    /// The agent id its route is under (`<project>-<agent_id>.localhost`).
    pub agent_id: String,
    /// The container it runs in.
    pub container: String,
    pub project: String,
    pub port: u16,
    pub command: String,
    /// Where the command runs, as a path in the container.
    pub dir: String,
}

impl Spec {
    /// Where the server's output goes, inside the container.
    pub fn log(&self) -> String {
        format!("/tmp/amx-dev-{}.log", self.project)
    }
}

/// The folder a command runs in: `/workspace/...`, or a path relative to it; nothing that leaves it.
pub fn clean_dir(dir: Option<&str>) -> Result<String, String> {
    let raw = dir.map(str::trim).filter(|d| !d.is_empty()).unwrap_or(WORKSPACE);
    let rel = match raw.strip_prefix(WORKSPACE) {
        Some("") => "",
        Some(rest) if rest.starts_with('/') => rest.trim_start_matches('/'),
        Some(_) => return Err(format!("dir {raw:?} is not in {WORKSPACE}")),
        None if raw.starts_with('/') => return Err(format!("dir {raw:?} is not in {WORKSPACE}")),
        None => raw,
    };
    if rel.split('/').any(|part| part == "..") || rel.contains('\0') {
        return Err(format!("dir {raw:?} leaves {WORKSPACE}"));
    }
    Ok(if rel.is_empty() { WORKSPACE.to_string() } else { format!("{WORKSPACE}/{}", rel.trim_end_matches('/')) })
}

/// A command worth running: not empty, and short enough to be one.
pub fn clean_command(command: &str) -> Result<String, String> {
    let command = command.trim();
    if command.is_empty() {
        return Err("command must not be empty".to_string());
    }
    if command.len() > MAX_COMMAND_BYTES || command.contains('\0') {
        return Err(format!("command must be text of at most {MAX_COMMAND_BYTES} bytes"));
    }
    Ok(command.to_string())
}

/// The dev servers kept up, in a file next to AgentMux's other shared state so they outlive srv too.
pub struct Store {
    path: Option<PathBuf>,
    specs: Mutex<Vec<Spec>>,
}

impl Store {
    /// The store in `path` (as left by an earlier run); in memory only when there is none.
    pub fn open(path: Option<PathBuf>) -> Self {
        let specs = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Self { path, specs: Mutex::new(specs) }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Spec>> {
        self.specs.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn save(&self, specs: &[Spec]) {
        let Some(path) = &self.path else { return };
        let tmp = path.with_extension("json.tmp");
        let written = serde_json::to_vec_pretty(specs).map_err(|e| e.to_string()).and_then(|bytes| {
            std::fs::create_dir_all(path.parent().unwrap_or(std::path::Path::new("."))).map_err(|e| e.to_string())?;
            std::fs::write(&tmp, bytes).and_then(|()| std::fs::rename(&tmp, path)).map_err(|e| e.to_string())
        });
        if let Err(e) = written {
            tracing::warn!(error = %e, "[dev-keepalive] couldn't save the dev servers kept up");
        }
    }

    /// Keep `spec` up, replacing the one for the same project in the same container.
    pub fn keep(&self, spec: Spec) -> Result<(), String> {
        let mut specs = self.lock();
        specs.retain(|s| !(s.container == spec.container && s.project == spec.project));
        if specs.iter().filter(|s| s.container == spec.container).count() >= MAX_PER_CONTAINER {
            return Err(format!("at most {MAX_PER_CONTAINER} dev servers can be kept up per container"));
        }
        specs.push(spec);
        self.save(&specs);
        Ok(())
    }

    /// Stop keeping `project` up in `container` (it was registered again without a command).
    pub fn drop_project(&self, container: &str, project: &str) {
        let mut specs = self.lock();
        let before = specs.len();
        specs.retain(|s| !(s.container == container && s.project == project));
        if specs.len() != before {
            self.save(&specs);
        }
    }

    pub fn for_container(&self, container: &str) -> Vec<Spec> {
        self.lock().iter().filter(|s| s.container == container).cloned().collect()
    }
}

/// The store every dev server is kept in.
pub fn store() -> &'static Store {
    static STORE: OnceLock<Store> = OnceLock::new();
    STORE.get_or_init(|| Store::open(agentmux_common::DataPaths::from_env().map(|p| p.shared_dir.join("dev-servers.json"))))
}

/// Why a route to a container's dev server can't be made.
#[derive(Debug)]
pub enum RouteError {
    /// No running container of that name on the agents network.
    NoContainer,
    /// The relay a Docker Desktop host reaches containers through isn't available.
    Relay(String),
    /// The container's address isn't one.
    Address(String),
}

/// How the proxy reaches `port` in `container`: its address on the agents network, or through the relay where the
/// daemon runs in a VM and that address isn't routable from here.
pub async fn route_target(cm: &ContainerManager, container: &str, port: u16) -> Result<Target, RouteError> {
    let ip = cm.agent_network_ip(container).await.ok_or(RouteError::NoContainer)?;
    if crate::backend::dev_relay::needed(cm).await {
        let image = cm.docker().inspect_container(container, None).await.ok().and_then(|c| c.config?.image).unwrap_or_default();
        let via = crate::backend::dev_relay::ensure(cm, &image).await.map_err(RouteError::Relay)?;
        return Ok(Target::Relay { via, container: container.to_string(), port });
    }
    format!("{ip}:{port}").parse::<std::net::SocketAddr>().map(Target::Direct).map_err(|e| RouteError::Address(format!("{ip}:{port}: {e}")))
}

/// Make sure `spec`'s server is listening: if it isn't, start its command and wait for it to begin.
pub async fn bring_up(cm: &ContainerManager, spec: &Spec) -> Result<(), String> {
    if cm.port_listening(&spec.container, spec.port).await {
        return Ok(());
    }
    let env = crate::backend::container_credential::watcher_polling_env(cfg!(any(windows, target_os = "macos")), &[]);
    cm.start_dev_server(&spec.container, &spec.command, &spec.dir, &env, &spec.log())
        .await
        .map_err(|e| format!("couldn't start {:?}: {e}", spec.command))?;
    let deadline = Instant::now() + STARTUP_WAIT;
    while Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if cm.port_listening(&spec.container, spec.port).await {
            return Ok(());
        }
    }
    Err(format!("{:?} didn't start listening on port {} within {}s; its output is in {} in the container", spec.command, spec.port, STARTUP_WAIT.as_secs(), spec.log()))
}

/// Start the dev servers kept up in `container` that aren't listening, and register their routes, without holding
/// up the caller. Called when the container is ready for a turn.
pub fn restore_in_background(cm: &ContainerManager, container: &str) {
    static RESTORING: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let specs = store().for_container(container);
    if specs.is_empty() {
        return;
    }
    let busy = RESTORING.get_or_init(Default::default);
    if !busy.lock().unwrap_or_else(|p| p.into_inner()).insert(container.to_string()) {
        return;
    }
    let (cm, container) = (cm.clone(), container.to_string());
    tokio::spawn(async move {
        for spec in &specs {
            let routed = async {
                bring_up(&cm, spec).await?;
                let target = route_target(&cm, &spec.container, spec.port).await.map_err(|e| format!("{e:?}"))?;
                let registry = cm.dev_proxy_registry().ok_or("no dev proxy")?;
                registry.register_from(&spec.project, &spec.agent_id, Some(&spec.container), target).await;
                Ok::<(), String>(())
            }
            .await;
            match routed {
                Ok(()) => tracing::info!(container = %spec.container, project = %spec.project, port = spec.port, "[dev-keepalive] dev server up"),
                Err(e) => tracing::warn!(container = %spec.container, project = %spec.project, error = %e, "[dev-keepalive] couldn't bring the dev server up"),
            }
        }
        busy.lock().unwrap_or_else(|p| p.into_inner()).remove(&container);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(container: &str, project: &str) -> Spec {
        Spec {
            agent_id: "kcont".into(),
            container: container.into(),
            project: project.into(),
            port: 4600,
            command: "npm run dev".into(),
            dir: "/workspace/landing".into(),
        }
    }

    #[test]
    fn a_dir_stays_in_the_workspace() {
        assert_eq!(clean_dir(None).unwrap(), "/workspace");
        assert_eq!(clean_dir(Some("  ")).unwrap(), "/workspace");
        assert_eq!(clean_dir(Some("/workspace")).unwrap(), "/workspace");
        assert_eq!(clean_dir(Some("landing")).unwrap(), "/workspace/landing");
        assert_eq!(clean_dir(Some("/workspace/landing/")).unwrap(), "/workspace/landing");
        for bad in ["/etc", "/workspacefoo", "../x", "/workspace/../etc", "a/../../b"] {
            assert!(clean_dir(Some(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_command_is_text_of_a_sensible_size() {
        assert_eq!(clean_command("  npm run dev \n").unwrap(), "npm run dev");
        assert!(clean_command("   ").is_err());
        assert!(clean_command(&"x".repeat(MAX_COMMAND_BYTES + 1)).is_err());
        assert!(clean_command("a\0b").is_err());
    }

    #[test]
    fn a_container_keeps_a_project_once_and_only_so_many() {
        let store = Store::open(None);
        store.keep(spec("c1", "landing")).unwrap();
        let mut again = spec("c1", "landing");
        again.command = "npm start".into();
        store.keep(again.clone()).unwrap();
        assert_eq!(store.for_container("c1"), vec![again], "the same project is replaced, not added");
        assert!(store.for_container("c2").is_empty());
        for i in 1..MAX_PER_CONTAINER {
            store.keep(spec("c1", &format!("p{i}"))).unwrap();
        }
        assert!(store.keep(spec("c1", "one-too-many")).is_err());
        store.keep(spec("c2", "other")).unwrap();
        store.drop_project("c1", "landing");
        assert!(store.for_container("c1").iter().all(|s| s.project != "landing"));
        assert_eq!(store.for_container("c2").len(), 1, "another container's servers are untouched");
    }

    #[test]
    fn what_is_kept_up_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dev-servers.json");
        let store = Store::open(Some(path.clone()));
        store.keep(spec("c1", "landing")).unwrap();
        let reopened = Store::open(Some(path.clone()));
        assert_eq!(reopened.for_container("c1"), vec![spec("c1", "landing")]);
        reopened.drop_project("c1", "landing");
        assert!(Store::open(Some(path)).for_container("c1").is_empty());
        std::fs::write(dir.path().join("bad.json"), "not json").unwrap();
        assert!(Store::open(Some(dir.path().join("bad.json"))).for_container("c1").is_empty(), "a damaged file is no servers, not a crash");
    }

    /// Against a real daemon: a server is started for a container, and started again in a recreated one.
    ///   cargo test -p agentmux-srv --bin agentmux-srv -- --ignored --nocapture itest_dev_keepalive
    #[tokio::test]
    #[ignore]
    async fn itest_dev_keepalive_starts_the_server_again_in_a_recreated_container() {
        use bollard::container::{Config, CreateContainerOptions, RemoveContainerOptions, StartContainerOptions};
        use bollard::models::HostConfig;
        let cm = ContainerManager::connect().expect("connect to docker (set DOCKER_HOST)");
        cm.check_available().await.expect("docker daemon must be reachable");
        let docker = cm.docker();
        let (image, name) = ("ghcr.io/agentmuxai/agent-base:0.60.3", "agentmux-itest-keepalive");
        let network = crate::backend::container::AGENTMUX_DOCKER_NETWORK;
        let _ = docker.create_network(bollard::network::CreateNetworkOptions { name: network, driver: "bridge", ..Default::default() }).await;
        async fn fresh(docker: &bollard::Docker, image: &str, name: &str, network: &str) {
            let _ = docker.remove_container(name, Some(RemoveContainerOptions { force: true, ..Default::default() })).await;
            docker
                .create_container(
                    Some(CreateContainerOptions { name, platform: None }),
                    Config {
                        image: Some(image.to_string()),
                        cmd: Some(vec!["sleep".into(), "300".into()]),
                        user: Some("agent".into()),
                        host_config: Some(HostConfig { network_mode: Some(network.to_string()), ..Default::default() }),
                        ..Default::default()
                    },
                )
                .await
                .expect("create");
            docker.start_container(name, None::<StartContainerOptions<String>>).await.expect("start");
        }
        let spec = Spec {
            agent_id: "itest".into(),
            container: name.into(),
            project: "itest".into(),
            port: 7000,
            command: "node -e \"require('http').createServer((q,r)=>r.end('hi')).listen(7000,'0.0.0.0')\"".into(),
            dir: "/tmp".into(),
        };
        fresh(docker, image, name, network).await;
        assert!(!cm.port_listening(name, 7000).await);
        bring_up(&cm, &spec).await.expect("started");
        assert!(cm.port_listening(name, 7000).await);
        bring_up(&cm, &spec).await.expect("already up: nothing is started twice");
        // The container is recreated: the server is gone, and comes back.
        fresh(docker, image, name, network).await;
        assert!(!cm.port_listening(name, 7000).await, "a recreated container starts empty");
        bring_up(&cm, &spec).await.expect("started again");
        assert!(cm.port_listening(name, 7000).await);
        let _ = docker.remove_container(name, Some(RemoveContainerOptions { force: true, ..Default::default() })).await;
    }
}
