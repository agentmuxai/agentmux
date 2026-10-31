// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Container management for container-type agent panes (Phase 1).
//!
//! Manages persistent Docker containers — one per container agent. Each
//! container runs the configured image (e.g. `ghcr.io/agentmuxai/agent-claude`)
//! with `tini` as PID 1 and is kept alive between turns. Individual turns are
//! executed via `docker exec -i` (no `-t` — avoids CR/LF corruption of NDJSON).
//!
//! Platform support (cross-platform, matches SPEC_CONTAINER_PANE_SUPPORT_2026_06_11.md):
//!   - `DOCKER_HOST` when set (unix, tcp with or without TLS, https, npipe), else the sockets
//!     Docker, Podman, Colima and OrbStack leave in their usual places; the first that answers a
//!     ping is used (`container_runtime.rs`).
//!   - Windows: named pipe `//./pipe/docker_engine`
//!
//! Independence boundary: this module has no dependency on any other project's container code.
//! Docker best practices here were researched independently.

pub(crate) mod drift;
pub mod limits;
pub mod oom;
mod keychain_credentials;
mod mount_source;
mod mounts;
mod pull;
mod reap;
pub use self::pull::{StartEvent, StartObserver};

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use tokio::io::AsyncWrite;
use tokio::sync::Mutex;
use futures_util::StreamExt as _;
use bollard::Docker;

use crate::backend::container_runtime;
use self::mount_source::fold_mount_source;
use self::mounts::{agent_home_mounts, host_name_for_new_container};
use bollard::container::{
    Config, CreateContainerOptions, ListContainersOptions, StartContainerOptions,
    StopContainerOptions,
};
use bollard::exec::{CreateExecOptions, StartExecOptions, StartExecResults};
use bollard::models::{ContainerInspectResponse, HostConfig, Mount, MountTypeEnum};
use crate::backend::container_cli::{
    check_says_present, install_failure_detail, reap_argv, reap_says_gone, CliInstall, InstallLimits,
};
use crate::backend::container_image::{CONTAINER_CLI_DIR,
    cli_install_failure_message, docker_error_message, docker_unreachable_message,
    looks_like_daemon_unreachable, pull_failure_message, PullFailure,
};

/// Where the agent CLI's config dir lives inside the image (`CLAUDE_CONFIG_DIR`).
pub const CONTAINER_CLAUDE_DIR: &str = "/home/agent/.claude";
/// Where the agent's working directory is bind-mounted inside the image.
pub const CONTAINER_WORKSPACE_DIR: &str = "/workspace";

/// Shared user-defined Docker bridge network every agent container is
/// attached to, ADDITIVE to whatever `network_mode`/`extra_hosts` already
/// set up (container→host reachability, unaffected — see the doc comment
/// on `network_mode` below). Unlike the default bridge network, a
/// user-defined one gives per-container DNS resolution and lets the
/// native dev-proxy (`backend::dev_proxy`) route to a container by its
/// internal IP. See docs/specs/SPEC_NATIVE_CONTAINER_DEV_PROXY_2026_09_19.md §3.1.
pub const AGENTMUX_DOCKER_NETWORK: &str = "agentmux-agents";

/// Host directories a container agent needs mounted to be able to work.
///
/// Both were missing entirely before 2026-09-02, which is why container agents
/// could never authenticate and always started with an empty `/workspace`. See
/// `docs/reports/REPORT_CONTAINER_AGENT_CREDENTIALS_AND_WORKSPACE_2026_09_02.md`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContainerMountSpec {
    /// The bound account's own resolved Claude config dir on the host —
    /// i.e. whatever `CLAUDE_CONFIG_DIR` the identity resolver produced for this
    /// agent. Its `.credentials.json` is bind-mounted into the container so the
    /// CLI there reads the same token the account login wrote (see
    /// [`agent_home_mounts`] for why it is that one file and not the whole dir).
    ///
    /// This is deliberately the ONE account's isolated dir, never the operator's
    /// global `~/.claude` — reaching into that would be the credential leak the
    /// named-volume design was avoiding.
    ///
    /// `None` keeps the old behavior (an empty per-agent named volume), which is
    /// correct only for an agent with no bound oauth account.
    pub claude_config_host_dir: Option<String>,
    /// The bound account's Claude config dir when its login is in the macOS Keychain and not in a file: the
    /// start of the container stages that login (see [`keychain_credentials`]) and fills
    /// [`Self::claude_config_host_dir`] with the staging folder. `None` everywhere else.
    pub keychain_config_dir: Option<String>,
    /// The agent's working directory on the host, bind-mounted at
    /// [`CONTAINER_WORKSPACE_DIR`]. `None`/empty leaves `/workspace` empty, which
    /// is what shipped before this existed.
    pub workspace_host_dir: Option<String>,
    /// The agent's Personal Memory folder on the host, bind-mounted where Claude reads memory in the container.
    pub memory_host_dir: Option<String>,
    /// AgentMux's git hooks (push scan, commit trailers) on the host, mounted read-only at
    /// [`crate::identity::github_tokens::CONTAINER_HOOKS_DIR`] where container agents get git through AgentMux.
    pub github_hooks_host_dir: Option<String>,
    /// This agent's AWS config folder (`identity::aws_broker::container`), mounted read-only where the host shares AWS.
    pub aws_config_host_dir: Option<String>,
    /// The agent's own CPU and memory limits (the app-wide setting and the default fill in what it leaves unset).
    pub limits: limits::ContainerLimits,
}

/// Normalize a host path for Docker's mount API: backslashes are not accepted in
/// a bind source, so `C:\Users\x` has to travel as `C:/Users/x`.
fn normalize_host_path(path: &str) -> String {
    path.replace('\\', "/")
}

/// The credentials file inside the account's config dir, and inside the image.
pub const CLAUDE_CREDENTIALS_FILE: &str = ".credentials.json";

/// The account config dir to hand to [`ContainerMountSpec::claude_config_host_dir`],
/// or `None` when this account has no file-backed credential to mount.
///
/// The existence check is load-bearing, not defensive tidiness (codex P1 on PR
/// #2933). **On macOS Claude Code keeps OAuth credentials in the encrypted
/// Keychain and never writes this file at all** — see
/// `identity/resolver/oauth_probe.rs` and
/// `docs/retro/retro-macos-keychain-credential-isolation-gap-2026-08-17.md`.
/// Binding a source that does not exist makes Docker reject `create_container`
/// outright, so an unguarded bind would turn "container agents cannot
/// authenticate on macOS" into "container agents cannot START on macOS" — a
/// strictly worse failure than the one this fix exists to remove.
///
/// Returning `None` degrades to an empty per-agent volume and logs why. On macOS the
/// container's start then stages the account's Keychain login instead (see
/// [`ContainerMountSpec::keychain_config_dir`] and `keychain_credentials`).
pub fn credentials_dir_if_file_backed(claude_config_host_dir: &str) -> Option<String> {
    if claude_config_host_dir.is_empty() {
        return None;
    }
    let creds = std::path::Path::new(claude_config_host_dir).join(CLAUDE_CREDENTIALS_FILE);
    if creds.is_file() {
        return Some(claude_config_host_dir.to_string());
    }
    tracing::warn!(
        dir = %claude_config_host_dir,
        "container agent: no {CLAUDE_CREDENTIALS_FILE} in the bound account's config dir — \
         not mounting credentials (expected on macOS, where Claude Code stores them in the \
         Keychain). The agent will start but will not be authenticated."
    );
    None
}

