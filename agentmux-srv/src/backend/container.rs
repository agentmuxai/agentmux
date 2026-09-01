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

/// Env var names that reference host-filesystem paths and must NOT be forwarded
/// into a container via `docker exec -e`. The container image supplies its own
/// values for these (e.g. `CLAUDE_CONFIG_DIR=/home/agent/.claude` baked in).
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
/// stdin never reaches EOF (see [`ContainerManager::exec`]). Turn input belongs
/// in argv. `output` is a bollard `LogOutput` stream; with
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

        let result = self.ensure_running_locked(container_name, image, volumes, env_vars).await;

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
    ) -> Result<(), ContainerError> {
        // Always query Docker — do not rely on an in-memory cache. The container
        // can be stopped or removed externally (e.g. `docker rm -f`), and a
        // stale cache entry would cause ensure_running to silently no-op while
        // a subsequent exec call fails.
        let existing = self.find_container(container_name).await?;

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
                self.create_and_start(container_name, image, volumes, env_vars).await?;
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
    /// no exit. Pass its input in argv instead.
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

        // claude-config named volume: ensure ~/.claude persists across container restarts.
        // Using a named volume (not a host bind mount) avoids credential leakage from the
        // host's .claude directory into the container.
        let mut all_mounts = vec![
            Mount {
                target: Some("/home/agent/.claude".to_string()),
                source: Some(format!("agentmux-claude-{container_name}")),
                typ: Some(MountTypeEnum::VOLUME),
                read_only: Some(false),
                ..Default::default()
            },
        ];
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
        cm.ensure_running(name, image, &[], &[]).await.expect("ensure_running (create)");
        // reuse path: already running → must no-op, not error or re-create
        cm.ensure_running(name, image, &[], &[]).await.expect("ensure_running (reuse)");

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
