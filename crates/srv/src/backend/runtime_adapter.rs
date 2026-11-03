// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Where an agent runs, behind one trait.
//!
//! An agent is a process on this machine (`NativeAdapter`) or a `docker exec` in its container
//! (`ContainerAdapter`). What differs between the two used to be answered at each place that asked
//! "is this a container agent?" (the env a turn gets, how a container reaches srv, which controller
//! may host a provider, whether attachments work, how a turn is launched). Each answer is now a
//! method here, so a new runtime (WSL, remote) is one more impl and the callers do not change.
//!
//! The methods:
//! - [`credentials`](RuntimeAdapter::credentials): the env a turn's process is given, with the
//!   instance key replaced by a scoped token where the runtime is not trusted with it.
//! - [`srv_endpoint`](RuntimeAdapter::srv_endpoint): how the process reaches this srv.
//! - [`map_mount`](RuntimeAdapter::map_mount): where a folder the host knows by role appears.
//! - [`tool_path`](RuntimeAdapter::tool_path): how a helper program is named in config files.
//! - [`materialize`](RuntimeAdapter::materialize): what a turn does before it runs (a container is
//!   started and the CLI installed in it).
//! - [`launch`](RuntimeAdapter::launch): start one turn.
//!
//! [`RuntimeKind`] is the one place `agentMode` / `agent_type` is read as "host or container".

use std::collections::HashMap;

use super::blockcontroller::subprocess::{StartupStep, SubprocessController, SubprocessSpawnConfig};
use super::container::{ContainerManager, CONTAINER_CLAUDE_DIR, CONTAINER_WORKSPACE_DIR};
use super::providers::ControllerType;

/// The value of `agentMode` (a pane) and `agent_type` (a definition) that names a container agent.
const CONTAINER: &str = "container";

/// Where an agent runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeKind {
    /// A process on this machine (`"host"`, the legacy `"standalone"`, empty or unknown).
    Host,
    /// A Docker container.
    Container,
}

impl RuntimeKind {
    /// The runtime of a pane's `agentMode`: `"container"` is a container, anything else is the host,
    /// as srv has always read it.
    pub fn of_agent_mode(agent_mode: &str) -> Self {
        if agent_mode == CONTAINER {
            Self::Container
        } else {
            Self::Host
        }
    }

    /// The runtime of a definition's `agent_type`. The same values as `agentMode`.
    pub fn of_agent_type(agent_type: &str) -> Self {
        Self::of_agent_mode(agent_type)
    }

    /// The runtime of a pane, from its block meta.
    pub fn of_block_meta(meta: &super::obj::MetaMapType) -> Self {
        Self::of_agent_mode(&super::obj::meta_get_string(meta, "agentMode", "host"))
    }

    pub fn is_container(self) -> bool {
        self == Self::Container
    }

    /// The adapter for the parts that need no per-agent state. A container adapter made here cannot
    /// [`launch`](RuntimeAdapter::launch); build one with [`ContainerAdapter::with_launch`] for that.
    pub fn adapter(self) -> Box<dyn RuntimeAdapter> {
        match self {
            Self::Host => Box::new(NativeAdapter),
            Self::Container => Box::new(ContainerAdapter::default()),
        }
    }
}

/// A folder the host knows by what it is for, not by its path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MountRole {
    /// The agent's working directory.
    Workspace,
    /// The folder the CLI keeps the agent's Personal Memory in.
    PersonalMemory,
    /// The CLI's own config folder (`CLAUDE_CONFIG_DIR`).
    CliHome,
}

/// A helper program AgentMux puts beside the agent's CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolId {
    /// The MCP server (`.mcp.json`'s `agentmux` entry).
    Mcp,
    /// The Bash wrapper the hooks name.
    Bashwrap,
}

pub trait RuntimeAdapter: Send + Sync {
    fn kind(&self) -> RuntimeKind;

    /// The env a turn's process is given, from the env the block resolved. `block_id` names whose
    /// scoped token a runtime that uses one hands out.
    fn credentials(&self, block_id: &str, env: &HashMap<String, String>) -> Vec<(String, String)>;

    /// `AGENTMUX_LOCAL_URL` as the process sees it, given how this srv sees itself.
    fn srv_endpoint(&self, local_url: &str) -> String;

    /// Where `host_path`, the folder of `role`, appears in the runtime.
    fn map_mount(&self, role: MountRole, host_path: &str) -> String;