/// Rewrite a host-loopback `AGENTMUX_LOCAL_URL` (e.g. `http://127.0.0.1:PORT`)
/// into something a container's network namespace can actually reach.
///
/// `docker exec` over the Docker socket never inherits the host process's
/// env the way a host subprocess does, so a container agent gets no sidecar
/// URL at all unless one is explicitly injected — and injecting the
/// loopback value verbatim would be reachable from the srv process but not
/// from inside the container. `host.docker.internal` is resolvable from
/// Docker Desktop (macOS/Windows) automatically, and from native Linux via
/// the `extra_hosts: host.docker.internal:host-gateway` entry
/// `create_and_start` sets — see #2939 workstream 1.
///
/// Only rewrites `127.0.0.1`/`localhost`; any other host (already a real
/// hostname/IP, or already `host.docker.internal`) passes through
/// unchanged, so this is safe to call unconditionally.
pub fn rewrite_local_url_for_container(local_url: &str) -> String {
    local_url
        .replace("127.0.0.1", "host.docker.internal")
        .replace("localhost", "host.docker.internal")
}

/// Env var names that reference host-filesystem paths and must NOT be forwarded
/// into a container via `docker exec -e`. The container image supplies its own
/// values for these (e.g. `CLAUDE_CONFIG_DIR=/home/agent/.claude` baked in).
///
/// `CLAUDE_CONFIG_DIR` being here is why a bound account's credentials cannot
/// reach the container as an env var, and therefore why they are MOUNTED
/// instead — see [`ContainerMountSpec::claude_config_host_dir`].
pub const CONTAINER_ENV_DENYLIST: &[&str] = &[
    "CLAUDE_CONFIG_DIR",
    // Deliberately dropped even though `backend::gh_guard` sets it on the spawns
    // it guards: the guard dir is a host path that means nothing inside the
    // image, and the image controls its own `gh` state.
    "GH_CONFIG_DIR",
    "PATH",
    "HOME",
    "USERPROFILE",
    "HOMEDRIVE",
    "HOMEPATH",
    "TMPDIR",
    "TEMP",
    "TMP",
    // The host path of the agent's workspace; srv sets it for host agents
    // only, this keeps a stale `cmd:env` copy out of a container too.
    "AGENTMUX_AGENT_WORKDIR",
    // A host path; the image's own wrapper may predate prefix mode (SPEC_BASH_STREAMING_VIA_SHELL_PREFIX §2.4).
    "CLAUDE_CODE_SHELL_PREFIX",
];

/// Shared container manager. Clone-on-Arc; cheap to pass around.
#[derive(Clone)]
pub struct ContainerManager {
    inner: Arc<ContainerManagerInner>,
}

struct ContainerManagerInner {
    docker: Docker,
    /// The runtime in use, for the pane: `Podman at unix:///…`. `None` for a connection made
    /// without detection ([`ContainerManager::connect`]).
    runtime: Option<String>,
    /// Per-container serialization lock: prevents concurrent ensure_running calls
    /// for the same container from both seeing "not found" in Docker and both
    /// attempting create_container (which would fail with a 409 name-conflict
    /// on the second caller).
    ensure_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    /// Per-container `(uid, gid)` of the user execs run as, resolved once and
    /// cached — see [`ContainerManager::exec_identity`].
    exec_identities: Mutex<HashMap<String, (u64, u64)>>,
    /// `(container, install marker)` pairs already confirmed installed, so a
    /// running agent pays the check once, not on every turn. Dropped when the
    /// container is created or removed.
    cli_ready: Mutex<std::collections::HashSet<String>>,
    /// Serializes `provision_cli` per container: two quick messages must not
    /// run two `npm install`s into the same directory.
    cli_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    /// Dev-proxy routing table, wired in post-construction by
    /// [`ContainerManager::attach_dev_proxy_registry`] (called by
    /// `ContainerRuntimeHandle`, which re-attaches after every reconnect
    /// since each reconnect creates a fresh `ContainerManagerInner`).
    /// `stop`/`remove` clear this agent's registrations when set — see
    /// `cleanup_dev_proxy_routes`. Unset (`None` via `OnceLock`) in tests
    /// and any path that never wires it up; cleanup is then simply
    /// skipped, never an error. See
    /// docs/specs/SPEC_NATIVE_CONTAINER_DEV_PROXY_2026_09_19.md.
    dev_proxy: std::sync::OnceLock<crate::backend::dev_proxy::DevProxyRegistry>,
}

/// Exec session handle for a single turn.
///
/// `input` is the stdin pipe into the container process. It is ONLY usable for a
/// process that completes on a newline: the write half can never be closed, so
/// stdin never reaches EOF (see [`ContainerManager::exec`]). Turn input goes in
/// a file instead — NOT argv, which would leak a pasted secret via `docker top`
/// / host `ps` / `/proc/<pid>/cmdline`; see
/// [`ContainerManager::upload_turn_prompt`]. `output` is a bollard `LogOutput`
/// stream; with
/// `tty: false` Docker multiplexes stdout as `LogOutput::StdOut` frames and
/// stderr as `LogOutput::StdErr` frames.
///
/// Env vars are passed via `CreateExecOptions.env` (Docker socket) — they never
/// appear in process argv, preventing CWE-214 credential exposure via ps/proc.
pub struct ExecSession {
    /// Docker exec id — pass to [`ContainerManager::inspect_exec`] after the
    /// output stream ends to retrieve the process's real exit code. The stream
    /// ending is NOT the exit status, so callers that care about success/failure
    /// must inspect the exec (the in-container CLI can exit non-zero while still
    /// closing stdout cleanly).
    pub exec_id: String,
    /// Stdin pipe to the container process. Unusable for EOF-terminated readers
    /// — see the type-level doc and [`ContainerManager::exec`]'s `attach_stdin`.
    pub input: Pin<Box<dyn AsyncWrite + Send>>,
    /// Multiplexed stdout/stderr stream (LogOutput::StdOut / LogOutput::StdErr).
    pub output: Pin<Box<dyn futures_util::Stream<Item = Result<bollard::container::LogOutput, bollard::errors::Error>> + Send>>,
}

/// Parse `id -u; id -g` output into `(uid, gid)`.
///
/// Split out so the parse is testable without a Docker daemon, and so a
/// malformed or empty probe result is `None` — never a silent `(0, 0)`, which
/// would produce a prompt file the exec user can neither read nor delete.
fn parse_id_output(out: &str) -> Option<(u64, u64)> {
    let mut fields = out.split_whitespace();
    let uid: u64 = fields.next()?.parse().ok()?;
    let gid: u64 = fields.next()?.parse().ok()?;
    Some((uid, gid))
}

#[cfg(test)]
mod exec_identity_tests {
    use super::parse_id_output;

    #[test]
    fn parses_the_ordinary_two_line_form() {
        assert_eq!(parse_id_output("1000\n1000\n"), Some((1000, 1000)));
    }

    #[test]
    fn parses_root_and_a_split_uid_gid() {
        assert_eq!(parse_id_output("0\n0\n"), Some((0, 0)));
        assert_eq!(parse_id_output("1000\n2000\n"), Some((1000, 2000)));
    }

    /// Docker multiplexes frames, so the two numbers may arrive however they
    /// arrive — whitespace splitting must not care.
    #[test]
    fn tolerates_arbitrary_whitespace_and_framing() {
        assert_eq!(parse_id_output("  1000 \r\n  1000  \r\n"), Some((1000, 1000)));
        assert_eq!(parse_id_output("1000 1000"), Some((1000, 1000)));
    }

    /// The whole point of returning Option: a failed or truncated probe must
    /// NOT silently become root (reagent P1, PR #2883).
    #[test]
    fn a_missing_or_malformed_probe_is_none_not_root() {
        assert_eq!(parse_id_output(""), None);
        assert_eq!(parse_id_output("1000"), None, "gid missing");
        assert_eq!(parse_id_output("id: command not found"), None);
        assert_eq!(parse_id_output("uid=1000(agent)"), None, "not the -u form");
        assert_eq!(parse_id_output("-1\n-1\n"), None, "negative is not a uid");
    }
}

