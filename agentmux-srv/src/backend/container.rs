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
//!   - Windows: named pipe `//./pipe/docker_engine`
//!   - macOS:   socket resolved via `docker context inspect` (Docker Desktop or Rancher)
//!   - Linux:   `/var/run/docker.sock` or rootless path in XDG_RUNTIME_DIR
//!
//! Independence boundary: this module has no dependency on a5af/claw.
//! Docker best practices here were researched independently.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use tokio::io::AsyncWrite;
use tokio::sync::Mutex;
use futures_util::StreamExt as _;
use bollard::Docker;
use bollard::container::{
    Config, CreateContainerOptions, ListContainersOptions, StartContainerOptions,
    StopContainerOptions,
};
use bollard::exec::{CreateExecOptions, StartExecOptions, StartExecResults};
use bollard::image::CreateImageOptions;
use bollard::models::{HostConfig, Mount, MountTypeEnum};

/// Where the agent CLI's config dir lives inside the image (`CLAUDE_CONFIG_DIR`).
pub const CONTAINER_CLAUDE_DIR: &str = "/home/agent/.claude";
/// Where the agent's working directory is bind-mounted inside the image.
pub const CONTAINER_WORKSPACE_DIR: &str = "/workspace";

/// Host directories a container agent needs mounted to be able to work.
///
/// Both were missing entirely before 2026-09-02, which is why container agents
/// could never authenticate and always started with an empty `/workspace`. See
/// `docs/reports/REPORT_CONTAINER_AGENT_CREDENTIALS_AND_WORKSPACE_2026_09_02.md`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContainerMountSpec {
    /// The bound Armory account's own resolved Claude config dir on the host —
    /// i.e. whatever `CLAUDE_CONFIG_DIR` the identity resolver produced for this
    /// agent. Its `.credentials.json` is bind-mounted into the container so the
    /// CLI there reads the same token the Armory login wrote (see
    /// [`agent_home_mounts`] for why it is that one file and not the whole dir).
    ///
    /// This is deliberately the ONE account's isolated dir, never the operator's
    /// global `~/.claude` — reaching into that would be the credential leak the
    /// named-volume design was avoiding.
    ///
    /// `None` keeps the old behavior (an empty per-agent named volume), which is
    /// correct only for an agent with no bound oauth account.
    pub claude_config_host_dir: Option<String>,
    /// The agent's working directory on the host, bind-mounted at
    /// [`CONTAINER_WORKSPACE_DIR`]. `None`/empty leaves `/workspace` empty, which
    /// is what shipped before this existed.
    pub workspace_host_dir: Option<String>,
}

/// Normalize a host path for Docker's mount API: backslashes are not accepted in
/// a bind source, so `C:\Users\x` has to travel as `C:/Users/x`.
fn normalize_host_path(path: &str) -> String {
    path.replace('\\', "/")
}

