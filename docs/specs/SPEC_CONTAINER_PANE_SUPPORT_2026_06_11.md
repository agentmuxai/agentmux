# SPEC: Container Pane Support

**Date:** 2026-06-11
**Author:** AgentY
**Status:** Draft — Research Complete
**Related:** a5af/claw docker/, https://github.com/anthropics/claude-code/issues/56593

---

## Overview

Add support for agent panes that run Claude Code inside Docker containers rather than on the host. Agent panes will have a `type` of either `"standalone"` (current default) or `"container"` (new). Container panes spin up a named persistent Docker container per agent, exec Claude Code inside it, and mount the agent's working directory and credential volume.

---

## Independence Boundary

**AgentMux and claw are fully independent. There is no runtime dependency, shared code, or coupling between them.**

What this means concretely:

- AgentMux ships its own `Dockerfile.agent-agentmux`, its own entrypoint, its own `ContainerManager` in Rust. None of it imports, sources, or references anything from the claw repository.
- The similarity to claw's container setup (`node:22-slim`, non-root user, tini, named volumes) is because both arrived at the same Docker best practices independently. Claw is prior art and a reference implementation, not a dependency.
- If both systems are running on the same machine, they coexist without conflict: AgentMux names its containers `agentmux-<slug>`; claw names its containers `agent1`–`agent5`. The Docker networks, volumes, and compose files are separate.
- Claw users who also run AgentMux get a consistent experience (same base image, same workspace layout) as a UX convenience — not because the systems share infrastructure.

The only coordination point is the workspace directory layout (`~/.claw/workspaces/agent1/`) which AgentMux respects as a convention, not a dependency. AgentMux can work with any workspace path the user configures.

---

## Decision: Reuse Claw's Containers?

**Verdict: Reuse the base image and `docker-compose` infrastructure. Rewrite the entrypoint.**

### What claw's containers provide