/// Errors from container operations.
#[derive(Debug, thiserror::Error)]
pub enum ContainerError {
    #[error("Docker API error: {0}")]
    Docker(#[from] bollard::errors::Error),
    #[error("Container has no id after create: {name}")]
    NoId { name: String },
    #[error("Docker is not available on this host: {0}")]
    NotAvailable(String),
    /// No container runtime answered; the text says where it looked and why each place failed
    /// ([`container_runtime::Missing::message`]).
    #[error("{0}")]
    NoRuntime(String),
    /// The image could not be downloaded. `detail` is the daemon's own text,
    /// kept for logs; the pane gets [`ContainerError::user_message`].
    #[error("could not pull image {image}: {detail}")]
    ImagePull {
        image: String,
        kind: PullFailure,
        detail: String,
    },
    /// The provider CLI could not be installed in the container.
    #[error("could not install {command} in the container: {detail}")]
    CliInstall { command: String, detail: String },
    /// A `container_volumes` entry whose source is the host's CLI session folder.
    #[error("the volume source {path} is a host folder the container may not mount")]
    VolumeRefused { path: String },
    /// The macOS Keychain login couldn't be given to the container (plain reason); raised before the image is pulled.
    #[error("no login for the container: {0}")]
    NoLogin(String),
}

impl ContainerError {
    /// What to tell the person using the agent: what happened and what to do
    /// next, never the raw Docker error.
    pub fn user_message(&self) -> String {
        match self {
            Self::ImagePull { image, kind, detail } => pull_failure_message(image, *kind, detail),
            Self::Docker(e) => {
                let text = e.to_string();
                if looks_like_daemon_unreachable(&text) {
                    docker_unreachable_message()
                } else {
                    docker_error_message(&text)
                }
            }
            Self::NotAvailable(_) => docker_unreachable_message(),
            Self::NoRuntime(message) => message.clone(),
            Self::NoId { .. } => docker_error_message("the container was created without an id"),
            Self::CliInstall { command, detail } => cli_install_failure_message(command, detail),
            Self::NoLogin(reason) => keychain_credentials::no_login_message(reason),
            Self::VolumeRefused { path } => format!(
                "This agent's volumes list a host folder a container may not mount ({path}): remove it from the agent's volumes."
            ),
        }
    }
}

impl ContainerManager {
    /// Connect to the local Docker daemon using environment/platform defaults.
    ///
    /// Honors `DOCKER_HOST` env var (e.g. for rootless or remote daemons).
    /// On Windows connects via named pipe; on macOS/Linux via Unix socket.
    pub fn connect() -> Result<Self, ContainerError> {
        let docker = Docker::connect_with_local_defaults()
            .map_err(|e| ContainerError::NotAvailable(e.to_string()))?;
        Ok(Self::with_docker(docker, None))
    }

    /// Connect to the runtime [`container_runtime`] finds: `DOCKER_HOST`, else the first socket
    /// of rootless Docker, Podman, Docker Desktop, Colima, OrbStack or the system that answers a
    /// ping. `Err` says where it looked and why each place failed.
    pub async fn connect_detected() -> Result<Self, ContainerError> {
        let env = container_runtime::Env::from_process();
        match container_runtime::connect(&env).await {
            Ok(found) => {
                let label = found.label();
                tracing::info!(runtime = %label, "container runtime found");
                Ok(Self::with_docker(found.docker, Some(label)))
            }
            Err(missing) => Err(ContainerError::NoRuntime(missing.message())),
        }
    }

    pub(crate) fn with_docker(docker: Docker, runtime: Option<String>) -> Self {
        Self {
            inner: Arc::new(ContainerManagerInner {
                docker,
                runtime,
                ensure_locks: Mutex::new(HashMap::new()),
                exec_identities: Mutex::new(HashMap::new()),
                cli_ready: Mutex::new(std::collections::HashSet::new()),
                cli_locks: Mutex::new(HashMap::new()),
                dev_proxy: std::sync::OnceLock::new(),
            }),
        }
    }

    /// Which runtime this connection is to, e.g. `Podman at unix:///run/user/1000/podman/podman.sock`.
    pub fn runtime_label(&self) -> Option<&str> {
        self.inner.runtime.as_deref()
    }

    /// Wires a dev-proxy routing table into this manager so `stop`/`remove`
    /// can clear an agent's registrations when its container goes away.
    /// Set once per `ContainerManager` instance (a second call is a
    /// harmless no-op — `OnceLock::set` fails silently, which is fine
    /// here: the registry itself doesn't change after boot).
    /// See docs/specs/SPEC_NATIVE_CONTAINER_DEV_PROXY_2026_09_19.md.
    pub fn attach_dev_proxy_registry(&self, registry: crate::backend::dev_proxy::DevProxyRegistry) {
        let _ = self.inner.dev_proxy.set(registry);
    }

    /// This container's IP address on [`AGENTMUX_DOCKER_NETWORK`] — what
    /// the dev-proxy registry stores as the routable backend address.
    /// `None` if the container isn't running, isn't on the network yet, or
    /// Docker is unreachable. Always a fresh `inspect_container`, never
    /// cached — the caller (`app_api::dev_server::handle_register_dev_server`)
    /// needs the CURRENT address, not one from whenever the container was
    /// first created.
    pub async fn agent_network_ip(&self, container_name: &str) -> Option<String> {
        let details = self.inner.docker.inspect_container(container_name, None).await.ok()?;
        details
            .network_settings?
            .networks?
            .get(AGENTMUX_DOCKER_NETWORK)?
            .ip_address
            .clone()
            .filter(|s| !s.is_empty())
    }

    /// Ping the Docker daemon. Returns `Ok(())` if available.
    pub async fn check_available(&self) -> Result<(), ContainerError> {
        self.inner.docker.ping().await?;
        Ok(())
    }

    /// Ensure the container for this agent is created and running.
    ///
    /// - If it exists and is running: no-op.
    /// - If it exists but stopped: starts it.
    /// - If it doesn't exist: creates and starts it.
    ///
    /// Always queries Docker (no in-memory cache) so externally killed containers
    /// are detected. Concurrent calls for the **same** `container_name` are
    /// serialized via a per-container mutex. Concurrent calls for **different**
    /// containers proceed in parallel.
    ///
    /// `observe` is told about the steps that take long enough for a person to
    /// wonder: downloading the image (with how far it has got) and creating the
    /// container. Nothing is reported when the container is already up.
    pub async fn ensure_running_observed(
        &self,
        container_name: &str,
        image: &str,
        volumes: &[String],
        env_vars: &[(String, String)],
        spec: &ContainerMountSpec,
        observe: StartObserver<'_>,
    ) -> Result<(), ContainerError> {
        // Acquire the per-container serialization lock before touching Docker.
        // Concurrent callers for the same container_name queue here; once the
        // first caller completes, the second finds the container already running
        // via the Docker query below and returns immediately.
        let container_lock = {
            let mut locks = self.inner.ensure_locks.lock().await;
            locks.entry(container_name.to_string())
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };
        let guard = container_lock.lock().await;

        let result = self.ensure_running_locked(container_name, image, volumes, env_vars, spec, observe).await;

        // Release the per-container lock, then evict its map entry when no other
        // caller is queued on it — otherwise `ensure_locks` grows unbounded (one
        // entry per distinct container ever seen) over the process lifetime.
        // A queued caller has already cloned the Arc (strong_count > 2), so we
        // only drop the entry when it's truly idle (map's ref + our clone == 2),
        // which can't reintroduce the create-race the lock guards: a new caller
        // can't clone the old Arc while we hold the map lock, and after eviction
        // it just creates a fresh one with no contention.
        drop(guard);
        {
            let mut locks = self.inner.ensure_locks.lock().await;
            if Arc::strong_count(&container_lock) <= 2 {
                locks.remove(container_name);
            }
        }

        result
    }