    /// How `tool` is named in the config files written for the agent.
    fn tool_path(&self, tool: ToolId) -> String;

    /// What a turn does before it runs. Call before [`launch`](Self::launch).
    fn materialize(&self, ctrl: &SubprocessController);

    /// Start one turn. `config` is in the host's shape (`cli_command`, `cli_args`, `working_dir`);
    /// a runtime that runs the CLI elsewhere turns that into its own.
    fn launch(&self, ctrl: &SubprocessController, config: SubprocessSpawnConfig) -> Result<(), String>;

    /// Whether the `.mcp.json` written for the agent carries its signing keys. A runtime that mounts
    /// the file where other code reads it does not.
    fn carries_signing_keys(&self) -> bool;

    /// Whether an attachment's host path means anything to the agent.
    fn accepts_attachments(&self) -> bool;

    /// Whether a long-lived (persistent) controller can host the agent.
    fn accepts_persistent_controller(&self) -> bool;

    /// Whether a provider run by `controller` can run here.
    fn can_host(&self, controller: ControllerType) -> bool;

    /// The controller type a new pane gets, given the provider's own.
    fn controller_type_for_open(&self, provider_default: &'static str) -> &'static str;
}

/// An agent that is a process on this machine.
#[derive(Debug, Clone, Copy, Default)]
pub struct NativeAdapter;

impl RuntimeAdapter for NativeAdapter {
    fn kind(&self) -> RuntimeKind {
        RuntimeKind::Host
    }

    fn credentials(&self, _block_id: &str, env: &HashMap<String, String>) -> Vec<(String, String)> {
        env.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }

    fn srv_endpoint(&self, local_url: &str) -> String {
        local_url.to_string()
    }

    fn map_mount(&self, _role: MountRole, host_path: &str) -> String {
        host_path.to_string()
    }

    fn tool_path(&self, tool: ToolId) -> String {
        bare_tool_name(tool).to_string()
    }

    fn materialize(&self, _ctrl: &SubprocessController) {}

    fn launch(&self, ctrl: &SubprocessController, config: SubprocessSpawnConfig) -> Result<(), String> {
        ctrl.spawn_turn(config)
    }

    fn carries_signing_keys(&self) -> bool {
        true
    }

    fn accepts_attachments(&self) -> bool {
        true
    }

    fn accepts_persistent_controller(&self) -> bool {
        true
    }

    fn can_host(&self, _controller: ControllerType) -> bool {
        true
    }

    fn controller_type_for_open(&self, provider_default: &'static str) -> &'static str {
        provider_default
    }
}

/// Both runtimes name a helper by its bare name today: the image has them on its `PATH`, and a host
/// pane finds them on its own.
fn bare_tool_name(tool: ToolId) -> &'static str {
    match tool {
        ToolId::Mcp => "agentmux-mcp",
        ToolId::Bashwrap => "agentmux-bashwrap",
    }
}

/// What a container turn needs that the pane's meta and the services give it.
#[derive(Clone)]
pub struct ContainerLaunch {
    pub cm: ContainerManager,
    pub container_name: String,
    /// The provider CLI as the image names it (`claude`), not the host's resolved path.
    pub container_command: String,
    pub provider_id: String,
    /// Starts the container and installs the CLI in it, at the start of the turn's own task.
    pub startup: StartupStep,
}

/// An agent in a Docker container, one `docker exec` per turn.
#[derive(Clone, Default)]
pub struct ContainerAdapter {
    launch: Option<ContainerLaunch>,
}

impl ContainerAdapter {
    pub fn with_launch(launch: ContainerLaunch) -> Self {
        Self { launch: Some(launch) }
    }
}

impl RuntimeAdapter for ContainerAdapter {
    fn kind(&self) -> RuntimeKind {
        RuntimeKind::Container
    }

    fn credentials(&self, block_id: &str, env: &HashMap<String, String>) -> Vec<(String, String)> {
        super::container_credential::container_exec_env(block_id, env)
    }

    /// A container can't reach the loopback address srv sees.
    fn srv_endpoint(&self, local_url: &str) -> String {
        super::container::rewrite_local_url_for_container(local_url)
    }

    fn map_mount(&self, role: MountRole, host_path: &str) -> String {
        let _ = host_path;
        match role {
            MountRole::Workspace => CONTAINER_WORKSPACE_DIR.to_string(),
            MountRole::PersonalMemory => super::container::container_memory_dir(),
            MountRole::CliHome => CONTAINER_CLAUDE_DIR.to_string(),
        }
    }

