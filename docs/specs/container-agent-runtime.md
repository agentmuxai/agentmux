# Spec: Container Agent Runtime

**Date:** 2026-03-18
**Status:** historical — pre-implementation planning doc, superseded in practice
by the real shipped implementation and by `agentmuxai/agentmux#2939` (the
current, actively-maintained tracker for this feature). Kept for history only;
do not follow this doc's design choices. Concretely stale: references the old
`agentmuxsrv-rs` crate name (now `agentmux-srv`), the pre-rename "Forge" UI
(now "Armory"), and a `bollard`-crate execution design the real implementation
does not use (it shells `docker exec` directly — see `container.rs`,
`container_spawn.rs`). Corrected 2026-09-18 during a consolidation pass; the
original "proposed — Design" header had never been updated since 2026-03-18
despite the feature having since shipped and moved on architecturally.
**Priority:** High — container agents (1/2/3) are defined but can't run
**Research:** See `docs/research-container-runtime-detection.md` for full platform-by-platform analysis

---

## Problem

AgentMux has schema, UI, and seed data for container agents (Agent1=claude, Agent2=codex, Agent3=gemini), but **no runtime execution layer**. Selecting a container agent attempts to run the CLI directly on the host — which either fails or runs as a host agent.

## Current State

**What exists:**
- ForgeAgent schema: `agent_type` ("host"/"container"), `environment` ("windows"/"linux")
- Forge UI: groups agents by type, shows HOST/CONTAINER badges
- Seed manifest: 3 container agents with `working_directory: "/workspace"`, `shell: "bash"`
- Docker detection: frontend checks if `docker` CLI exists before launch
- Block metadata: `agentMode` stored per block

**What's missing:**
- No container launch code (no `docker run`)
- No container image specification
- No volume mounting
- No auth token passthrough
- No container lifecycle management
- Controller selection ignores `agent_type` — both types use `SubprocessController`

---

## Docker/Container Runtime Detection

### Detection Strategy (cross-platform)

```
1. Check for `docker` in PATH (where/which)
2. If found, check daemon: `docker info --format '{{.ServerVersion}}'`
3. If daemon not running → specific error: "Docker is installed but not running"
4. If not found, check for alternatives:
   - Podman: `podman` in PATH (drop-in Docker replacement)
   - nerdctl: `nerdctl` in PATH (containerd CLI)
```

### Platform-Specific Paths

| Platform | Docker Desktop | Alternative |
|----------|---------------|-------------|
| Windows | `C:\Program Files\Docker\Docker\resources\bin\docker.exe` | WSL2 + Docker inside WSL |
| macOS | `/usr/local/bin/docker` (symlink from Docker.app) | OrbStack, Colima, Podman |
| Linux | `/usr/bin/docker` | Podman (`/usr/bin/podman`) |

### Daemon Health Check

`docker info` is the most reliable check:
- Returns version + daemon status
- Fails with clear error if daemon not running
- Fails with permission error if user not in docker group (Linux)

`docker version` only checks client version — daemon could be down.

### Common Failure Modes

| Symptom | Cause | Fix |
|---------|-------|-----|
| `docker: command not found` | Not installed | Show install link |
| `Cannot connect to Docker daemon` | Daemon not running | "Start Docker Desktop" |
| `permission denied` | User not in docker group (Linux) | `sudo usermod -aG docker $USER` |
| `WSL integration not enabled` | Docker Desktop setting | "Enable WSL integration in Docker Desktop settings" |

---

## Auto-Install

Docker Desktop **cannot** be silently installed — it requires:
- Windows: admin/UAC elevation for the installer
- macOS: admin password for privileged helper
- Linux: `sudo` for apt/dnf/pacman

**Recommendation:** Don't auto-install. Instead:
1. Detect Docker is missing
2. Show clear error with platform-specific install instructions
3. Offer to open the download page in browser (`getApi().openExternal(url)`)

### Install Links

| Platform | Method | URL |
|----------|--------|-----|
| Windows | Docker Desktop | `https://desktop.docker.com/win/main/amd64/Docker%20Desktop%20Installer.exe` |
| macOS | Docker Desktop | `https://desktop.docker.com/mac/main/amd64/Docker.dmg` |
| Linux | apt/dnf | `https://docs.docker.com/engine/install/` |
| All | Podman | `https://podman.io/docs/installation` |

---

## Container Execution Architecture

### Option A: Wrap SubprocessController (Recommended)

Reuse `SubprocessController` but prefix the CLI command with `docker exec`:

```
Host agent:    claude -p --output-format stream-json ...
Container:     docker exec -i <container> claude -p --output-format stream-json ...
```

**Advantages:** Minimal code change, reuse existing I/O model
**Requirement:** Container must be pre-started and running

### Option B: ContainerController (New Controller Type)

Create a dedicated `ContainerController` that manages the full container lifecycle.

**Advantages:** Clean separation, full lifecycle control
**Disadvantages:** Much more code, duplicates SubprocessController I/O logic

### Recommendation: Option A with a container lifecycle layer