    /// Create/reuse logic for [`ensure_running`], run while holding the
    /// per-container serialization lock. Split out so `ensure_running` can wrap
    /// it with lock acquisition + map-entry eviction on every exit path.
    async fn ensure_running_locked(
        &self,
        container_name: &str,
        image: &str,
        volumes: &[String],
        env_vars: &[(String, String)],
        spec: &ContainerMountSpec,
        observe: StartObserver<'_>,
    ) -> Result<(), ContainerError> {
        // Before the container is reused or made: an existing one is refused as well.
        if let Some(path) = crate::backend::cli_messaging::refused_volume_source(volumes.iter().filter_map(|v| parse_volume_spec(v)).map(|(s, _, _)| s)) {
            return Err(ContainerError::VolumeRefused { path });
        }
        // Always query Docker — do not rely on an in-memory cache. The container
        // can be stopped or removed externally (e.g. `docker rm -f`), and a
        // stale cache entry would cause ensure_running to silently no-op while
        // a subsequent exec call fails.
        let existing = self.find_container(container_name).await?;

        // A container created before this agent had credentials (or a workspace)
        // keeps those mounts for its whole life — `docker` cannot add a mount to
        // an existing container. Without this check the fix would silently never
        // reach any agent whose container is already up, which is every agent
        // that has ever run. Recreate on drift; the per-agent named volume
        // carries session/project state across the swap.
        let existing = match existing {
            Some(status) => match self.container_drift(container_name, spec).await {
                Some(reason) => {
                    tracing::info!(container = container_name, reason, "container spec drift detected — recreating");
                    let _ = self.stop(container_name, 10).await;
                    self.remove(container_name, true).await?;
                    self.restage_keychain_login(container_name, spec).await;
                    None
                }
                None => Some(status),
            },
            None => None,
        };

        match existing {
            Some(status) => {
                if status != "running" {
                    // Exists but stopped — start it.
                    self.inner.docker
                        .start_container(container_name, None::<StartContainerOptions<String>>)
                        .await?;
                    tracing::info!(container = container_name, "restarted stopped container");
                }
                self.secure_credentials_once(container_name).await;
                self.limits_apply(container_name, spec.limits).await;
            }
            None => {
                // Pull the image via Docker socket before create so the Engine API
                // has the image locally. Unlike `docker run`, the Engine's
                // create_container does NOT auto-pull — it returns 404 if the image
                // is absent. pull_image is a no-op when the image already exists.
                let image = self.pull_with_fallback(image, observe).await?;
                // Create and start.
                observe(StartEvent::Starting);
                self.create_and_start(container_name, &image, volumes, env_vars, spec).await?;
                self.prepare_home(container_name, spec).await;
                tracing::info!(container = container_name, image = image, "created and started container");
            }
        }

        // Additive, best-effort: attach to the shared dev-proxy network
        // regardless of which branch above ran, so an already-running
        // container from before this feature shipped still gets picked up
        // on its next turn. See `ensure_agent_network`'s own doc comment.
        self.ensure_agent_network(container_name).await;

        Ok(())
    }

    /// Launch an exec session inside a running container.
    ///
    /// Uses `-i` (not `-t`) to avoid tty CR/LF corruption of NDJSON output.
    /// The caller receives `ExecSession` whose `output` stream carries
    /// multiplexed stdout/stderr for piping into the block.
    ///
    /// `attach_stdin` MUST be `false` unless the in-container process is a
    /// reader that completes on a newline rather than on EOF. Attaching stdin
    /// is a one-way door: bollard's hijacked exec stream cannot half-close the
    /// write side (see the `exec (io)` integration test below), and on Windows
    /// the Docker transport is a named pipe, which has no half-close at all —
    /// so the process's stdin NEVER reaches EOF. Neither `drop(input)` nor an
    /// explicit `input.shutdown().await` changes that; both were tried against
    /// a live container on 2026-09-01 and the process hung indefinitely.
    ///
    /// Any command that reads stdin to EOF (`cat`, or `claude -p` taking its
    /// prompt on stdin) will therefore hang forever, producing no output and
    /// no exit. Give it a file to read instead — see
    /// [`ContainerManager::upload_turn_prompt`], which also covers why argv and
    /// env are both the wrong place for that input.
    pub async fn exec(
        &self,
        container_name: &str,
        cmd: &[String],
        working_dir: Option<&str>,
        env_vars: &[(String, String)],
        attach_stdin: bool,
    ) -> Result<ExecSession, ContainerError> {
        let env: Vec<String> = env_vars.iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();

        let exec = self.inner.docker
            .create_exec(container_name, CreateExecOptions {
                attach_stdin: Some(attach_stdin),
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                tty: Some(false), // NO tty — preserves NDJSON newlines
                cmd: Some(cmd.iter().map(String::as_str).collect()),
                working_dir,
                env: if env.is_empty() { None } else { Some(env.iter().map(String::as_str).collect()) },
                ..Default::default()
            })
            .await?;

        let results = self.inner.docker
            .start_exec(&exec.id, Some(StartExecOptions {
                detach: false,
                ..Default::default()
            }))
            .await?;

        match results {
            StartExecResults::Attached { input, output } => {
                Ok(ExecSession { exec_id: exec.id, input, output })
            }
            StartExecResults::Detached => {
                Err(ContainerError::Docker(
                    bollard::errors::Error::DockerResponseServerError {
                        status_code: 0,
                        message: "start_exec returned Detached unexpectedly (detach=false)".to_string(),
                    }
                ))
            }
        }
    }

    /// The `(uid, gid)` that `exec` runs as inside this container.
    ///
    /// Needed so an uploaded file can be OWNED by that user. A tar entry
    /// defaults to uid 0, and `/tmp` is sticky (`1777`) in every standard
    /// image — under which a non-root process cannot unlink a root-owned file
    /// no matter what its mode is. That is not theoretical: it silently leaked
    /// one prompt file per turn (`rm -f` fails quietly), including multi-
    /// hundred-KB ones, until a live check caught the files piling up.
    ///
    /// Resolved by asking the container rather than assuming a uid, so this
    /// holds for any image regardless of which user it runs as.
    ///
    /// **Only a successful probe is cached.** An earlier cut cached a `(0, 0)`
    /// fallback on any failure, including a transient Docker hiccup — which
    /// poisoned the cache for the container's whole life (reagent P1, PR
    /// #2883). That is not a degraded mode but a permanent break: at mode
    /// 0600 a root-owned prompt file is unreadable AND un-unlinkable by the
    /// real exec user, so every later turn on that container would fail to
    /// open its prompt and orphan another file. A failure now returns `None`
    /// and is retried on the next turn.
    async fn exec_identity(&self, container_name: &str) -> Option<(u64, u64)> {
        if let Some(hit) = self.inner.exec_identities.lock().await.get(container_name) {
            return Some(*hit);
        }

        let probe = async {
            let session = self
                .exec(
                    container_name,
                    &["sh".to_string(), "-c".to_string(), "id -u; id -g".to_string()],
                    None,
                    &[],
                    false,
                )
                .await
                .ok()?;
            let mut out = String::new();
            let mut stream = session.output;
            while let Some(Ok(frame)) = stream.next().await {
                out.push_str(&frame.to_string());
            }
            parse_id_output(&out)
        }
        .await;

        match probe {
            Some(resolved) => {
                self.inner
                    .exec_identities
                    .lock()
                    .await
                    .insert(container_name.to_string(), resolved);
                Some(resolved)
            }
            None => {
                // NOT cached — the next turn probes again.
                tracing::warn!(
                    container = container_name,
                    "could not resolve container exec uid/gid; will retry next turn",
                );
                None
            }
        }
    }