/// Canonical form of a mount source for comparison.
///
/// Case folding is Windows-only, deliberately (codex P2 on PR #2933). Docker on
/// Windows can report a differently-cased drive letter than we asked for, so a
/// case-sensitive compare there produces phantom drift and a needless recreate.
/// Folding UNCONDITIONALLY is the opposite bug: on a case-sensitive host,
/// switching an agent's workspace from `/work/Foo` to the genuinely different
/// `/work/foo` would look like a match, `ensure_running` would skip the
/// recreate, and `/workspace` would keep serving the old directory.
fn fold_mount_source(source: &str) -> String {
    let normalized = normalize_host_path(source);
    if cfg!(windows) {
        normalized.to_lowercase()
    } else {
        normalized
    }
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
/// Returning `None` degrades to exactly the previous behavior (an empty per-agent
/// volume) instead, and logs why. Keychain-backed provisioning is a separate
/// piece of work; this only guarantees we do not regress it.
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

/// The mounts that make up the agent's home dir inside the container.
///
/// The per-agent named volume stays mounted at [`CONTAINER_CLAUDE_DIR`] exactly
/// as before, and a bound account contributes ONE extra mount: its
/// `.credentials.json`, bind-mounted as a single file on top of that volume.
///
/// Binding the account's whole config dir over `…/.claude` would be the more
/// obvious shape, and it does NOT work — both failures verified live on
/// 2026-09-02 against `ghcr.io/agentmuxai/agent-claude:latest`:
///
///  1. That dir's `projects` entry is a symlink into the shared identities tree,
///     which does not resolve in-container (`ls` → "No such file or directory").
///  2. Shadowing it with a nested volume at `…/.claude/projects` SILENTLY does
///     not mount — Docker will not mount over a path that is a symlink inside a
///     bind mount. `mount` shows only the parent 9p bind, and the CLI is left
///     writing session state through the broken link. Chowning does not help;
///     there is nothing there to chown.
///
/// Keeping the volume and binding just the credential file avoids both, and
/// preserves the existing ownership story: the volume is initialised from the
/// image, so `projects`/`sessions` stay writable by uid 1000. Verified end to
/// end — a container agent using this shape completed a real turn against the
/// bound account (`"result":"AUTH_OK"`, `is_error: false`).
///
/// Known limitation — HOST UID PARITY. A bind preserves the host file's owner
/// and mode (`.credentials.json` is typically 0600), while the image always
/// execs as `agent`, uid 1000. On a host whose AgentMux user is not uid 1000,
/// the container cannot read the token and authentication stays broken there.
///
/// That is deliberately NOT worked around here. It is the container design's
/// pre-existing, documented assumption — "`agent` non-root user (UID 1000) …
/// Host filesystem UID parity" (SPEC_CONTAINER_PANE_SUPPORT_2026_06_11.md §
/// "What claw's containers provide"; the Dockerfile renames node:22-slim's
/// uid-1000 user for exactly this) — and the `/workspace` bind below has the
/// identical problem: at uid != 1000 the agent cannot write its own workspace
/// either. Special-casing credentials would fix one symptom of that assumption
/// and leave the other, while adding a second provisioning path to maintain.
/// The real fix is to exec as the host uid/gid, which is an architectural change
/// affecting every mount, and belongs in its own PR.
///
/// Known limitation: a single-file bind follows the inode. If the CLI ever
/// replaces `.credentials.json` by atomic rename rather than writing in place, a
/// refreshed token would land on the host but not be seen by the running
/// container until it is recreated. Reads — the case that was totally broken —
/// are unaffected.
fn agent_home_mounts(container_name: &str, spec: &ContainerMountSpec) -> Vec<Mount> {
    let mut mounts = vec![Mount {
        target: Some(CONTAINER_CLAUDE_DIR.to_string()),
        source: Some(format!("agentmux-claude-{container_name}")),
        typ: Some(MountTypeEnum::VOLUME),
        read_only: Some(false),
        ..Default::default()
    }];

    if let Some(host_dir) = spec.claude_config_host_dir.as_deref().filter(|d| !d.is_empty()) {
        let host_dir = normalize_host_path(host_dir);
        mounts.push(Mount {
            target: Some(format!("{CONTAINER_CLAUDE_DIR}/{CLAUDE_CREDENTIALS_FILE}")),
            source: Some(format!("{}/{CLAUDE_CREDENTIALS_FILE}", host_dir.trim_end_matches('/'))),
            typ: Some(MountTypeEnum::BIND),
            // Writable: an in-place token refresh should reach the host copy
            // rather than being silently confined to the container.
            read_only: Some(false),
            ..Default::default()
        });
    }
    mounts
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
    "GH_CONFIG_DIR",
    "PATH",
    "HOME",
    "USERPROFILE",
    "HOMEDRIVE",
    "HOMEPATH",
    "TMPDIR",
    "TEMP",
    "TMP",
];

/// Shared container manager. Clone-on-Arc; cheap to pass around.
#[derive(Clone)]
pub struct ContainerManager {
    inner: Arc<ContainerManagerInner>,
}

struct ContainerManagerInner {
    docker: Docker,
    /// Per-container serialization lock: prevents concurrent ensure_running calls
    /// for the same container from both seeing "not found" in Docker and both
    /// attempting create_container (which would fail with a 409 name-conflict
    /// on the second caller).
    ensure_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    /// Per-container `(uid, gid)` of the user execs run as, resolved once and
    /// cached — see [`ContainerManager::exec_identity`].
    exec_identities: Mutex<HashMap<String, (u64, u64)>>,
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
}

impl ContainerManager {
    /// Connect to the local Docker daemon using environment/platform defaults.
    ///
    /// Honors `DOCKER_HOST` env var (e.g. for rootless or remote daemons).
    /// On Windows connects via named pipe; on macOS/Linux via Unix socket.
    pub fn connect() -> Result<Self, ContainerError> {
        let docker = Docker::connect_with_local_defaults()
            .map_err(|e| ContainerError::NotAvailable(e.to_string()))?;
        Ok(Self {
            inner: Arc::new(ContainerManagerInner {
                docker,
                ensure_locks: Mutex::new(HashMap::new()),
                exec_identities: Mutex::new(HashMap::new()),
            }),
        })
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
    pub async fn ensure_running(
        &self,
        container_name: &str,
        image: &str,
        volumes: &[String],
        env_vars: &[(String, String)],
        spec: &ContainerMountSpec,
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

        let result = self.ensure_running_locked(container_name, image, volumes, env_vars, spec).await;

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
    ) -> Result<(), ContainerError> {
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
            Some(_) if !self.mounts_match(container_name, spec).await => {
                tracing::info!(
                    container = container_name,
                    "container mounts differ from the desired spec (credentials/workspace) — recreating"
                );
                let _ = self.stop(container_name, 10).await;
                self.remove(container_name, true).await?;
                None
            }
            other => other,
        };

        match existing {
            Some(status) if status == "running" => {
                // Already running — nothing to do.
            }
            Some(_) => {
                // Exists but stopped — start it.
                self.inner.docker
                    .start_container(container_name, None::<StartContainerOptions<String>>)
                    .await?;
                tracing::info!(container = container_name, "restarted stopped container");
            }
            None => {
                // Pull the image via Docker socket before create so the Engine API
                // has the image locally. Unlike `docker run`, the Engine's
                // create_container does NOT auto-pull — it returns 404 if the image
                // is absent. pull_image is a no-op when the image already exists.
                self.pull_image(image).await?;
                // Create and start.
                self.create_and_start(container_name, image, volumes, env_vars, spec).await?;
                tracing::info!(container = container_name, image = image, "created and started container");
            }
        }
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
        Ok(())
    }

    /// Remove a container (must be stopped first or use `force = true`).
    pub async fn remove(&self, container_name: &str, force: bool) -> Result<(), ContainerError> {
        use bollard::container::RemoveContainerOptions;
        self.inner.docker
            .remove_container(container_name, Some(RemoveContainerOptions {
                force,
                ..Default::default()
            }))
            .await?;
        tracing::info!(container = container_name, "removed container");
        Ok(())
    }

    // ---- private helpers ----

    /// Every mount target this module owns. Anything NOT in here (the caller's
    /// `agent:container_volumes`) is the user's business and is never a reason
    /// to rebuild their container.
    fn owned_mount_targets() -> [String; 3] {
        [
            CONTAINER_CLAUDE_DIR.to_string(),
            format!("{CONTAINER_CLAUDE_DIR}/{CLAUDE_CREDENTIALS_FILE}"),
            CONTAINER_WORKSPACE_DIR.to_string(),
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

    /// Does the existing container already carry exactly the mounts `spec` asks
    /// for? Used to decide whether an agent that is already up has to be
    /// recreated to pick up (or lose) credentials and a workspace.
    ///
    /// Compares the owned set in BOTH directions — a mount that should now be
    /// ABSENT is drift too, not just a missing or changed one. That case is the
    /// security-relevant one (reagent P1 on PR #2933): unbinding an agent's
    /// Armory account is a normal transition that never reaches
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
    /// Fails SAFE: any inspect error or missing data returns `true` ("matches"),
    /// leaving the container alone. A spurious recreate kills a live agent and
    /// loses its container-local state, which is worse than one more restart on
    /// a stale mount.
    async fn mounts_match(&self, container_name: &str, spec: &ContainerMountSpec) -> bool {
        let Ok(details) = self.inner.docker.inspect_container(container_name, None).await else {
            return true;
        };
        let Some(actual) = details.mounts else {
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
        Self::owned_mount_targets().iter().all(|target| {
            let want = desired.get(target).map(|s| fold_mount_source(s));
            source_at(target) == want
        })
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

    /// Pull `image` via the Docker socket (create_image API).
    ///
    /// Streams the pull response to completion before returning. If the image is
    /// already present locally, Docker returns an empty stream immediately — this
    /// function treats that as success (no pull needed).
    ///
    /// Errors only on genuine pull failures (network, auth, no such image).
    async fn pull_image(&self, image: &str) -> Result<(), ContainerError> {
        // Skip the pull entirely when the image is already present locally.
        // `create_image` always contacts the registry, so on an offline host the
        // pull stream errors out even when the image is cached — aborting
        // container creation even though `docker run` would start it from the
        // local cache. An `inspect_image` short-circuit makes the cached-offline
        // path work (and avoids a redundant registry round-trip when online).
        if self.inner.docker.inspect_image(image).await.is_ok() {
            tracing::info!(image = image, "image already present locally; skipping pull");
            return Ok(());
        }

        tracing::info!(image = image, "pulling image (not cached locally)");
        let mut stream = self.inner.docker.create_image(
            Some(CreateImageOptions {
                from_image: image,
                ..Default::default()
            }),
            None,
            None,
        );
        while let Some(item) = stream.next().await {
            match item {
                Ok(info) => {
                    if let Some(status) = info.status {
                        tracing::debug!(image = image, status = %status, "pull progress");
                    }
                }
                Err(e) => {
                    tracing::warn!(image = image, error = %e, "image pull error");
                    return Err(ContainerError::Docker(e));
                }
            }
        }
        tracing::info!(image = image, "image pull complete (or already cached)");
        Ok(())
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

        let env: Vec<String> = env_vars.iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();

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
        all_mounts.extend(mounts);

        let config: Config<String> = Config {
            image: Some(image.to_string()),
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
                ..Default::default()
            }),
            ..Default::default()
        };

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
}

/// State of the container-runtime slot held by [`ContainerRuntimeHandle`].
enum RuntimeSlot {
    /// Test/no-docker-expected fixtures — `get()` never attempts a real
    /// connect, always reports unavailable.
    Disabled,
    /// No manager yet: never connected, or the last connect attempt
    /// failed. `get()` retries `ContainerManager::connect()` on every
    /// call while in this state.
    Empty,
    Connected(ContainerManager),
}

/// Self-healing holder for the container runtime connection.
///
/// A plain `Option<ContainerManager>` fixed at process boot means a Docker
/// daemon that starts (or a socket/named-pipe that appears) AFTER AgentMux
/// launched is never picked up without an app restart — every consumer
/// (the availability probe AND the actual container-launch code paths)
/// would see a permanent `None`. This type retries on demand instead, so
/// "start Docker Desktop while AgentMux is already running" is picked up
/// within one call, everywhere `container_manager` is read.
///
/// See docs/retro/RETRO_DOCKER_DETECTION_DIVERGENCE_2026_07_04.md.
pub struct ContainerRuntimeHandle {
    slot: tokio::sync::RwLock<RuntimeSlot>,
}

impl ContainerRuntimeHandle {
    /// Boot-time constructor. Attempts one connect (same as the prior
    /// behavior) but never permanently gives up on failure — a failed
    /// attempt leaves the slot `Empty` so `get()`/`is_available()` retry
    /// later instead of staying stuck for the process lifetime.
    pub fn connect_at_startup() -> Self {
        let slot = match ContainerManager::connect() {
            Ok(mgr) => RuntimeSlot::Connected(mgr),
            Err(_) => RuntimeSlot::Empty,
        };
        Self { slot: tokio::sync::RwLock::new(slot) }
    }

    /// Test-only constructor: permanently reports unavailable and never
    /// attempts a real connect, keeping host-only unit tests hermetic and
    /// deterministic regardless of whether the test box has Docker.
    pub fn disabled() -> Self {
        Self { slot: tokio::sync::RwLock::new(RuntimeSlot::Disabled) }
    }

    /// Returns a connected manager, retrying `ContainerManager::connect()`
    /// if the slot is currently `Empty`. Cheap on the happy path (a
    /// read-lock plus a cheap `Arc`-backed clone); only takes the write
    /// lock and re-dials when we don't already have a manager.
    pub async fn get(&self) -> Option<ContainerManager> {
        {
            let slot = self.slot.read().await;
            match &*slot {
                RuntimeSlot::Connected(mgr) => return Some(mgr.clone()),
                RuntimeSlot::Disabled => return None,
                RuntimeSlot::Empty => {}
            }
        }
        let mut slot = self.slot.write().await;
        // Re-check under the write lock — another caller may have already
        // connected between our read-unlock and this write-lock.
        if let RuntimeSlot::Connected(mgr) = &*slot {
            return Some(mgr.clone());
        }
        if matches!(&*slot, RuntimeSlot::Disabled) {
            return None;
        }
        match ContainerManager::connect() {
            Ok(mgr) => {
                *slot = RuntimeSlot::Connected(mgr.clone());
                Some(mgr)
            }
            Err(_) => None,
        }
    }

    /// True iff a manager is available AND its daemon answers a live ping
    /// right now. Never trusts a cached `Connected` slot alone — the
    /// daemon can go down again after a successful connect, and
    /// `check_available` is a real `docker.ping()`, not a cached flag.
    pub async fn is_available(&self) -> bool {
        match self.get().await {
            Some(mgr) => mgr.check_available().await.is_ok(),
            None => false,
        }
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
mod tests {
    use super::*;

    #[test]
    fn test_container_name_for_slug() {
        assert_eq!(container_name_for_slug("my-agent"), "agentmux-my-agent");
        assert_eq!(container_name_for_slug("agent1"), "agentmux-agent1");
    }

    // ── Credential + workspace mounts ───────────────────────────────────────
    // See docs/reports/REPORT_CONTAINER_AGENT_CREDENTIALS_AND_WORKSPACE_2026_09_02.md

    fn mount_at<'a>(mounts: &'a [Mount], target: &str) -> Option<&'a Mount> {
        mounts.iter().find(|m| m.target.as_deref() == Some(target))
    }

    /// THE fix: the bound account's `.credentials.json` is bind-mounted into the
    /// container, so the CLI there reads the same token the Armory login wrote.
    /// Without it a container agent authenticates NEVER — `CLAUDE_CONFIG_DIR` is
    /// a host path and is stripped by CONTAINER_ENV_DENYLIST before exec.
    #[test]
    fn a_bound_accounts_credentials_file_is_bind_mounted_into_the_container() {
        let spec = ContainerMountSpec {
            claude_config_host_dir: Some(r"C:\Users\me\.agentmux\identities\acct\claude".into()),
            workspace_host_dir: None,
        };
        let mounts = agent_home_mounts("agentmux-x", &spec);

        let creds = mount_at(&mounts, "/home/agent/.claude/.credentials.json")
            .expect("credentials must be mounted");
        assert_eq!(creds.typ, Some(MountTypeEnum::BIND));
        assert_eq!(
            creds.source.as_deref(),
            Some("C:/Users/me/.agentmux/identities/acct/claude/.credentials.json"),
            "backslashes are not accepted in a Docker bind source",
        );
        assert_eq!(creds.read_only, Some(false), "an in-place token refresh must reach the host");
    }

    /// The named volume must REMAIN at ~/.claude. Binding the account's whole
    /// config dir there instead looks tidier and is broken: its `projects` entry
    /// is a symlink that does not resolve in-container, and a nested volume to
    /// shadow it silently fails to mount (both verified live, 2026-09-02).
    /// Keeping the volume is what leaves projects/sessions writable by uid 1000.
    #[test]
    fn the_claude_dir_itself_stays_the_per_agent_named_volume() {
        let spec = ContainerMountSpec {
            claude_config_host_dir: Some("/host/acct/claude".into()),
            workspace_host_dir: None,
        };
        let mounts = agent_home_mounts("agentmux-x", &spec);

        let claude = mount_at(&mounts, CONTAINER_CLAUDE_DIR).expect("claude dir mounted");
        assert_eq!(
            claude.typ,
            Some(MountTypeEnum::VOLUME),
            "must NOT become a bind of the host config dir — see agent_home_mounts",
        );
        assert_eq!(claude.source.as_deref(), Some("agentmux-claude-agentmux-x"));
        assert!(
            mount_at(&mounts, "/home/agent/.claude/projects").is_none(),
            "a nested projects mount does not work and must not be reintroduced",
        );
    }

    /// An agent with no bound account keeps exactly the shape that shipped
    /// before — this fix adds a mount, it does not rewrite the existing one.
    #[test]
    fn without_a_bound_account_the_named_volume_shape_is_unchanged() {
        let mounts = agent_home_mounts("agentmux-x", &ContainerMountSpec::default());

        assert_eq!(mounts.len(), 1, "no credential bind");
        let claude = mount_at(&mounts, CONTAINER_CLAUDE_DIR).expect("claude dir mounted");
        assert_eq!(claude.typ, Some(MountTypeEnum::VOLUME));
        assert_eq!(claude.source.as_deref(), Some("agentmux-claude-agentmux-x"));
    }

    /// An empty config dir is "no account", not a bind of "/.credentials.json".
    #[test]
    fn an_empty_config_dir_is_treated_as_no_bound_account() {
        let spec = ContainerMountSpec {
            claude_config_host_dir: Some(String::new()),
            workspace_host_dir: None,
        };
        assert_eq!(agent_home_mounts("agentmux-x", &spec).len(), 1);
    }

    // ── Drift detection must be bidirectional ───────────────────────────────
    // reagent P1 on PR #2933. `mounts_match` compares against these, so the
    // desired-set logic is what the tests pin; the Docker inspect half needs a
    // live daemon and is covered by the ignored integration test.

    /// Unbinding an agent's Armory account must show up as drift. It is a normal
    /// transition that never trips SpawnGateError::MissingCredentials, so if the
    /// desired set still claimed a credentials mount — or if a presence-only
    /// check ignored the now-absent one — the OLD account's token would stay
    /// bind-mounted and writable in the running container forever.
    #[test]
    fn unbinding_an_account_drops_the_credentials_mount_from_the_desired_set() {
        let bound = ContainerMountSpec {
            claude_config_host_dir: Some("/host/acct/claude".into()),
            workspace_host_dir: None,
        };
        let unbound = ContainerMountSpec::default();
        let creds_target = format!("{CONTAINER_CLAUDE_DIR}/{CLAUDE_CREDENTIALS_FILE}");

        let while_bound = ContainerManager::desired_owned_mounts("agentmux-x", &bound);
        assert_eq!(
            while_bound.get(&creds_target).map(String::as_str),
            Some("/host/acct/claude/.credentials.json"),
        );

        let after_unbind = ContainerManager::desired_owned_mounts("agentmux-x", &unbound);
        assert!(
            !after_unbind.contains_key(&creds_target),
            "an unbound agent must want NO credentials mount — otherwise the old \
             account stays authenticated inside the running container",
        );
        assert!(
            ContainerManager::owned_mount_targets().contains(&creds_target),
            "and that target must be one we compare, or its absence is never noticed",
        );
    }

    /// Clearing the working directory is the same shape of transition.
    #[test]
    fn clearing_the_working_directory_drops_the_workspace_from_the_desired_set() {
        let with_ws = ContainerMountSpec {
            claude_config_host_dir: None,
            workspace_host_dir: Some(r"C:\repo".into()),
        };
        let desired = ContainerManager::desired_owned_mounts("agentmux-x", &with_ws);
        assert_eq!(desired.get(CONTAINER_WORKSPACE_DIR).map(String::as_str), Some("C:/repo"));

        let without = ContainerManager::desired_owned_mounts("agentmux-x", &ContainerMountSpec::default());
        assert!(!without.contains_key(CONTAINER_WORKSPACE_DIR));
    }

    /// The comparison set must stay scoped to this module's own mounts, so a
    /// user's `container_volumes` can never trigger a rebuild of their container.
    #[test]
    fn only_this_modules_own_mount_targets_are_compared() {
        let owned = ContainerManager::owned_mount_targets();
        assert!(owned.contains(&CONTAINER_CLAUDE_DIR.to_string()));
        assert!(owned.contains(&CONTAINER_WORKSPACE_DIR.to_string()));
        assert!(owned.contains(&format!("{CONTAINER_CLAUDE_DIR}/{CLAUDE_CREDENTIALS_FILE}")));
        assert_eq!(owned.len(), 3, "a user volume target must never appear here");
    }

    /// macOS keeps OAuth credentials in the Keychain and never writes this file
    /// (oauth_probe.rs). Binding a nonexistent source makes Docker refuse to
    /// create the container, so an unguarded bind would turn "cannot
    /// authenticate on macOS" into "cannot START on macOS".
    #[test]
    fn a_config_dir_with_no_credentials_file_is_not_offered_for_mounting() {
        let dir = std::env::temp_dir().join("agentmux-creds-guard-empty");
        std::fs::create_dir_all(&dir).unwrap();
        let _ = std::fs::remove_file(dir.join(CLAUDE_CREDENTIALS_FILE));

        assert_eq!(
            credentials_dir_if_file_backed(&dir.to_string_lossy()),
            None,
            "no credentials file (the macOS Keychain case) must not produce a bind",
        );
        assert_eq!(credentials_dir_if_file_backed(""), None);
    }

    /// …and when the file IS there, it is offered, so the fix still applies on
    /// every platform that writes it.
    #[test]
    fn a_config_dir_holding_a_credentials_file_is_offered_for_mounting() {
        let dir = std::env::temp_dir().join("agentmux-creds-guard-present");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(CLAUDE_CREDENTIALS_FILE), b"{}").unwrap();

        assert_eq!(
            credentials_dir_if_file_backed(&dir.to_string_lossy()).as_deref(),
            Some(dir.to_string_lossy().as_ref()),
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Case folding must be Windows-only: on a case-sensitive host `/work/Foo`
    /// and `/work/foo` are different directories, and folding them together
    /// would hide real drift and leave /workspace on the stale one.
    #[test]
    fn mount_source_case_is_folded_only_where_the_filesystem_is_case_insensitive() {
        let a = fold_mount_source("/work/Foo");
        let b = fold_mount_source("/work/foo");
        if cfg!(windows) {
            assert_eq!(a, b, "windows paths are case-insensitive");
        } else {
            assert_ne!(a, b, "a case-sensitive host must see these as different mounts");
        }
        // Slash normalization is unconditional either way.
        assert_eq!(fold_mount_source(r"C:\a\b"), fold_mount_source("C:/a/b"));
    }

    #[test]
    fn host_paths_are_normalized_for_the_docker_mount_api() {
        assert_eq!(normalize_host_path(r"C:\Users\me\dir"), "C:/Users/me/dir");
        assert_eq!(normalize_host_path("/already/posix"), "/already/posix");
    }

    #[test]
    fn test_parse_volume_spec_unix_named() {
        let (src, tgt, ro) = parse_volume_spec("myvolume:/data").unwrap();
        assert_eq!(src, "myvolume");
        assert_eq!(tgt, "/data");
        assert!(!ro);
    }

    #[test]
    fn test_parse_volume_spec_unix_bind_readonly() {
        let (src, tgt, ro) = parse_volume_spec("/host/path:/container/path:ro").unwrap();
        assert_eq!(src, "/host/path");
        assert_eq!(tgt, "/container/path");
        assert!(ro);
    }

    #[test]
    fn test_parse_volume_spec_windows_bind() {
        // Drive-letter path must not be split at the drive colon.
        let (src, tgt, ro) = parse_volume_spec("C:\\Users\\me\\repo:/workspace").unwrap();
        assert_eq!(src, "C:\\Users\\me\\repo");
        assert_eq!(tgt, "/workspace");
        assert!(!ro);
    }

    #[test]
    fn test_parse_volume_spec_windows_bind_readonly() {
        let (src, tgt, ro) = parse_volume_spec("C:/Users/me/repo:/workspace:ro").unwrap();
        assert_eq!(src, "C:/Users/me/repo");
        assert_eq!(tgt, "/workspace");
        assert!(ro);
    }

    #[test]
    fn test_parse_volume_spec_malformed() {
        assert!(parse_volume_spec("nocodon").is_none());
    }

    /// Docker-gated integration test for the container lifecycle + exec path.
    /// Requires a reachable Docker daemon (Colima/Docker Desktop), so it is
    /// `#[ignore]` by default. Run with:
    ///   DOCKER_HOST=unix://$HOME/.colima/default/docker.sock \
    ///     cargo test -p agentmux-srv -- --ignored --nocapture itest_container
    ///
    /// Validates the foundation Phase 2 rests on against a real daemon:
    /// `ensure_running` create→reuse, env delivered into the exec via the Docker
    /// socket (`CreateExecOptions.env`, NOT argv — the CWE-214 guard), and the
    /// stdin→stdout exec I/O contract `spawn_container_turn` depends on.
    #[tokio::test]
    #[ignore]
    async fn itest_container_lifecycle_exec_env_and_io() {
        use tokio::io::AsyncWriteExt as _;

        async fn drain_stdout<S>(mut out: S) -> String
        where
            S: futures_util::Stream<
                    Item = Result<bollard::container::LogOutput, bollard::errors::Error>,
                > + Unpin,
        {
            use futures_util::StreamExt as _;
            let mut s = String::new();
            while let Some(item) = out.next().await {
                if let Ok(bollard::container::LogOutput::StdOut { message }) = item {
                    s.push_str(&String::from_utf8_lossy(&message));
                }
            }
            s
        }

        let cm = ContainerManager::connect().expect("connect to docker (set DOCKER_HOST)");
        cm.check_available().await.expect("docker daemon must be reachable");

        let name = "agentmux-itest-1357";
        // Public, pullable image with a long-lived default CMD (nginx daemon) and
        // a shell — `create_and_start` sets no cmd, so the image default must keep
        // PID 1 alive between exec turns (the persistent-container model).
        let image = "nginx:alpine";
        let _ = cm.remove(name, true).await; // clean slate; ignore if absent

        // create path: pull + create + start
        cm.ensure_running(name, image, &[], &[], &ContainerMountSpec::default()).await.expect("ensure_running (create)");
        // reuse path: already running → must no-op, not error or re-create
        cm.ensure_running(name, image, &[], &[], &ContainerMountSpec::default()).await.expect("ensure_running (reuse)");

        // env reaches the in-container process via the Docker socket, not argv.
        // Drop the (unused) stdin half first: bollard's exec output stream does
        // not EOF while the write-half is held open — the same reason
        // spawn_container_turn drops `input` before reading output.
        let ExecSession { input, output, .. } = cm
            .exec(
                name,
                &["sh".into(), "-c".into(), "printf %s \"$ITEST_KEY\"".into()],
                None,
                &[("ITEST_KEY".into(), "val-42".into())],
                false,
            )
            .await
            .expect("exec (env)");
        drop(input);
        let out = drain_stdout(output).await;
        assert!(out.contains("val-42"), "env must reach container; got {out:?}");

        // stdin → stdout I/O contract: write a newline-terminated message and
        // read it back. Use `read` (completes on the first '\n', then the shell
        // exits) rather than `cat` (which blocks until stdin EOF) — `claude`
        // likewise consumes newline-delimited JSON without waiting for EOF, and
        // bollard's hijacked exec stream does not reliably half-close stdin on
        // `drop(input)`, so an EOF-dependent reader would hang.
        let ExecSession { mut input, output, .. } = cm
            .exec(
                name,
                &["sh".into(), "-c".into(), "read line; printf 'got:%s' \"$line\"".into()],
                None,
                &[],
                true, // this test is specifically the stdin contract
            )
            .await
            .expect("exec (io)");
        input.write_all(b"hello-stdin\n").await.expect("write stdin");
        input.flush().await.expect("flush stdin");
        drop(input);
        let out = drain_stdout(output).await;
        assert!(out.contains("got:hello-stdin"), "stdin must reach the container process; got {out:?}");

        // cleanup
        cm.remove(name, true).await.expect("remove");
    }
}
