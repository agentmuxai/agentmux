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
    /// container_name → running-state cache.
    state: Mutex<HashMap<String, ContainerState>>,
}

#[derive(Debug, Clone, PartialEq)]
enum ContainerState {
    Running,
    Stopped,
}

/// Exec session handles for a single turn. Contains the two async byte-streams
/// (stdout + stderr merged into stdout, stderr empty) that Phase 2 will pipe
/// into the block's stdin/stdout channels.
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
                state: Mutex::new(HashMap::new()),
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
    /// Returns the container name (same as `container_name` in `AgentDefinition`).
    pub async fn ensure_running(
        &self,
        container_name: &str,
        image: &str,
        volumes: &[String],
        env_vars: &[(String, String)],
    ) -> Result<(), ContainerError> {
        let mut state = self.inner.state.lock().await;
        if state.get(container_name) == Some(&ContainerState::Running) {
            return Ok(());
        }
        drop(state); // release lock before async Docker calls

        // Check if container exists in Docker
        let existing = self.find_container(container_name).await?;

        match existing {
            Some(status) if status == "running" => {
                let mut state = self.inner.state.lock().await;
                state.insert(container_name.to_string(), ContainerState::Running);
            }
            Some(_) => {
                // Exists but stopped — start it
                self.inner.docker
                    .start_container(container_name, None::<StartContainerOptions<String>>)
                    .await?;
                let mut state = self.inner.state.lock().await;
                state.insert(container_name.to_string(), ContainerState::Running);
                tracing::info!(container = container_name, "restarted stopped container");
            }
            None => {
                // Create and start
                self.create_and_start(container_name, image, volumes, env_vars).await?;
                let mut state = self.inner.state.lock().await;
                state.insert(container_name.to_string(), ContainerState::Running);
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
        let mut state = self.inner.state.lock().await;
        state.insert(container_name.to_string(), ContainerState::Stopped);
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
        let mut state = self.inner.state.lock().await;
        state.remove(container_name);
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

        // Parse volume specs: "source:target" or "source:target:options"
        // Named volumes (no path separator in source) are treated as Docker named volumes;
        // paths starting with / are bind mounts.
        let mounts: Vec<Mount> = volumes.iter().filter_map(|spec| {
            let parts: Vec<&str> = spec.splitn(3, ':').collect();
            if parts.len() < 2 {
                tracing::warn!(spec = spec, "ignoring malformed volume spec (expected source:target)");
                return None;
            }
            let source = parts[0];
            let target = parts[1];
            let read_only = parts.get(2).map(|o| o.contains("ro")).unwrap_or(false);
            let mount_type = if source.starts_with('/') || source.starts_with('~') ||
                             source.len() > 1 && source.chars().nth(1) == Some(':') {
                // Absolute path → bind mount
                MountTypeEnum::BIND
            } else {
                // Name without path → named volume
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
            // Keep container alive between turns — entrypoint keeps process running.
            // The agent-claude image uses `tini` as PID 1 so SIGTERM routes correctly.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_container_name_for_slug() {
        assert_eq!(container_name_for_slug("my-agent"), "agentmux-my-agent");
        assert_eq!(container_name_for_slug("agent1"), "agentmux-agent1");
    }
}