    /// Remove a prompt file uploaded by [`upload_turn_prompt`].
    ///
    /// The turn's own wrapper script removes the file after the CLI exits, so
    /// this is only for the path where the wrapper never runs at all — the
    /// upload succeeded but starting the exec failed, which would otherwise
    /// orphan the file (reagent P2, PR #2883).
    ///
    /// Best-effort by design: it runs as the same user that owns the file, and
    /// a failure here is not worth failing an already-failing turn over.
    pub async fn remove_turn_prompt(&self, container_name: &str, prompt_path: &str) {
        let removed = self
            .exec(
                container_name,
                &[
                    "sh".to_string(),
                    "-c".to_string(),
                    r#"rm -f "$1""#.to_string(),
                    "agentmux-cleanup".to_string(),
                    prompt_path.to_string(),
                ],
                None,
                &[],
                false,
            )
            .await;
        match removed {
            Ok(session) => {
                // Drive the stream to completion so the exec actually runs —
                // Docker starts it lazily on the attached connection.
                let mut stream = session.output;
                while let Some(Ok(_)) = stream.next().await {}
            }
            Err(e) => tracing::warn!(
                container = container_name,
                path = prompt_path,
                "could not remove orphaned prompt file: {e}",
            ),
        }
    }

    /// Write a turn's prompt into the container as a file, returning its
    /// absolute in-container path.
    ///
    /// This exists because a container turn has no other way to hand the CLI
    /// its prompt:
    ///
    ///   * **exec stdin** can never reach EOF — bollard can't half-close the
    ///     hijacked stream's write side, and on Windows the Docker transport is
    ///     a named pipe, which has no half-close at all. A CLI that reads to
    ///     EOF (`claude -p`) hangs forever with no output and no exit.
    ///   * **argv** would expose any secret pasted into a chat message via
    ///     `docker top` / host `ps` / `/proc/<pid>/cmdline`, breaking this
    ///     module's own no-secrets-in-argv invariant (reagent P1, PR #2883).
    ///   * **an env var** keeps it off those host surfaces, but shares argv's
    ///     per-string `MAX_ARG_STRLEN` ceiling — measured in this very image at
    ///     ~128 KiB (130,000 B passes, 200,000 B fails with "Argument list too
    ///     long"). A long paste or a large `# Session Context` would break the
    ///     turn (codex P2, PR #2883).
    ///
    /// A file has none of those limits: unbounded size, invisible to every
    /// host-side process listing, and redirecting from it gives the CLI a real
    /// stdin that reaches a real EOF.
    ///
    /// Ownership is the exec user's (see [`exec_identity`]) so the turn can
    /// delete the file afterwards. Mode is 0600: only that user ever needs it,
    /// and it is the same user the CLI already runs as.
    pub async fn upload_turn_prompt(
        &self,
        container_name: &str,
        turn_id: &str,
        message: &str,
    ) -> Result<String, ContainerError> {
        // No usable uid means no usable file: at 0600 a root-owned prompt is
        // unreadable by the exec user, and /tmp's sticky bit makes it
        // un-unlinkable too. Failing here is a clear, retryable error; writing
        // it anyway would be a permission-denied on open plus a leaked file.
        let (uid, gid) = self.exec_identity(container_name).await.ok_or_else(|| {
            ContainerError::NotAvailable(format!(
                "could not resolve the exec user of container {container_name}; \
                 cannot write a readable turn prompt"
            ))
        })?;
        let file_name = format!("agentmux-turn-{turn_id}");
        let bytes = message.as_bytes();

        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o600);
        header.set_uid(uid);
        header.set_gid(gid);
        header.set_mtime(0);
        header.set_cksum();

        let mut archive = tar::Builder::new(Vec::new());
        archive
            .append_data(&mut header, &file_name, bytes)
            .map_err(|e| ContainerError::NotAvailable(format!("tar build failed: {e}")))?;
        let tar_bytes = archive
            .into_inner()
            .map_err(|e| ContainerError::NotAvailable(format!("tar finish failed: {e}")))?;

        self.inner
            .docker
            .upload_to_container(
                container_name,
                Some(bollard::container::UploadToContainerOptions {
                    path: "/tmp",
                    no_overwrite_dir_non_dir: "true",
                }),
                tar_bytes.into(),
            )
            .await?;

        Ok(format!("/tmp/{file_name}"))
    }

    /// Retrieve the exit code of a finished exec via the Docker socket.
    ///
    /// Returns `Ok(Some(code))` once the exec has exited, `Ok(None)` while it is
    /// still running (no code yet) or if Docker did not report one. Call this
    /// after the exec's output stream has ended — unlike a child process's
    /// `wait()`, the attached output stream closing does not carry the exit
    /// status, so the turn's success/failure can only be known by inspecting.
    pub async fn inspect_exec(&self, exec_id: &str) -> Result<Option<i64>, ContainerError> {
        let info = self.inner.docker.inspect_exec(exec_id).await?;
        // `running == Some(true)` means no meaningful code yet.
        if info.running == Some(true) {
            return Ok(None);
        }
        Ok(info.exit_code)
    }

    /// Run `argv` in the container and collect its output and exit code.
    async fn exec_collect(
        &self,
        container_name: &str,
        argv: &[String],
        limit: std::time::Duration,
    ) -> Result<ExecOutcome, ContainerError> {
        const KEEP_BYTES: usize = 16 * 1024;
        let session = self.exec(container_name, argv, None, &[], false).await?;
        let exec_id = session.exec_id.clone();
        let mut stream = session.output;
        let mut output = String::new();
        let drain = async {
            while let Some(item) = stream.next().await {
                let chunk = item?;
                output.push_str(&String::from_utf8_lossy(&chunk.into_bytes()));
                if output.len() > KEEP_BYTES {
                    let mut cut = output.len() - KEEP_BYTES;
                    while !output.is_char_boundary(cut) {
                        cut += 1;
                    }
                    output.drain(..cut);
                }
            }
            Ok::<(), bollard::errors::Error>(())
        };
        let timed_out = match tokio::time::timeout(limit, drain).await {
            Ok(result) => {
                result?;
                false
            }
            Err(_) => true,
        };
        let exit = if timed_out { None } else { self.inspect_exec(&exec_id).await? };
        Ok(ExecOutcome { exec_id, exit, output, timed_out })
    }