    fn tool_path(&self, tool: ToolId) -> String {
        bare_tool_name(tool).to_string()
    }

    fn materialize(&self, ctrl: &SubprocessController) {
        if let Some(launch) = &self.launch {
            ctrl.set_container_startup(launch.startup.clone());
        }
    }

    fn launch(&self, ctrl: &SubprocessController, mut config: SubprocessSpawnConfig) -> Result<(), String> {
        let launch = self.launch.as_ref().ok_or("this container adapter was made without a container to launch in")?;
        // The container runs the image's own CLI with its own working directory: the host's resolved
        // command and path mean nothing there.
        let mut base_cmd = vec![launch.container_command.clone()];
        base_cmd.extend(super::container_argv::container_argv(std::mem::take(&mut config.cli_args), &launch.provider_id));
        config.cli_command = String::new();
        config.working_dir = String::new();
        ctrl.spawn_container_turn(launch.cm.clone(), launch.container_name.clone(), base_cmd, config)
    }

    fn carries_signing_keys(&self) -> bool {
        false
    }

    fn accepts_attachments(&self) -> bool {
        false
    }

    /// A container turn is one `docker exec`; a long-lived stdin has nothing to attach to.
    fn accepts_persistent_controller(&self) -> bool {
        false
    }

    /// A terminal or an MSP session runs on the host.
    fn can_host(&self, controller: ControllerType) -> bool {
        !matches!(controller, ControllerType::Terminal | ControllerType::Msp)
    }

