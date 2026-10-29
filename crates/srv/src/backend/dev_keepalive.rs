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

use agentmux_common::time::now_secs_u64;
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
const STARTUP_WAIT_SECS: u64 = 90;
const STARTUP_WAIT: Duration = Duration::from_secs(STARTUP_WAIT_SECS);
/// A server nobody has run a turn beside for this long is no longer kept up.
const FORGET_AFTER_SECS: u64 = 30 * 24 * 3600;

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
    /// When the container last ran a turn with this kept up (unix seconds): one nobody has used for a month is let go.
    #[serde(default)]
    pub seen: u64,
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
        let mut specs: Vec<Spec> = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        let now = now_secs_u64();
        specs.retain(|s| s.seen == 0 || now.saturating_sub(s.seen) < FORGET_AFTER_SECS);
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

    /// Whether `spec` can be kept: the container has room for it, or it replaces its project's.
    pub fn can_keep(&self, spec: &Spec) -> Result<(), String> {
        let specs = self.lock();
        let others = specs.iter().filter(|s| s.container == spec.container && s.project != spec.project).count();
        if others >= MAX_PER_CONTAINER {
            return Err(format!("at most {MAX_PER_CONTAINER} dev servers can be kept up per container"));
        }
        Ok(())
    }

    /// Keep `spec` up, replacing the one for the same project in the same container.
    pub fn keep(&self, mut spec: Spec) -> Result<(), String> {
        self.can_keep(&spec)?;
        spec.seen = now_secs_u64();
        let mut specs = self.lock();
        specs.retain(|s| !(s.container == spec.container && s.project == spec.project));
        specs.push(spec);
        self.save(&specs);
        Ok(())
    }

    /// `container` ran a turn: what is kept up in it is in use.
    fn touch(&self, container: &str) {
        let mut specs = self.lock();
        let now = now_secs_u64();
        for s in specs.iter_mut().filter(|s| s.container == container) {
            s.seen = now;
        }
        // And those nobody has used for a month: this runs on every turn, so a long-lived srv lets them go too.
        specs.retain(|s| s.seen == 0 || now.saturating_sub(s.seen) < FORGET_AFTER_SECS);
        self.save(&specs);
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

    /// Whether `spec` is still what is kept for its project: not dropped, and not replaced by another command or port.
    pub fn is_current(&self, spec: &Spec) -> bool {
        self.lock().iter().any(|s| {
            (&s.container, &s.project, s.port, &s.command, &s.dir) == (&spec.container, &spec.project, spec.port, &spec.command, &spec.dir)
        })
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

/// One start at a time per container, and a server started a moment ago is waited for, not started again.
fn start_lock(container: &str) -> std::sync::Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<Mutex<std::collections::HashMap<String, std::sync::Arc<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    LOCKS.get_or_init(Default::default).lock().unwrap_or_else(|p| p.into_inner()).entry(container.to_string()).or_default().clone()
}

/// When each `(container, project)` was last started, and in which incarnation of the container (its Docker id): a
/// recreated container has none of what ran in the old one. Tells a server that is slow to listen from one not running.
fn started_recently(container: &str, project: &str, incarnation: &str, set: bool) -> bool {
    static STARTED: OnceLock<Mutex<std::collections::HashMap<(String, String), (Instant, String)>>> = OnceLock::new();
    let mut started = STARTED.get_or_init(Default::default).lock().unwrap_or_else(|p| p.into_inner());
    let key = (container.to_string(), project.to_string());
    if set {
        started.insert(key, (Instant::now(), incarnation.to_string()));
        return true;
    }
    started.get(&key).is_some_and(|(at, id)| id == incarnation && at.elapsed() < STARTUP_WAIT)
}

/// Start `spec`'s command unless the server is listening already or was started within the startup wait.
pub async fn start(cm: &ContainerManager, spec: &Spec) -> Result<(), String> {
    let lock = start_lock(&spec.container);
    let _one_at_a_time = lock.lock().await;
    // The container's Docker id and when it last started: a stopped container that starts again keeps its id.
    let incarnation = cm
        .docker()
        .inspect_container(&spec.container, None)
        .await
        .ok()
        .map(|c| format!("{}@{}", c.id.unwrap_or_default(), c.state.and_then(|s| s.started_at).unwrap_or_default()))
        .unwrap_or_default();
    if cm.port_listening(&spec.container, spec.port).await || started_recently(&spec.container, &spec.project, &incarnation, false) {
        return Ok(());
    }
    let env = crate::backend::container_credential::watcher_polling_env(cfg!(any(windows, target_os = "macos")), &[]);
    cm.start_dev_server(&spec.container, &spec.command, &spec.dir, &env, &spec.log())
        .await
        .map_err(|e| format!("couldn't start {:?}: {e}", spec.command))?;
    started_recently(&spec.container, &spec.project, &incarnation, true);
    Ok(())
}

/// Whether `spec`'s server is listening within `wait`.
pub async fn wait_listening(cm: &ContainerManager, spec: &Spec, wait: Duration) -> bool {
    let deadline = Instant::now() + wait;
    loop {
        if cm.port_listening(&spec.container, spec.port).await {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Make sure `spec`'s server is listening: start it if it isn't, and wait up to the startup wait.
pub async fn bring_up(cm: &ContainerManager, spec: &Spec) -> Result<(), String> {
    start(cm, spec).await?;
    if wait_listening(cm, spec, STARTUP_WAIT).await {
        return Ok(());
    }
    Err(format!("{:?} didn't start listening on port {} within {}s; its output is in {} in the container", spec.command, spec.port, STARTUP_WAIT_SECS, spec.log()))
}

/// Register `spec`'s route once its server listens, in the background; for a server too slow to wait for in a request.
pub fn route_when_listening(cm: &ContainerManager, spec: Spec) {
    let cm = cm.clone();
    tokio::spawn(async move {
        if !wait_listening(&cm, &spec, STARTUP_WAIT).await {
            tracing::warn!(container = %spec.container, project = %spec.project, "[dev-keepalive] the dev server never listened; its output is in {}", spec.log());
            return;
        }
        match route_target(&cm, &spec.container, spec.port).await {
            // Only if the agent hasn't registered the project again, or dropped it, while this waited.
            Ok(target) if store().is_current(&spec) => {
                if let Some(registry) = cm.dev_proxy_registry() {
                    registry.register_from(&spec.project, &spec.agent_id, Some(&spec.container), target).await;
                }
            }
            Ok(_) => {}
            Err(e) => tracing::warn!(container = %spec.container, error = ?e, "[dev-keepalive] no route to the dev server"),
        }
    });
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
    store().touch(container);
    /// Clears the container's entry however the task ends.
    struct Done(String);
    impl Drop for Done {
        fn drop(&mut self) {
            RESTORING.get_or_init(Default::default).lock().unwrap_or_else(|p| p.into_inner()).remove(&self.0);
        }
    }
    let (cm, container) = (cm.clone(), container.to_string());
    tokio::spawn(async move {
        let _done = Done(container.clone());
        for spec in &specs {
            let routed = async {
                bring_up(&cm, spec).await?;
                let target = route_target(&cm, &spec.container, spec.port).await.map_err(|e| format!("{e:?}"))?;
                let registry = cm.dev_proxy_registry().ok_or("no dev proxy")?;
                // Only if it is still what the agent wants kept: it may have registered the project again while this ran.
                if store().is_current(spec) {
                    registry.register_from(&spec.project, &spec.agent_id, Some(&spec.container), target).await;
                }
                Ok::<(), String>(())
            }
            .await;
            match routed {
                Ok(()) => tracing::info!(container = %spec.container, project = %spec.project, port = spec.port, "[dev-keepalive] dev server up"),
                Err(e) => tracing::warn!(container = %spec.container, project = %spec.project, error = %e, "[dev-keepalive] couldn't bring the dev server up"),
            }
        }
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
            seen: 0,
        }
    }

    /// What a store holds for `container`, without the time each was last used.
    fn kept(store: &Store, container: &str) -> Vec<Spec> {
        store.for_container(container).into_iter().map(|s| Spec { seen: 0, ..s }).collect()
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
        assert_eq!(kept(&store, "c1"), vec![again], "the same project is replaced, not added");
        assert!(store.for_container("c2").is_empty());
        for i in 1..MAX_PER_CONTAINER {
            store.keep(spec("c1", &format!("p{i}"))).unwrap();
        }
        assert!(store.can_keep(&spec("c1", "one-too-many")).is_err(), "the cap is known before anything is started");
        assert!(store.can_keep(&spec("c1", "landing")).is_ok(), "replacing a project needs no room");
        assert!(store.keep(spec("c1", "one-too-many")).is_err());
        store.keep(spec("c2", "other")).unwrap();
        store.drop_project("c1", "landing");
        assert!(store.for_container("c1").iter().all(|s| s.project != "landing"));
        assert_eq!(store.for_container("c2").len(), 1, "another container's servers are untouched");
    }

    #[test]
    fn a_spec_is_current_only_until_the_project_is_registered_again_or_dropped() {
        let store = Store::open(None);
        store.keep(spec("c1", "landing")).unwrap();
        assert!(store.is_current(&spec("c1", "landing")));
        let mut moved = spec("c1", "landing");
        moved.port = 5000;
        store.keep(moved.clone()).unwrap();
        assert!(!store.is_current(&spec("c1", "landing")), "the old port is no longer wanted");
        assert!(store.is_current(&moved));
        store.drop_project("c1", "landing");
        assert!(!store.is_current(&moved));
    }

    #[test]
    fn what_is_kept_up_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dev-servers.json");
        let store = Store::open(Some(path.clone()));
        store.keep(spec("c1", "landing")).unwrap();
        let reopened = Store::open(Some(path.clone()));
        assert_eq!(kept(&reopened, "c1"), vec![spec("c1", "landing")]);
        reopened.drop_project("c1", "landing");
        assert!(Store::open(Some(path)).for_container("c1").is_empty());
        std::fs::write(dir.path().join("bad.json"), "not json").unwrap();
        assert!(Store::open(Some(dir.path().join("bad.json"))).for_container("c1").is_empty(), "a damaged file is no servers, not a crash");
    }

    #[test]
    fn a_server_nobody_has_run_a_turn_beside_for_a_month_is_let_go() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dev-servers.json");
        let (old, fresh) = (Spec { seen: now_secs_u64() - FORGET_AFTER_SECS - 10, ..spec("c1", "old") }, Spec { seen: now_secs_u64(), ..spec("c1", "fresh") });
        std::fs::write(&path, serde_json::to_vec(&[old, fresh]).unwrap()).unwrap();
        let store = Store::open(Some(path));
        assert_eq!(store.for_container("c1").iter().map(|s| s.project.as_str()).collect::<Vec<_>>(), ["fresh"]);
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
            seen: 0,
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