    /// Poll until the exec has stopped running, or `limit` passes. True when it
    /// is confirmed finished.
    async fn wait_exec_finished(&self, exec_id: &str, limit: std::time::Duration) -> bool {
        let deadline = std::time::Instant::now() + limit;
        loop {
            match self.docker_exec_running(exec_id).await {
                Some(false) => return true,
                Some(true) => {}
                None => return false,
            }
            if std::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }

    /// Whether the exec is still running; `None` when Docker can't say.
    async fn docker_exec_running(&self, exec_id: &str) -> Option<bool> {
        self.inner.docker.inspect_exec(exec_id).await.ok().map(|i| i.running == Some(true))
    }

    /// Kill any `npm install` into the CLI directory and wait until none is left.
    /// True only when that is confirmed.
    async fn reap_cli_install(&self, container_name: &str) -> bool {
        match self
            .exec_collect(container_name, &reap_argv(), std::time::Duration::from_secs(30))
            .await
        {
            Ok(out) => out.exit == Some(0) && reap_says_gone(&out.output),
            Err(e) => {
                tracing::warn!(container = container_name, error = %e, "could not check for a running CLI install");
                false
            }
        }
    }

    /// Make sure the provider CLI is usable in this container, installing it on
    /// first use. `on_install_start` runs just before an install begins (the
    /// caller shows "Installing ..." then), never when nothing needs installing.
    pub async fn provision_cli(
        &self,
        container_name: &str,
        install: &CliInstall,
        on_install_start: impl FnOnce(),
    ) -> Result<CliProvision, ContainerError> {
        self.provision_cli_with(container_name, install, InstallLimits::default(), on_install_start).await
    }

    /// [`provision_cli`](Self::provision_cli) with explicit time limits.
    async fn provision_cli_with(
        &self,
        container_name: &str,
        install: &CliInstall,
        limits: InstallLimits,
        on_install_start: impl FnOnce(),
    ) -> Result<CliProvision, ContainerError> {
        let key = install.cache_key(container_name);
        if self.inner.cli_ready.lock().await.contains(&key) {
            return Ok(CliProvision::Present);
        }

        let lock = {
            let mut locks = self.inner.cli_locks.lock().await;
            locks.entry(container_name.to_string()).or_insert_with(|| Arc::new(Mutex::new(()))).clone()
        };
        let guard = lock.lock().await;
        let result = self.provision_cli_locked(container_name, install, &key, limits, on_install_start).await;
        drop(guard);
        {
            let mut locks = self.inner.cli_locks.lock().await;
            if Arc::strong_count(&lock) <= 2 {
                locks.remove(container_name);
            }
        }
        result
    }

    async fn provision_cli_locked(
        &self,
        container_name: &str,
        install: &CliInstall,
        key: &str,
        limits: InstallLimits,
        on_install_start: impl FnOnce(),
    ) -> Result<CliProvision, ContainerError> {
        if self.inner.cli_ready.lock().await.contains(key) {
            return Ok(CliProvision::Present);
        }
        let check = self
            .exec_collect(container_name, &install.check_argv(), std::time::Duration::from_secs(30))
            .await?;
        if check.exit == Some(0) && check_says_present(&check.output) {
            self.inner.cli_ready.lock().await.insert(key.to_string());
            return Ok(CliProvision::Present);
        }

        on_install_start();

        // An earlier attempt may still be running (it timed out, or srv restarted
        // under it). A second npm into the same volume could corrupt it, so
        // nothing starts until the first is confirmed gone.
        let fail = |detail: String| ContainerError::CliInstall { command: install.command.clone(), detail };
        if !self.reap_cli_install(container_name).await {
            return Err(fail("an earlier install is still running; wait a minute and try again".to_string()));
        }

        tracing::info!(container = container_name, command = %install.command, specs = ?install.specs, "installing the provider CLI in the container");
        let started = std::time::Instant::now();
        let reaper = reap::ReapOnDrop::new(self, container_name);
        let run = self
            .exec_collect(container_name, &install.install_argv(limits), limits.run)
            .await?;
        reaper.disarm();
        if run.exit == Some(0) {
            self.inner.cli_ready.lock().await.insert(key.to_string());
            return Ok(CliProvision::Installed { seconds: started.elapsed().as_secs_f64() });
        }

        // The host-side limit passed while the install was still running. Stop
        // it and confirm it is gone BEFORE returning: the caller's lock is held
        // until then, so a retry cannot start a second install on top of it.
        let stopped = if run.timed_out {
            let reaped = self.reap_cli_install(container_name).await;
            let finished = self.wait_exec_finished(&run.exec_id, std::time::Duration::from_secs(30)).await;
            Some(reaped && finished)
        } else {
            None
        };
        let detail = install_failure_detail(run.exit, run.timed_out, stopped, &run.output);
        tracing::warn!(container = container_name, command = %install.command, detail = %detail, "provider CLI install failed");
        Err(fail(detail))
    }

    async fn forget_cli_ready(&self, container_name: &str) {
        let prefix = format!("{container_name}\u{0}");
        self.inner.cli_ready.lock().await.retain(|k| !k.starts_with(&prefix));
    }

    /// Best-effort interruption of the turn's process(es) inside a container.
    ///
    /// Docker/bollard has no "kill exec" API, so we `pkill` the matching process
    /// via a short detached exec. The persistent-container model runs one turn at
    /// a time (guarded by `run_lock`), so a single CLI process matches `pattern`
    /// (the command name, e.g. `claude`). `-f` matches the full command line
    /// because the CLI runs under `node`, whose process name isn't the CLI's.
    ///
    /// Requires `pkill` (procps) in the image. Fire-and-forget (detached); a
    /// non-match (`pkill` exit 1, e.g. the turn already exited) is not an error.
    pub async fn signal_exec_process(
        &self,
        container_name: &str,
        pattern: &str,
        force: bool,
    ) -> Result<(), ContainerError> {
        let signal = if force { "-KILL" } else { "-TERM" };
        let cmd: Vec<&str> = vec!["pkill", signal, "-f", pattern];
        let exec = self.inner.docker
            .create_exec(container_name, CreateExecOptions {
                cmd: Some(cmd),
                attach_stdout: Some(false),
                attach_stderr: Some(false),
                ..Default::default()
            })
            .await?;
        self.inner.docker
            .start_exec(&exec.id, Some(StartExecOptions { detach: true, ..Default::default() }))
            .await?;
        Ok(())
    }

    /// Gracefully stop a container. Uses SIGTERM → SIGKILL after `timeout_secs`.
    pub async fn stop(&self, container_name: &str, timeout_secs: i64) -> Result<(), ContainerError> {
        self.inner.docker
            .stop_container(container_name, Some(StopContainerOptions { t: timeout_secs }))
            .await?;
        tracing::info!(container = container_name, "stopped container");
        self.cleanup_dev_proxy_routes(container_name).await;
        Ok(())
    }

    /// Remove a container (must be stopped first or use `force = true`).
    pub async fn remove(&self, container_name: &str, force: bool) -> Result<(), ContainerError> {
        use bollard::container::RemoveContainerOptions;
        // A login the container refreshed goes back to the Keychain while the container is still there to ask.
        self.write_back_keychain_login(container_name).await;
        self.inner.docker
            .remove_container(container_name, Some(RemoveContainerOptions {
                force,
                ..Default::default()
            }))
            .await?;
        tracing::info!(container = container_name, "removed container");
        self.drop_staged_keychain_login(container_name).await;
        self.forget_cli_ready(container_name).await;
        self.cleanup_dev_proxy_routes(container_name).await;
        Ok(())
    }

    /// Clears this container's dev-proxy routes (see
    /// docs/specs/SPEC_NATIVE_CONTAINER_DEV_PROXY_2026_09_19.md) if a
    /// registry was ever wired in via `attach_dev_proxy_registry` — a
    /// no-op everywhere else (tests, non-agent Docker use). Never fails
    /// `stop`/`remove`: the container operation itself already succeeded
    /// by the time this runs, and a routing-table cleanup miss is
    /// recoverable (the entry just goes stale until re-registered) in a
    /// way an interrupted `stop`/`remove` wouldn't be.
    async fn cleanup_dev_proxy_routes(&self, container_name: &str) {
        if let Some(registry) = self.inner.dev_proxy.get() {
            registry.unregister_container(container_name).await;
        }
    }

    // ---- private helpers ----

    /// Every mount target this module owns. Anything NOT in here (the caller's
    /// `agent:container_volumes`) is the user's business and is never a reason
    /// to rebuild their container.
    fn owned_mount_targets() -> [String; 5] {
        [
            CONTAINER_CLAUDE_DIR.to_string(),
            format!("{CONTAINER_CLAUDE_DIR}/{CLAUDE_CREDENTIALS_FILE}"),
            CONTAINER_WORKSPACE_DIR.to_string(),
            crate::identity::github_tokens::CONTAINER_HOOKS_DIR.to_string(),
            crate::identity::aws_broker::container::CONTAINER_AWS_DIR.to_string(),
        ]
    }

    /// The `(target -> source)` this module wants mounted for `spec`. A target
    /// absent from the map is one that must NOT be mounted.
    fn desired_owned_mounts(
        container_name: &str,
        spec: &ContainerMountSpec,
    ) -> HashMap<String, String> {
        let mut desired: HashMap<String, String> = agent_home_mounts(container_name, spec)
            .into_iter()
            .filter_map(|m| Some((m.target?, m.source?)))
            .collect();
        if let Some(dir) = spec.workspace_host_dir.as_deref().filter(|d| !d.is_empty()) {
            desired.insert(CONTAINER_WORKSPACE_DIR.to_string(), normalize_host_path(dir));
        }
        desired
    }

    /// Single Docker inspect covering both drift checks below — mounts and
    /// extra_hosts previously each did their own `inspect_container` round
    /// trip, doubling the Docker-socket call on every `ensure_running` for
    /// the common case of an already-compliant container (reagent P2 on PR
    /// #3393). Returns a human-readable reason if either has drifted, or
    /// `None` if the container matches (or the inspect itself failed —
    /// fails safe, same as the two checks below always did: a spurious
    /// recreate kills a live agent and loses its container-local state,
    /// which is worse than one more restart on stale state we couldn't
    /// even confirm).
    async fn container_drift(&self, container_name: &str, spec: &ContainerMountSpec) -> Option<&'static str> {
        let Ok(details) = self.inner.docker.inspect_container(container_name, None).await else {
            return None;
        };
        if !Self::mounts_match(&details, container_name, spec) {
            return Some("mounts differ from the desired spec (credentials/workspace)");
        }
        if !Self::extra_hosts_match(&details) {
            return Some("extra_hosts missing host.docker.internal:host-gateway");
        }
        None
    }

