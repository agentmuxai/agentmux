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
use std::sync::Arc;
use tokio::sync::Mutex;
use bollard::Docker;
use bollard::container::{
    Config, CreateContainerOptions, ListContainersOptions, StartContainerOptions,
    StopContainerOptions,
};
use bollard::exec::{CreateExecOptions, StartExecOptions, StartExecResults};
use bollard::models::{HostConfig, Mount, MountTypeEnum};

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
}

/// Exec session handle for a single turn.
///
/// `output` is a bollard `StartExecResults` stream. With `tty: false` +
/// `attach_stderr: true`, Docker multiplexes stdout and stderr as separate
/// `LogOutput::StdOut` / `LogOutput::StdErr` frames (not merged). Phase 2
/// should fan both into the block's output channel.
pub struct ExecSession {
    pub output: StartExecResults,
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
        let _guard = container_lock.lock().await;

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
    pub async fn exec(
        &self,
        container_name: &str,
        cmd: &[String],
        working_dir: Option<&str>,
        env_vars: &[(String, String)],
    ) -> Result<ExecSession, ContainerError> {
        let env: Vec<String> = env_vars.iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();

        let exec = self.inner.docker
            .create_exec(container_name, CreateExecOptions {
                attach_stdin: Some(true),
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                tty: Some(false), // NO tty — preserves NDJSON newlines
                cmd: Some(cmd.iter().map(String::as_str).collect()),
                working_dir,
                env: if env.is_empty() { None } else { Some(env.iter().map(String::as_str).collect()) },
                ..Default::default()
            })
            .await?;

        let output = self.inner.docker
            .start_exec(&exec.id, Some(StartExecOptions {
                detach: false,
                ..Default::default()
            }))
            .await?;

        Ok(ExecSession { output })
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

    async fn create_and_start(
        &self,
        container_name: &str,
        image: &str,
        volumes: &[String],
        env_vars: &[(String, String)],
    ) -> Result<(), ContainerError> {
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
}