    /// Per-turn `docker exec` needs the subprocess controller, whatever the provider defaults to (Claude's
    /// is persistent, which would skip the container exec path).
    fn controller_type_for_open(&self, _provider_default: &'static str) -> &'static str {
        "subprocess"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn get(out: &[(String, String)], key: &str) -> Option<String> {
        out.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
    }

    #[test]
    fn only_container_names_a_container() {
        assert_eq!(RuntimeKind::of_agent_mode("container"), RuntimeKind::Container);
        assert_eq!(RuntimeKind::of_agent_type("container"), RuntimeKind::Container);
        for other in ["host", "standalone", "", "Container", "docker"] {
            assert_eq!(RuntimeKind::of_agent_mode(other), RuntimeKind::Host, "{other:?}");
        }
        assert!(RuntimeKind::Container.adapter().kind().is_container());
        assert!(!RuntimeKind::Host.adapter().kind().is_container());
    }

    /// The adapter answers what `container_exec_env` answered, so the live tests that go through
    /// the env see no change.
    #[test]
    fn the_container_adapters_credentials_are_the_exec_env() {
        let vars = env(&[("AGENTMUX_AUTH_KEY", "instance-key"), ("AGENTMUX_AGENT_TOKEN", "agent-tok"), ("FOO", "bar")]);
        let adapter = RuntimeKind::Container.adapter();
        let mut got = adapter.credentials("ra-block-a", &vars);
        let mut want = super::super::container_credential::container_exec_env("ra-block-a", &vars);
        got.sort();
        want.sort();
        assert_eq!(got, want);
        let token = get(&got, "AGENTMUX_AUTH_KEY").unwrap();
        assert!(token.starts_with(super::super::container_credential::TOKEN_PREFIX), "the instance key never enters a container");
        assert_eq!(get(&got, "AGENTMUX_IN_CONTAINER").as_deref(), Some("1"));
        assert_eq!(get(&got, "FOO").as_deref(), Some("bar"));
    }

    #[test]
    fn the_native_adapter_hands_the_env_over_unchanged() {
        let vars = env(&[("AGENTMUX_AUTH_KEY", "instance-key"), ("CLAUDE_CONFIG_DIR", "/home/u/.claude-a")]);
        let mut got = RuntimeKind::Host.adapter().credentials("ra-block-b", &vars);
        got.sort();
        assert_eq!(
            got,
            vec![
                ("AGENTMUX_AUTH_KEY".to_string(), "instance-key".to_string()),
                ("CLAUDE_CONFIG_DIR".to_string(), "/home/u/.claude-a".to_string()),
            ]
        );
    }

    #[test]
    fn a_container_reaches_srv_by_the_docker_host_name_and_a_process_by_loopback() {
        let url = "http://127.0.0.1:51234";
        assert_eq!(RuntimeKind::Host.adapter().srv_endpoint(url), url);
        assert_eq!(RuntimeKind::Container.adapter().srv_endpoint(url), "http://host.docker.internal:51234");
        // A name that is not loopback passes through.
        assert_eq!(RuntimeKind::Container.adapter().srv_endpoint("http://srv.example:9"), "http://srv.example:9");
    }

    #[test]
    fn mounts_keep_their_host_path_natively_and_appear_at_the_images_paths_in_a_container() {
        let native = RuntimeKind::Host.adapter();
        let container = RuntimeKind::Container.adapter();
        for role in [MountRole::Workspace, MountRole::PersonalMemory, MountRole::CliHome] {
            assert_eq!(native.map_mount(role, "/home/u/work"), "/home/u/work");
        }
        assert_eq!(container.map_mount(MountRole::Workspace, "/home/u/work"), "/workspace");
        assert_eq!(container.map_mount(MountRole::CliHome, "/home/u/.claude-a"), "/home/agent/.claude");
        assert_eq!(
            container.map_mount(MountRole::PersonalMemory, "/home/u/.claude-a/projects/x/memory"),
            "/home/agent/.claude/projects/-workspace/memory"
        );
    }

    #[test]
    fn both_runtimes_name_the_helpers_by_their_bare_names_for_now() {
        for kind in [RuntimeKind::Host, RuntimeKind::Container] {
            let adapter = kind.adapter();
            assert_eq!(adapter.tool_path(ToolId::Mcp), "agentmux-mcp");
            assert_eq!(adapter.tool_path(ToolId::Bashwrap), "agentmux-bashwrap");
        }
    }

    /// What the scattered `if container` branches decided, in one table.
    #[test]
    fn the_capabilities_the_branches_used_to_decide() {
        let (native, container) = (RuntimeKind::Host.adapter(), RuntimeKind::Container.adapter());
        assert!(native.carries_signing_keys() && !container.carries_signing_keys());
        assert!(native.accepts_attachments() && !container.accepts_attachments());
        assert!(native.accepts_persistent_controller() && !container.accepts_persistent_controller());
        assert!(native.can_host(ControllerType::Terminal) && native.can_host(ControllerType::Msp));
        assert!(!container.can_host(ControllerType::Terminal) && !container.can_host(ControllerType::Msp));
        assert!(container.can_host(ControllerType::Persistent));
        assert_eq!(native.controller_type_for_open("persistent"), "persistent");
        assert_eq!(container.controller_type_for_open("persistent"), "subprocess");
    }

    #[test]
    fn a_container_adapter_without_a_container_refuses_to_launch() {
        let ctrl = SubprocessController::new("tab".into(), "ra-block-c".into(), None, None, None, None, None, "boot".into());
        let err = RuntimeKind::Container.adapter().launch(&ctrl, SubprocessSpawnConfig::default()).unwrap_err();
        assert!(err.contains("without a container"), "{err}");
    }

    /// "Is this a container agent?" is answered by [`RuntimeKind`]. A new `== "container"` outside the
    /// files below is a branch the adapter should own; move it behind a method here instead.
    #[test]
    fn nothing_but_the_adapter_compares_a_mode_to_container() {
        // Files that still read the string, and why.
        let allowed = [
            "backend/runtime_adapter.rs",                           // the one derivation
            "migrations/m0043_rotate_container_agent_keys.rs",      // a frozen migration reads the stored value
            "backend/operator_config_seed.rs",                      // Global Memory audience (#13666 step 5)
        ];
        fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, out);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    out.push(path);
                }
            }
        }
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        walk(&src, &mut files);
        let mut offenders = Vec::new();
        for file in files {
            let rel = file.strip_prefix(&src).unwrap().to_string_lossy().replace('\\', "/");
            if allowed.contains(&rel.as_str()) || rel.ends_with("tests.rs") || rel.contains("/tests/") {
                continue;
            }
            let text = std::fs::read_to_string(&file).unwrap();
            for (n, line) in text.lines().enumerate() {
                let code = line.split("//").next().unwrap_or("");
                if code.contains("== \"container\"") || code.contains("!= \"container\"") {
                    offenders.push(format!("{rel}:{}", n + 1));
                }
            }
        }
        assert!(offenders.is_empty(), "compare through RuntimeKind, not the string: {offenders:?}");
    }
}