    /// Does the existing container already carry exactly the mounts `spec` asks
    /// for? Used to decide whether an agent that is already up has to be
    /// recreated to pick up (or lose) credentials and a workspace.
    ///
    /// Compares the owned set in BOTH directions — a mount that should now be
    /// ABSENT is drift too, not just a missing or changed one. That case is the
    /// security-relevant one (reagent P1 on PR #2933): unbinding an agent's
    /// account is a normal transition that never reaches
    /// `SpawnGateError::MissingCredentials` (that gate fires when credentials
    /// are expected and missing, not when an agent legitimately has none). A
    /// presence-only check would leave the old account's `.credentials.json`
    /// bind-mounted and WRITABLE in the running container forever, so every
    /// later turn would keep authenticating — and refreshing tokens — as the
    /// account the operator just unbound.
    ///
    /// Only this module's own targets are considered; the caller's extra
    /// `container_volumes` are ignored, so a user adding an unrelated mount is
    /// never a reason to destroy and rebuild their container underneath them.
    ///
    /// Fails SAFE: missing mount data on the passed-in inspect result returns
    /// `true` ("matches"), leaving the container alone — the inspect-error case
    /// itself is now handled once, by `container_drift`'s caller.
    fn mounts_match(details: &ContainerInspectResponse, container_name: &str, spec: &ContainerMountSpec) -> bool {
        let Some(actual) = details.mounts.as_ref() else {
            return true;
        };

        let source_at = |target: &str| -> Option<String> {
            actual
                .iter()
                .find(|m| m.destination.as_deref() == Some(target))
                .and_then(|m| m.name.clone().or_else(|| m.source.clone()))
                .map(|s| fold_mount_source(&s))
        };

        let desired = Self::desired_owned_mounts(container_name, spec);
        // The AWS folder counts only where the host shares AWS: not sharing it must not rebuild every container.
        let aws = crate::identity::aws_broker::container::CONTAINER_AWS_DIR;
        Self::owned_mount_targets().iter().filter(|t| spec.aws_config_host_dir.is_some() || t.as_str() != aws).all(|target| {
            let want = desired.get(target).map(|s| fold_mount_source(s));
            source_at(target) == want
        })
    }

    /// Whether an existing container's `HostConfig.extra_hosts` already has
    /// the `host.docker.internal:host-gateway` entry `create_and_start` sets
    /// (reagent P1, PR #3393). Docker cannot add an extra_hosts entry to an
    /// already-existing container — only `container_drift`'s recreate path
    /// (via `ensure_running_locked`) can actually fix a container that
    /// predates this.
    fn extra_hosts_match(details: &ContainerInspectResponse) -> bool {
        let Some(extra_hosts) = details.host_config.as_ref().and_then(|hc| hc.extra_hosts.as_ref()) else {
            return false;
        };
        extra_hosts
            .iter()
            .any(|h| h == "host.docker.internal:host-gateway")
    }

    /// Running Docker Compose containers whose project directory is `dir` or
    /// inside it (Compose's `com.docker.compose.project.working_dir` label):
    /// the ones an agent working there may have started. Names only.
    pub async fn running_compose_containers_under(&self, dir: &std::path::Path) -> Result<Vec<String>, ContainerError> {
        const WORKING_DIR: &str = "com.docker.compose.project.working_dir";
        let mut filters = HashMap::new();
        filters.insert("label", vec![WORKING_DIR]);
        filters.insert("status", vec!["running"]);
        let list = self.inner.docker
            .list_containers(Some(ListContainersOptions { all: false, filters, ..Default::default() }))
            .await?;
        Ok(list
            .into_iter()
            .filter(|c| {
                c.labels
                    .as_ref()
                    .and_then(|l| l.get(WORKING_DIR))
                    .is_some_and(|wd| std::path::Path::new(wd).starts_with(dir))
            })
            .filter_map(|c| c.names.and_then(|n| n.first().map(|n| n.trim_start_matches('/').to_string())))
            .collect())
    }

    /// Returns the container status string ("running", "exited", …) or `None` if not found.
    async fn find_container(&self, name: &str) -> Result<Option<String>, ContainerError> {
        let mut filters = HashMap::new();
        filters.insert("name", vec![name]);
        let list = self.inner.docker
            .list_containers(Some(ListContainersOptions {
                all: true,
                filters,
                ..Default::default()
            }))
            .await?;

        // `name` filter is a prefix match — verify exact name.
        let canonical = format!("/{name}");
        for c in &list {
            if let Some(names) = &c.names {
                if names.iter().any(|n| n == &canonical) {
                    return Ok(c.state.clone());
                }
            }
        }
        Ok(None)
    }

    /// Whether the daemon already has `image`.
    pub async fn has_image_locally(&self, image: &str) -> bool {
        self.inner.docker.inspect_image(image).await.is_ok()
    }