1. **On agent select:** Start container if not running (`docker run -d`)
2. **On each turn:** `docker exec -i <container> claude -p ...` via SubprocessController
3. **On idle timeout:** Stop container (`docker stop`)

---

## Container Configuration

### Per-Agent Container Spec

Extend ForgeAgent with container fields:

```typescript
// New fields on ForgeAgent
container_image?: string;     // e.g., "agentmux/agent-claude:latest"
container_volumes?: string;   // e.g., "/host/path:/workspace"
container_ports?: string;     // e.g., "3000:3000"
container_env?: string;       // additional env vars for container
```

### Default Container Image

Build and publish `agentmux/agent-base:latest`:

```dockerfile
FROM node:22-slim
RUN apt-get update && apt-get install -y git curl bash && rm -rf /var/lib/apt/lists/*
WORKDIR /workspace
# CLIs installed at runtime via npm (same as host flow)
```

Provider-specific images extend base:
- `agentmux/agent-claude:latest` — pre-installed Claude Code
- `agentmux/agent-codex:latest` — pre-installed Codex CLI
- `agentmux/agent-gemini:latest` — pre-installed Gemini CLI

### Volume Mounting

```
docker run -d \
  --name agentmux-agent1 \
  -v ~/.agentmux/agents/agent1:/workspace \
  -v ~/.agentmux/config/claude-agent1:/home/node/.claude \
  -e AGENTMUX_AGENT_ID=Agent1 \
  agentmux/agent-claude:latest \
  tail -f /dev/null
```

Key mounts:
- Working directory → `/workspace`
- CLI config (auth tokens) → provider-specific config dir
- SSH keys (optional) → `~/.ssh` (read-only)

### Auth Token Passthrough

The host handles CLI auth. Container needs access to auth tokens:
- **Claude:** Mount `~/.claude/` or set `CLAUDE_CONFIG_DIR`
- **Codex:** Mount `~/.codex/` or pass `OPENAI_API_KEY`
- **Gemini:** Mount `~/.config/gemini/` or pass `GOOGLE_API_KEY`