| Component | Status | Notes |
|---|---|---|
| `node:22-slim` base image | ✅ Reuse | Correct base — glibc, slim, Claude Code-compatible |
| System tools (git, curl, jq, aws, gh, ripgrep, etc.) | ✅ Reuse | Production-hardened selection |
| `agent` non-root user (UID 1000) | ✅ Reuse | Host filesystem UID parity |
| Docker-in-Docker socket mount | ✅ Reuse | Needed if agents run docker commands |
| Modular entrypoint pattern (lib/*.sh) | ✅ Reuse pattern | Good design, bad coupling — see below |
| npm cache shared named volume | ✅ Reuse | Reduces install time across agents |
| Traefik + dnsmasq network | ✅ Reuse | For multi-agent port routing |

### What must be rewritten

| Component | Problem | AgentMux Replacement |
|---|---|---|
| Claude OAuth via `~/.claude` host mount + 2min sync loop | Requires claw on host; fragile across OS | Named Docker volume per agent; pass `ANTHROPIC_API_KEY` as env var |
| GitHub App tokens hardcoded per agent | App IDs baked into entrypoint — not portable | Dynamic config from settings; OR env var injection |
| AWS Secrets Manager for all tokens | Requires a5af AWS account | Pass tokens as env vars; SM as optional source |
| MCP warmup (`@a5af/` packages) | a5af-specific package names | Configurable warmup list; default to empty |
| VS Code Bridge health check at `host.docker.internal:3101` | Optional host service | Check only if `VSCODE_BRIDGE_URL` is set |
| SSH private key from Secrets Manager | Hardcoded server target | Remove from default image; user-composable |

### Summary

The claw entrypoint is a production-ready initialization system that happens to be tightly coupled to a5af's infrastructure (AWS account, GitHub App registrations, dev-tools packages). The Dockerfile itself is clean and portable. The right approach is:

1. Adopt `docker/Dockerfile.agent` as the upstream template — copy it, strip a5af-specific tool installations, add `tini`
2. Write a clean `entrypoint.sh` for AgentMux that accepts credentials via env vars and named volumes, sources optional user-provided lib scripts for extensibility
3. Keep docker-compose network topology (Traefik, dnsmasq, Playwright browsers) as the reference architecture for multi-agent setups

---

## Cross-Platform Best Practices

### Docker Socket Detection

Never hardcode a socket path. Detection order on all platforms:

```
1. DOCKER_HOST env var — user has already resolved their runtime; honor it
2. Platform-specific probe (see below)
3. Surface clear "Docker not found" error with install instructions
```

**Windows:**
```
Try: //./pipe/docker_engine
  ↳ Works for: Docker Desktop, Docker Engine (Moby) direct install
  ↳ Fails: pipe does not exist until the Docker service creates it (not running = file not found)
Fallback: wsl.exe docker version (query Docker inside WSL2)
Note: Never try to access /var/run/docker.sock from Win32 side
```

Docker Desktop is not the only path on Windows. Docker Engine (Moby) installs as a Windows service without Docker Desktop. For users with Docker Desktop licensing concerns, document the [Moby direct install via Microsoft script](https://docs.docker.com/engine/install/windows/) as a supported path.

**macOS:**
```
Use docker context inspect to get the active context's socket — handles all runtimes:
  Docker Desktop  → ~/.docker/run/docker.sock  (also symlinked to /var/run/docker.sock)
  OrbStack        → ~/.orbstack/run/docker.sock (also symlinked)
  Colima          → ~/.colima/default/docker.sock (no automatic symlink)
  Rancher Desktop → ~/.rd/docker.sock
```

Probe order if `docker context` fails:
`~/.docker/run/docker.sock` → `~/.orbstack/run/docker.sock` → `/var/run/docker.sock` → `~/.rd/docker.sock` → `~/.colima/default/docker.sock`

Recommended runtime for users: **OrbStack** (fastest, lowest config). **Colima** for licensing-sensitive or CLI-first workflows.

**Linux:**
```
Primary:  /var/run/docker.sock  (rootful Docker, requires docker group membership)
Rootless: /run/user/$UID/docker.sock  (rootless Docker, preferred for security)
Podman:   $XDG_RUNTIME_DIR/podman/podman.sock
```

If `/var/run/docker.sock` fails with a **permission error** (not file-not-found), surface the docker group fix. If file-not-found, check the rootless path. Document rootless Docker as the recommended setup for security-conscious deployments.

### Rust SDK: bollard

**Use bollard. It is the correct and only serious choice.**

```toml
# Cargo.toml
bollard = { version = "0.21", features = ["chrono"] }
```

| Criterion | bollard | Alternatives |
|---|---|---|
| Async (Tokio) | ✅ Full | `docker-api`: yes; CLI shelling out: no |
| Windows named pipe | ✅ First-class | `docker-api`: unknown; CLI: fragile |
| Podman compat | ✅ First-class | `docker-api`: no |
| API version | Docker 1.52 (latest) | `docker-api`: older |
| Maintenance | Active (v0.21, May 2026) | `docker-api`: stale |

`Docker::connect_with_defaults()` honors `DOCKER_HOST` and is cross-platform.
`Docker::connect_with_local_defaults()` uses OS-specific defaults (named pipe on Windows, unix socket elsewhere).

**For the startup probe, use `connect_with_defaults()` first; fall back to manual socket probing using the detection order above if it fails.**

### Podman

Support as secondary runtime on Linux/macOS at near-zero cost:

```rust
// After Docker probe fails, try Podman (Linux/macOS only)
#[cfg(not(windows))]
let docker = Docker::connect_with_podman_defaults(timeout)?;
```

Known compatibility gap: Podman's Docker-compat API omits `HostConfig.NetworkMode` and `NetworkSettings.Networks` from inspect responses. Not blocking for AgentMux's use case (exec, volumes, lifecycle). Not worth supporting on Windows (small user base).

---

## Container Image Design

### Base Image

**Use `node:22-slim` (Debian Bookworm slim).**

| Image | glibc | Claude Code | Notes |
|---|---|---|---|
| `node:22-slim` | ✅ | ✅ | **Recommended** — 220MB, correct ABI |
| `node:22` full | ✅ | ✅ | Too large (~950MB) |
| `ubuntu:24.04` + node | ✅ | ✅ | Fine, slightly larger |
| `node:22-alpine` | ❌ musl | ❌ Crashes | Confirmed broken with Claude Code |

Alpine is ruled out. Claude Code has native binaries that require glibc.

### Signal Handling: tini as PID 1

**Install `tini` and use it as the entrypoint.** This is the most robust fix for Docker's signal-propagation gap with `docker exec`.

```dockerfile
RUN apt-get install -y --no-install-recommends tini
ENTRYPOINT ["/usr/bin/tini", "--", "claude"]
```

Why:
- `docker exec -i` without `-t` does NOT forward signals (SIGINT/SIGTERM) from the host process to the exec'd process. This is a long-standing Docker issue ([docker/cli#2607](https://github.com/docker/cli/issues/2607)).
- `tini` as PID 1 handles signal reaping and zombie reaping correctly.
- Always use exec-form CMD/ENTRYPOINT (`["claude"]` not `claude`). Shell-form makes bash PID 1, which swallows signals.
- For stopping a running agent: use `docker stop` (sends SIGTERM to PID 1 → tini forwards to claude) rather than trying to signal the exec session directly.

### Claude Code Installation

```dockerfile
FROM node:22-slim

# Reproducible builds: pin exact version
ARG CLAUDE_VERSION=latest
RUN npm install -g @anthropic-ai/claude-code@${CLAUDE_VERSION}

# Prevent in-container auto-updates
ENV DISABLE_AUTOUPDATER=1
ENV NO_UPDATE_NOTIFIER=1
```

**Version update strategy:** Build a new image tag with each Claude Code release. Provide an explicit "Update agent image" action in the AgentMux UI. Never auto-update inside running containers — it mutates state unpredictably.

### Full Dockerfile

```dockerfile
FROM node:22-slim

# System dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    tini git curl jq bash ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Non-root user with UID 1000 (matches host agent user on Linux)
RUN useradd -m -u 1000 -s /bin/bash agent

# Claude Code
ARG CLAUDE_VERSION=latest
RUN npm install -g @anthropic-ai/claude-code@${CLAUDE_VERSION}

ENV DISABLE_AUTOUPDATER=1
ENV NO_UPDATE_NOTIFIER=1
ENV HOME=/home/agent
ENV CLAUDE_CONFIG_DIR=/home/agent/.claude

WORKDIR /workspace
USER agent

ENTRYPOINT ["/usr/bin/tini", "--", "claude"]
```

This is intentionally minimal. Add tools (gh CLI, aws, etc.) in a derived image or via claw's existing Dockerfile as the upstream.

---

## Volume Strategy

### Per-Platform Workspace Mounting

**Windows:**
Pass native Windows paths directly to Docker. Docker Desktop translates them to Linux paths inside the container automatically.

```
Host:      C:\Users\asafe\.claw\workspaces\agent1
Mount arg: C:/Users/asafe/.claw/workspaces/agent1:/workspace
Container: /workspace
```

Do NOT mount from `/mnt/c/...` (WSL2 virtio-9p path) for performance-sensitive workspaces. Files accessed via `/mnt/c` cross the WSL2 filesystem boundary on every stat/open/write.

**Performance caveat (Windows):** inotify/file-watch events are unreliable across the Windows↔WSL2 boundary. Set `CHOKIDAR_USEPOLLING=true` for file watchers (copied from claw's env template — already correct).

**macOS and Linux:** No path translation needed. Bind mount directly.

### Volume Assignments

| Data | Volume Type | Rationale |
|---|---|---|
| Workspace (user's code) | Bind mount from host | User must see changes on host filesystem |
| `~/.claude` (credentials + history) | Named Docker volume | Isolated per agent; survives container recreate; not exposed to host |
| `node_modules`, build artifacts | Named Docker volume | Avoids NTFS/APFS overhead on Windows/macOS |
| Read-only config files | Bind mount `:ro` | Static; host is source of truth |

**Do NOT bind-mount `~/.claude` from the host.** Anthropic's recommendation and our research both confirm: use a named volume per agent, and pass `ANTHROPIC_API_KEY` as an environment variable. This prevents cross-agent credential leakage and avoids the NTFS `ReadOnly` attribute issue documented in [#56593](https://github.com/anthropics/claude-code/issues/56593).

```yaml
# docker-compose reference
volumes:
  agent1-claude-config:  # named volume, isolated per agent
  npm-cache:             # shared across agents (read-write ok, no secrets)

services:
  agent1:
    volumes:
      - ${HOME}/.claw/workspaces/agent1:/workspace
      - agent1-claude-config:/home/agent/.claude
      - npm-cache:/home/agent/.npm
    environment:
      - ANTHROPIC_API_KEY=${ANTHROPIC_API_KEY}
```

---

## Container Lifecycle

### Persistent Container (Recommended)

**One persistent container per agent pane.** This is the VS Code Dev Containers model and the correct design for Claude Code agents.

| Factor | Persistent (exec per turn) | Ephemeral (new container per turn) |
|---|---|---|
| Cold start latency | None (already running) | +1–3s per turn |
| Installed tool state | Preserved | Lost |
| Build artifacts | Preserved | Lost |
| Claude Code session state | Preserved | Must reload from volume |
| Container image pull frequency | Once per update | Every turn (or cached) |

Claude Code makes tool calls that install dependencies and run builds — state must persist across calls within a session. Ephemeral containers make this impossible without full reinit on every call.

### Lifecycle State Machine

```
MISSING → docker run → RUNNING → docker stop → STOPPED
              ↑                                     |
              └──────────── docker start ───────────┘
                      (fast — no image pull needed)
```

- `ensure_running()`: idempotent. RUNNING → return immediately. STOPPED → `docker start`. MISSING → `docker run` (+ pull image if needed).
- `docker run --name agentmux-agent1 -d ...` with explicit `--name` for deterministic lookup.
- **Never use `--rm`.** Removing the container on stop loses the named volume binding and requires full `docker run` on next start.
- **Stop strategy:** `docker stop --time 10` (sends SIGTERM to PID 1 / tini, waits 10s, then SIGKILL).

### Signal Routing

```
AgentMux UI "Stop" → docker.stop_container(id) → SIGTERM → tini → claude (graceful shutdown)
                                                ↓ (10s timeout)
                                            SIGKILL → container exits
```

Do NOT try to signal the `docker exec` child process directly — it does not cross PID namespace boundaries reliably. Route all lifecycle signals through `docker stop`.

---

## Architecture Changes

### What Already Exists in agentmux-srv

| Field | Location | Status |
|---|---|---|
| `agent_type` | `db_agent_definitions` | ✅ Exists (`"standalone"` or `"container"`) |
| `environment` | `db_agent_definitions` | ✅ Exists (`"windows"` or `"linux"`) |
| `environment` | `CommandAgentDefineData` (RPC) | ✅ In wire types |
| `Controller` trait | `subprocess.rs:865` | ✅ Abstraction point |
| Spawn site | `subprocess.rs:388–449` | ✅ Single site to branch |

### What Needs to Be Built

| Gap | Scope |
|---|---|
| `bollard` dependency | `Cargo.toml` |
| `container.rs` — `ContainerManager` | ~500 lines new |
| `container_image`, `container_volumes`, `container_name` columns | DB migration + `agents.rs` |
| RPC fields in `CommandAgentDefineData` | `rpc_types.rs` |
| Spawn branching on `agent_type` | `subprocess.rs` |
| Startup Docker availability check | `main.rs` |
| Cross-platform socket detection | `container.rs` |
| Windows path translation utility | `container.rs` |
| Settings schema + template | `schema/settings.json`, `settings-template.jsonc` |

---

## Implementation Phases

### Phase 0 — Storage & Schema

**`agents.rs`** — Add to `AgentDefinition`:
```rust
pub container_image: String,    // "ghcr.io/agentmuxai/agent-claude:latest"
pub container_volumes: String,  // JSON array: ["/host/path:/workspace"]
pub container_name: String,     // "agentmux-<slug>" (set by server, not user)
```

**Migration:**
```sql
ALTER TABLE db_agent_definitions ADD COLUMN container_image   TEXT NOT NULL DEFAULT '';
ALTER TABLE db_agent_definitions ADD COLUMN container_volumes TEXT NOT NULL DEFAULT '[]';
ALTER TABLE db_agent_definitions ADD COLUMN container_name    TEXT NOT NULL DEFAULT '';
```

**`rpc_types.rs`** — Add to `CommandAgentDefineData`:
```rust
pub container_image:   Option<String>,
pub container_volumes: Option<Vec<String>>,
```

---

### Phase 1 — `ContainerManager`

**`Cargo.toml`:**
```toml
bollard = { version = "0.21", features = ["chrono"] }
```

**`agentmux-srv/src/backend/container.rs`:**

```rust
pub struct ContainerManager { client: Docker }

impl ContainerManager {
    /// Detect Docker/Podman. Returns Err if unavailable.
    /// Use this at startup; register result in app state.
    pub fn detect() -> Result<Self> {
        // 1. Honor DOCKER_HOST env
        // 2. OS-specific probe (named pipe on Windows, socket on Unix)
        // 3. On Linux/macOS: try Podman socket as fallback
        let docker = Docker::connect_with_defaults()
            .or_else(|_| Self::probe_platform_socket())
            .or_else(|_| Self::probe_podman())?;
        Ok(Self { client: docker })
    }

    /// Idempotent: returns container_name. Starts or creates if needed.
    pub async fn ensure_running(&self, def: &AgentDefinition) -> Result<String>;

    /// SIGTERM → 10s → SIGKILL. Does NOT remove container.
    pub async fn stop(&self, name: &str, timeout_secs: i64) -> Result<()>;

    /// Pull only if image not present locally.
    pub async fn pull_if_missing(&self, image: &str) -> Result<()>;

    /// ["docker", "exec", "-i", name]
    pub fn exec_prefix(name: &str) -> Vec<String>;

    /// Translate Windows path for Docker volume arg.
    /// "C:\Users\foo\bar" → "C:/Users/foo/bar"
    pub fn normalize_volume_path(path: &str) -> String;
}
```

**Container naming:**
```rust
fn container_name(def: &AgentDefinition) -> String {
    format!("agentmux-{}", def.slug)  // deterministic, stable across restarts
}
```

**Volume mounts built in `ensure_running`:**
```rust
let volumes = vec![
    format!("{}:/workspace", Self::normalize_volume_path(&def.working_directory)),
    format!("agentmux-claude-{}:/home/agent/.claude", def.slug),  // named volume
];
// Plus any user-configured volumes from def.container_volumes (JSON array)
```

---

### Phase 2 — Spawn Integration

Modify `subprocess.rs:388–449` (`spawn_turn`). Branch on `agent_type`:

```rust
let (cmd_path, cmd_args) = match block_meta.agent_type.as_str() {
    "container" => {
        let name = app_state.container_manager
            .as_ref()
            .ok_or("Docker not available")?
            .ensure_running(&agent_def)
            .await?;
        let mut args = ContainerManager::exec_prefix(&name);
        args.extend(base_args);
        (args.remove(0), args)
    }
    _ => (config.cli_command.clone(), base_args),
};
```

**Env injection for container exec:**

The existing `cmd.env(k, v)` calls translate to `-e K=V` flags in the exec prefix builder. Ensure the exec prefix includes `-e` for each injected env var:

```rust
pub fn exec_prefix_with_env(name: &str, env: &[(String, String)]) -> Vec<String> {
    let mut args = vec!["docker".into(), "exec".into(), "-i".into()];
    for (k, v) in env {
        args.extend(["-e".into(), format!("{k}={v}")]);
    }
    args.push(name.into());
    args
}
```

**PTY:** Use `-i` only. Never `-t`. `-t` allocates a pseudo-TTY and corrupts NDJSON output.

---

### Phase 3 — Lifecycle Management

- **Crash detection:** Existing `process_waiter` watches the `docker exec` child. Non-zero exit (container died mid-turn) → existing crash/restart logic fires. Next turn's `ensure_running()` restarts container.
- **Graceful stop:** `docker.stop_container(id, Some(10))` — SIGTERM via tini → 10s → SIGKILL.
- **Health check in image:**
  ```dockerfile
  HEALTHCHECK --interval=30s --timeout=5s CMD claude --version || exit 1
  ```
- **Container status API:** Expose via RPC so the frontend can show "container starting" / "running" / "stopped" badges on panes.

---

### Phase 4 — Pre-built Image

Build and push `ghcr.io/agentmuxai/agent-claude` on every release:

```yaml
# .github/workflows/container-image.yml
on:
  push:
    tags: ['v*']

jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: docker/build-push-action@v5
        with:
          context: docker/
          file: docker/Dockerfile.agent-agentmux
          tags: |
            ghcr.io/agentmuxai/agent-claude:latest
            ghcr.io/agentmuxai/agent-claude:${{ github.ref_name }}
          build-args: |
            CLAUDE_VERSION=${{ env.CLAUDE_VERSION }}
```

---

### Phase 5 — Settings & UI

**`settings-template.jsonc`:**
```jsonc
{
    "name": "Agent1",
    "provider": "claude",
    "agent_type": "container",
    "environment": "linux",
    "container_image": "ghcr.io/agentmuxai/agent-claude:latest",
    "container_volumes": [
        // Workspace mounted automatically from working_directory
        // Add extra mounts here:
        // "/host/extra:/container/path"
    ],
    "working_directory": "/workspace"
}
```

**Agent define UI:**
- `environment = "linux"` selected → show Container Image + Volumes fields
- Container image: text input with `ghcr.io/agentmuxai/agent-claude:latest` placeholder
- Volumes: list editor (host_path:container_path pairs)
- Pane header: Docker badge distinguishes container panes from standalone

---

### Phase 6 — Startup Docker Check

```rust
// agentmux-srv/src/main.rs
let container_manager = match ContainerManager::detect() {
    Ok(mgr) => {
        match mgr.client.version().await {
            Ok(v) => info!("Docker available: {:?}", v.version),
            Err(e) => warn!("Docker connected but version check failed: {e}"),
        }
        Some(mgr)
    }
    Err(e) => {
        warn!("Docker not available — container agents disabled: {e}");
        // UI: container pane type grayed out with "Docker required" tooltip
        None
    }
};
app_state.container_manager = container_manager;
```

**Rule:** Never hard-fail on Docker absence. Standalone agents must work regardless.

---

## Open Questions

1. **claw interop:** Claw already manages agent1–5 as docker-compose containers. Should AgentMux adopt the same container names (`agentmux-agent1` vs `agent1`)? Or manage its own containers alongside claw's? Safest default: AgentMux uses `agentmux-<slug>` naming and ignores claw-managed containers unless the user explicitly points an agent definition at a claw workspace.

2. **Credential provisioning:** Who provides `ANTHROPIC_API_KEY` to the container? Options:
   - Passed through from host env var at container start (simplest)
   - AgentMux reads from its own credential store and injects
   - Named volume with credential file (claw pattern, but recommended against by Anthropic)
   Decision needed before Phase 2.

3. **WSL2 workspace path performance:** If the user's claw workspace lives on `C:\` (Windows NTFS), bind-mounting it will have degraded inotify. Should AgentMux warn users and recommend moving workspaces into WSL2's Linux filesystem?

4. **Podman on macOS:** Low adoption compared to OrbStack/Colima. Probably not worth testing in CI but worth documenting for users who report it.

5. **`scripts/import-agents.sh` interaction:** The import script rehydrates Claude session transcripts into `~/.claude` paths. If `~/.claude` is a named Docker volume, the script's rehydration logic needs updating to write into the volume. This is a cross-team concern.

---

## Estimated Scope

| Component | New Lines | Modified Lines |
|---|---|---|
| `container.rs` (new) | ~550 | — |
| `subprocess.rs` branching | — | ~60 |
| `agents.rs` + migration | ~30 | ~80 |
| `rpc_types.rs` | ~20 | ~10 |
| `main.rs` startup check | ~25 | ~10 |
| `Cargo.toml` | ~2 | — |
| `Dockerfile.agent-agentmux` (new) | ~35 | — |
| Schema + settings template | ~50 | ~20 |
| **Total** | **~715** | **~180** |

Phases 0, 1, and 4 can land in parallel. Phase 2 gates everything downstream.

---

## Branch Sequencing

| Phase | Branch | Depends On |
|---|---|---|
| 0 | `agenty/container-schema` | — |
| 1 | `agenty/container-manager` | — |
| 4 | `agenty/container-image` | — |
| 2 | `agenty/container-spawn` | P0, P1 |
| 3 | `agenty/container-lifecycle` | P2 |
| 5 | `agenty/container-settings` | P0 |
| 6 | `agenty/container-startup-check` | P1 |