    async fn create_and_start(
        &self,
        container_name: &str,
        image: &str,
        volumes: &[String],
        env_vars: &[(String, String)],
        spec: &ContainerMountSpec,
    ) -> Result<(), ContainerError> {
        // A container name can be reused: `ensure_running` detects an
        // externally removed container and recreates it under the same name.
        // The replacement may exec as a DIFFERENT user -- a changed custom
        // image, or the same mutable tag repulled -- and a stale cached
        // identity would then chown every prompt file to the wrong uid, making
        // it unreadable (mode 0600) for the life of the process (codex P2,
        // PR #2883). The cache is keyed by name, so evict here, at the one
        // place a name starts pointing at a new container.
        self.inner.exec_identities.lock().await.remove(container_name);
        self.forget_cli_ready(container_name).await;

        let env: Vec<String> = env_vars.iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        let hostname = host_name_for_new_container();

        let mounts: Vec<Mount> = volumes.iter().filter_map(|spec| {
            let (source, target, read_only) = parse_volume_spec(spec)?;
            if target.is_empty() {
                tracing::warn!(spec = %spec, "ignoring malformed volume spec (expected source:target)");
                return None;
            }
            let mount_type = if source.starts_with('/') || source.starts_with('~')
                || (source.len() >= 2 && source.as_bytes()[1] == b':')
            {
                MountTypeEnum::BIND
            } else {
                MountTypeEnum::VOLUME
            };
            Some(Mount {
                target: Some(target.to_string()),
                source: Some(source.to_string()),
                typ: Some(mount_type),
                read_only: Some(read_only),
                ..Default::default()
            })
        }).collect();

        let mut all_mounts = agent_home_mounts(container_name, spec);
        if let Some(workspace) = spec.workspace_host_dir.as_deref().filter(|d| !d.is_empty()) {
            all_mounts.push(Mount {
                target: Some(CONTAINER_WORKSPACE_DIR.to_string()),
                source: Some(normalize_host_path(workspace)),
                typ: Some(MountTypeEnum::BIND),
                read_only: Some(false),
                ..Default::default()
            });
        }
        all_mounts.push(cli_dir_mount(container_name));
        all_mounts.extend(mounts);
        all_mounts.extend(self.cache_mounts_for_new(container_name, image, &all_mounts).await);

        let wanted_limits = self.wanted_limits_for_new(image, spec.limits).await;
        let mut config: Config<String> = Config {
            image: Some(image.to_string()),
            hostname,
            env: if env.is_empty() { None } else { Some(env) },
            // Keep container alive between turns — idle PID 1 (`sleep infinity`
            // via tini) stays running until `docker stop`. Agent turns run via
            // `docker exec`, not as the PID-1 process.
            tty: Some(false),
            open_stdin: Some(true),
            host_config: Some(HostConfig {
                mounts: Some(all_mounts),
                // Security: no host network, no privileged mode.
                network_mode: Some("bridge".to_string()),
                // Lets the container reach agentmux-srv via `host.docker.internal` (`run_agent_turn`/`agent_io`
                // rewrite AGENTMUX_LOCAL_URL to it, #2939). Docker Desktop resolves the name itself; `host-gateway`
                // is what makes it resolve on native Linux's default bridge network. A no-op where it already does.
                extra_hosts: Some(vec!["host.docker.internal:host-gateway".to_string()]),
                ..Default::default()
            }),
            ..Default::default()
        };
        wanted_limits.apply_to(config.host_config.get_or_insert_with(Default::default));

        let container = self.inner.docker
            .create_container(
                Some(CreateContainerOptions {
                    name: container_name,
                    platform: None,
                }),
                config,
            )
            .await?;

        let id = container.id;
        if id.is_empty() {
            return Err(ContainerError::NoId { name: container_name.to_string() });
        }

        self.inner.docker
            .start_container(&id, None::<StartContainerOptions<String>>)
            .await?;

        Ok(())
    }

    /// Best-effort: create the shared [`AGENTMUX_DOCKER_NETWORK`] (if it
    /// doesn't already exist) and attach `container_name` to it, additive
    /// to whatever `network_mode`/`extra_hosts` `create_and_start` already
    /// set up (container→host reachability via `host.docker.internal`,
    /// entirely unaffected). Never fails `ensure_running` — this is purely
    /// in service of dev-proxy routing
    /// (docs/specs/SPEC_NATIVE_CONTAINER_DEV_PROXY_2026_09_19.md §3.1), not
    /// required for a container agent's core turn-execution path. Called
    /// on every `ensure_running`, not just at creation, so an
    /// already-running container from before this feature shipped is
    /// picked up too, and so a network deleted out-of-band (`docker
    /// network rm`) is recreated automatically.
    async fn ensure_agent_network(&self, container_name: &str) {
        use bollard::models::EndpointSettings;
        use bollard::network::{ConnectNetworkOptions, CreateNetworkOptions};

        let create_opts = CreateNetworkOptions {
            name: AGENTMUX_DOCKER_NETWORK,
            driver: "bridge",
            ..Default::default()
        };
        if let Err(e) = self.inner.docker.create_network(create_opts).await {
            let already_exists = matches!(
                &e,
                bollard::errors::Error::DockerResponseServerError { message, .. }
                    if message.to_lowercase().contains("already exists")
            );
            if !already_exists {
                tracing::warn!(
                    network = AGENTMUX_DOCKER_NETWORK,
                    error = %e,
                    "dev-proxy: failed to create shared Docker network — dev-server routing unavailable this run"
                );
                return;
            }
        }
        let connect_opts = ConnectNetworkOptions {
            container: container_name,
            endpoint_config: EndpointSettings {
                aliases: Some(vec![container_name.to_string()]),
                ..Default::default()
            },
        };
        if let Err(e) = self.inner.docker.connect_network(AGENTMUX_DOCKER_NETWORK, connect_opts).await {
            let already_attached = matches!(
                &e,
                bollard::errors::Error::DockerResponseServerError { message, .. }
                    if message.to_lowercase().contains("already exists")
            );
            if !already_attached {
                tracing::warn!(
                    container = container_name,
                    network = AGENTMUX_DOCKER_NETWORK,
                    error = %e,
                    "dev-proxy: failed to attach container to shared Docker network"
                );
            }
        }
        crate::backend::container_listener::ensure_bound(&self.inner.docker).await;
    }
}

pub use crate::backend::container_runtime::ContainerRuntimeHandle;

/// What a collected exec produced.
struct ExecOutcome {
    exec_id: String,
    exit: Option<i64>,
    output: String,
    timed_out: bool,
}

/// Whether `provision_cli` found the CLI in place or had to install it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CliProvision {
    Present,
    Installed { seconds: f64 },
}

/// The per-agent volume the provider CLI is installed into.
fn cli_dir_mount(container_name: &str) -> Mount {
    Mount {
        target: Some(CONTAINER_CLI_DIR.to_string()),
        source: Some(format!("agentmux-cli-{container_name}")),
        typ: Some(MountTypeEnum::VOLUME),
        read_only: Some(false),
        ..Default::default()
    }
}

/// Derive the stable container name for an agent from its slug.
/// Format: `agentmux-<slug>`. Deterministic so restarts reuse the same container.
pub fn container_name_for_slug(slug: &str) -> String {
    format!("agentmux-{slug}")
}

/// Parse a Docker volume spec into `(source, target, read_only)`.
///
/// Format: `source:target` or `source:target:options`.
///
/// Handles Windows drive-letter bind paths (e.g. `C:\Users\me\repo:/workspace:ro`)
/// by treating the drive-letter colon (`X:`) as part of the source path, not as
/// the source/target separator. A plain `splitn(3, ':')` would split `C:\path`
/// into `C` and `\path`, losing the drive letter.
///
/// Returns `None` for malformed specs (no target separator found).
fn parse_volume_spec(spec: &str) -> Option<(&str, &str, bool)> {
    let bytes = spec.as_bytes();

    // Detect Windows drive-letter prefix: single ASCII letter followed by ':\'  or ':/'
    let (source, rest) = if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
    {
        // Windows path: skip the drive colon and split on the next ':'.
        let tail = &spec[2..]; // starts at '\' or '/'
        let pos = tail.find(':')?;
        (&spec[..2 + pos], &tail[pos + 1..])
    } else {
        let pos = spec.find(':')?;
        (&spec[..pos], &spec[pos + 1..])
    };

    // Split target from optional options.
    let (target, options) = if let Some(pos) = rest.find(':') {
        (&rest[..pos], &rest[pos + 1..])
    } else {
        (rest, "")
    };

    Some((source, target, options.contains("ro")))
}

#[cfg(test)]
mod tests;