Per-agent isolation (from PR #154) already creates separate config dirs — mount those into the container.

---

## Implementation Phases

### Phase 1: Docker Detection + Clear Error UX (Current)
- [x] Check for `docker` CLI on container agent select
- [ ] Check daemon status (`docker info`)
- [ ] Platform-specific error messages
- [ ] "Install Docker" button that opens browser
- **Scope:** Frontend only, no container execution

### Phase 2: Container Lifecycle Layer (Rust backend)
- [ ] New module: `agentmuxsrv-rs/src/backend/container.rs`
- [ ] RPC: `DockerCheckCommand` — detect + health check
- [ ] RPC: `ContainerStartCommand` — start/create container
- [ ] RPC: `ContainerStopCommand` — stop container
- [ ] RPC: `ContainerStatusCommand` — is container running?
- [ ] Container naming: `agentmux-{agent_id}` (e.g., `agentmux-agent1`)

### Phase 3: Wrap SubprocessController for Container Agents
- [ ] If `agentMode === "container"`, prefix CLI command with `docker exec -i <container>`
- [ ] Auto-start container before first turn if not running
- [ ] Route stdin/stdout through `docker exec`
- [ ] Health monitor tracks container health alongside CLI health

### Phase 4: Container Image Management
- [ ] Extend ForgeAgent schema with `container_image`
- [ ] `docker pull` on first use (with progress in agent pane)
- [ ] Pre-built images on Docker Hub / GitHub Container Registry
- [ ] Forge UI: image selection field

### Phase 5: Advanced
- [ ] Devcontainer spec support (`.devcontainer/devcontainer.json`)
- [ ] Podman compatibility (`podman` as drop-in for `docker`)
- [ ] Container resource limits (CPU, memory)
- [ ] Container networking (inter-agent communication)
- [ ] Container persistence (named volumes for workspace state)

---

## Which srv routes a container agent may call

The rule (operator, 2026-10-28): a sandbox agent can do everything a native agent can do except
directly update and run files on the machine. A container's scoped token (`amxc_…`) is accepted on
every srv route except the ones in `backend/container_routes.rs`; a route added later is open to
containers unless it is added there. A refusal answers `403` with `this route is not available to
container agents: <why, and what to do instead>`.

| Route | Container | Why |
|---|---|---|
| `/api/v1/shell/*`, `/api/v1/ptyshell/*` (Terminal, Shell) | no | runs commands on the host |
| `/api/v1/conn/*`, `/api/v1/askpass` (connections: WSL, SSH as the user) | no | the same, on other machines as the user |
| `/api/v1/agent/desktop/*` (Desktop control) | no | drives the OS and can launch programs (borderline; left to the operator) |
| `/api/v1/host/*`, `/agentmux/reactive/{poller/*,unregister,ensure-signing-key,agents,audit}`, `/agentmux/agent/holding`, `/agentmux/diag/sagas`, `/api/messaging/*`, `/api/post-chat-message`, `/webhook/*` | no | the app's own plumbing: approvals, state the windows report, controls no agent tool calls and whose handlers don't check who asks |
| `/agentmux/service`, `/ws`, `/agentmux/wps/publish` | no | generic gateways to everything else |
| `/agentmux/stream-local-file`, `/agentmux/file`, `/agentmux/file-icon` | no | read host files for the app's windows; a container agent reads its own files in `/workspace` directly |
| `/api/v1/ui/browser/set_files` | no, for now | uploads from a path; needs the container's `/workspace` mapped to the host folder first |
| `/api/v1/pane/open` | no, for now | a pane shows a host file in the user's windows, which read the path they are given; a container agent reads its own files in `/workspace` directly, or asks a native agent to open one |
| `/api/v1/agent/globalmemory/{write,remove,revert}` | no | Global Memory is composed into every native agent's context at launch; a container reads it, a native agent writes it |
| `/api/v1/agent/self/upgrade` | no | replaces the app on the host |
| `/agentmux/agents/self/keys`, `/api/v1/agent/self/mcp-beat`, `/agentmux/reactive/register` | no | srv does this for a container agent itself |
| `/agentmux/jobs/stop-run`, `/agentmux/viewer/*`, `/.well-known/*` | no | between AgentMux instances |
| Everything else: Presence, Vault (with the user's approvals), Browser and UI (open, snapshot, find, act, navigate, eval, screenshot, query, click, shortcuts), Tasks, Schedule, Memory, Message, Agents, Widget (installing one from a folder in `/workspace`, which the handler maps to the host's), the GitHub token for the agent's own git, Google, RegisterDevServer, … | yes | what a native agent has |

Two things stay true for the routes that are open. A request is attributed to the agent the token
was minted for, whatever the body says, and every route opened since the denylist needs that
identity (`route_needs_identity`). And a container's default browser is its own Incognito jar,
never the user's Personal profile; it browses as a named profile only if the user switched on
"Agents may use it" for it in Settings → Browser.

## Devcontainer Consideration

The [Dev Containers spec](https://containers.dev/) is tempting but adds complexity:
- Requires `devcontainer` CLI (`npm install -g @devcontainers/cli`)
- Config format is verbose for our use case
- Designed for IDE integration, not subprocess management

**Recommendation:** Skip devcontainers for now. Use simple `docker run` + `docker exec`. Revisit if users request VS Code workspace sharing.

---

## Rust Implementation: Bollard Crate

Use the `bollard` crate (async, tokio-native) instead of shelling out to `docker` CLI:

```toml
# agentmuxsrv-rs/Cargo.toml
bollard = "0.17"
```

**Why Bollard over CLI:**
- Async/tokio-native — fits existing backend architecture
- `connect_with_defaults()` auto-detects Docker socket across platforms (named pipe on Windows, unix socket on macOS/Linux)
- Works transparently with OrbStack, Colima, Podman (if Docker-compatible socket exposed)
- Structured API responses — no stdout parsing
- Exec API for running commands inside containers

**Key Bollard operations:**
```rust
use bollard::Docker;
use bollard::container::{Config, CreateContainerOptions, StartContainerOptions};
use bollard::exec::{CreateExecOptions, StartExecResults};

// Detection + health check
let docker = Docker::connect_with_defaults()?;
let version = docker.version().await?;  // ~100ms

// Start persistent container
docker.create_container(Some(CreateContainerOptions { name: "agentmux-agent1" }), Config {
    image: Some("agentmux/agent-claude:latest"),
    cmd: Some(vec!["tail", "-f", "/dev/null"]),  // keep alive
    host_config: Some(HostConfig {
        binds: Some(vec!["~/.agentmux/agents/agent1:/workspace"]),
        ..Default::default()
    }),
    ..Default::default()
}).await?;

// Run agent turn inside container
let exec = docker.create_exec("agentmux-agent1", CreateExecOptions {
    attach_stdin: Some(true),
    attach_stdout: Some(true),
    attach_stderr: Some(true),
    cmd: Some(vec!["claude", "-p", "--output-format", "stream-json", ...]),
    ..Default::default()
}).await?;
```

---

## Files to Change

| Phase | File | Change |
|-------|------|--------|
| 1 | [`frontend/app/view/agent/agent-view.tsx`](../../frontend/app/view/agent/agent-view.tsx) | Enhanced Docker detection with daemon check |
| 2 | `agentmuxsrv-rs/src/backend/container.rs` | New module: container lifecycle |
| 2 | `agentmuxsrv-rs/src/server/websocket.rs` | New RPC handlers for container commands |
| 3 | `agentmuxsrv-rs/src/backend/blockcontroller/subprocess.rs` | Docker exec wrapper for container agents |
| 3 | `agentmuxsrv-rs/src/server/websocket.rs` | AgentInput handler: auto-start container |
| 4 | `agentmuxsrv-rs/src/backend/storage/migrations.rs` | Add container_image column |
| 4 | `agentmuxsrv-rs/forge-seed.json` | Add image references to container agents |
| 4 | `frontend/app/view/forge/forge-view.tsx` | Image selection in forge form |
