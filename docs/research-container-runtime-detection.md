# Container Runtime Detection & Management for AgentMux

Research document for adding container support to AgentMux (Tauri v2, Rust backend, React/TS frontend).

**Date:** 2026-03-18

---

## 1. Docker Desktop Detection by Platform

### Windows

| Signal | Path / Command | Notes |
|--------|---------------|-------|
| Registry key | `HKLM\SOFTWARE\Docker Inc.\Docker Desktop` | Critical for Docker Desktop to function; absence = not installed |
| Uninstall entry | `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Docker Desktop` | Standard Windows uninstall detection |
| Install directory | `C:\Program Files\Docker\Docker Desktop.exe` | Default install path |
| CLI binary | `C:\Program Files\Docker\cli-plugins\` and `docker.exe` in PATH | `where docker` from cmd or `which docker` from bash |
| Named pipe | `//./pipe/docker_engine` | Bollard connects here by default on Windows |
| Auto-start | `HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Run` | Presence = Docker Desktop set to auto-start |

**Detection strategy (Rust):**
```rust
// 1. Check PATH
std::process::Command::new("docker").arg("--version").output()
// 2. Check registry (winreg crate)
RegKey::predef(HKEY_LOCAL_MACHINE)
    .open_subkey("SOFTWARE\\Docker Inc.\\Docker Desktop")
// 3. Check named pipe exists
std::path::Path::new(r"\\.\pipe\docker_engine").exists()
```

### macOS

| Signal | Path | Notes |
|--------|------|-------|
| Docker Desktop app | `/Applications/Docker.app` | Standard install location |
| CLI symlink | `/usr/local/bin/docker` | Symlinked from `Docker.app/Contents/Resources/bin/` |
| Docker socket | `/var/run/docker.sock` | Symlink managed by Docker Desktop (or OrbStack/Colima) |
| Docker context | `~/.docker/config.json` | Contains `currentContext` indicating active engine |

**Detection strategy:**
```rust
// 1. which docker
// 2. Path::new("/Applications/Docker.app").exists()
// 3. Path::new("/var/run/docker.sock").exists()
// 4. docker context inspect → tells you WHO provides the socket
```

### Linux

| Signal | Path / Command | Notes |
|--------|---------------|-------|
| CLI binary | `which docker` | Installed via apt/dnf/pacman |
| Daemon socket | `/var/run/docker.sock` | Owned by `root:docker` |
| Systemd service | `systemctl is-active docker` | Returns "active" or "inactive" |
| User in docker group | `id -nG $USER \| grep docker` | If not, commands fail with permission denied |

**Detection strategy:**
```rust
// 1. which docker
// 2. Path::new("/var/run/docker.sock").exists()
// 3. Command::new("systemctl").args(["is-active", "docker"]).output()
// 4. Try docker info — catches both "not installed" and "permission denied"
```

---

## 2. Docker Alternatives

### Podman

| Aspect | Details |
|--------|---------|
| Detection | `which podman`, `podman --version` |
| CLI compatibility | ~95% drop-in replacement for `docker` CLI. Same subcommands (`run`, `build`, `ps`, `exec`). |
| Architecture | **Daemonless** — each container is a child process of the CLI, no background daemon |
| Socket | `podman system service` exposes a Docker-compatible API on a Unix socket |
| Gotchas | `podman ps` only shows current user's containers (rootless). Some Docker Compose features may not work identically. No Docker BuildKit by default. |
| Podman Desktop | GUI app analogous to Docker Desktop; available on all platforms |
| Windows | Podman Desktop installs `podman.exe` and manages a Podman machine (WSL2-based or HyperV) |
| macOS | `brew install podman` + `podman machine init && podman machine start` |

**Recommendation:** Support Podman as a first-class alternative. Detect via `which podman`. Use the same container commands — just swap the binary name.

### Rancher Desktop

| Aspect | Details |
|--------|---------|
| Engine choice | User picks **dockerd (Moby)** or **containerd** at setup time |
| If dockerd | `docker` CLI works normally, `/var/run/docker.sock` available |
| If containerd | Only `nerdctl` works; `docker` CLI will fail |
| Detection | Check for `docker` and `nerdctl` in PATH. `nerdctl` presence + no `docker` = likely Rancher Desktop with containerd |
| PATH setup | Automatic on all platforms during install |

**Recommendation:** If `docker` is in PATH and works, treat it the same regardless of whether Rancher Desktop or Docker Desktop provides it. Only special-case `nerdctl` if we want containerd support (not recommended initially).

### Colima (macOS/Linux only)

| Aspect | Details |
|--------|---------|
| What it is | Lightweight VM manager that runs Docker/containerd in a Lima VM |
| Docker CLI | Uses Docker CLI from Homebrew (`brew install docker`) — Colima just provides the daemon |
| Socket | `~/.colima/default/docker.sock` — may also be symlinked to `/var/run/docker.sock` |
| Detection | `colima status` returns running/stopped. Socket path differs from Docker Desktop. |
| DOCKER_HOST | Users often set `export DOCKER_HOST=unix://$HOME/.colima/default/docker.sock` |

**Recommendation:** Respect `DOCKER_HOST` env var. Bollard's `connect_with_defaults()` already does this.

### OrbStack (macOS only)

| Aspect | Details |
|--------|---------|
| What it is | Fast, lightweight Docker Desktop replacement for macOS |
| CLI tools | Installs `docker`, `docker compose`, `docker buildx` automatically to `/usr/local/bin/` and `~/.orbstack/bin/` |
| Socket | Automatically updates `/var/run/docker.sock` symlink to point to OrbStack's engine |
| Detection | `/Applications/OrbStack.app` exists, or `orb version` / `orbctl` commands available |
| Compatibility | Runs an unmodified Docker engine — excellent compatibility |
| Data location | `~/Library/Group Containers/HUAQ24HBR6.dev.orbstack/data` |

**Recommendation:** OrbStack is transparent — `docker` CLI just works. No special handling needed.

### WSL2 with Docker (Windows)

| Aspect | Details |
|--------|---------|
| How it works | Docker Desktop runs its daemon inside a dedicated `docker-desktop` WSL distro |
| Host access | `docker.exe` on Windows PATH communicates with the WSL2 daemon via named pipe |
| WSL integration | Enabling WSL integration for a distro (e.g., Ubuntu) makes `docker` CLI available inside that distro |
| Without Docker Desktop | Can install Docker Engine directly inside a WSL2 distro — but `docker` won't be on Windows PATH |
| Performance | Linux containers run natively (no emulation). Dynamic memory allocation. |

**Recommendation:** From the Windows host, always use `docker.exe` in PATH. Don't try to reach into WSL directly.

---

## 3. Docker Daemon Health Check

### Recommended Check Sequence

```
Step 1: which docker / where docker
  FAIL → Docker not installed

Step 2: docker version --format '{{.Server.Version}}'
  FAIL "Cannot connect to the Docker daemon" → Daemon not running
  FAIL "permission denied" → User not in docker group (Linux)
  SUCCESS → Daemon running, get version

Step 3: docker info --format '{{.OSType}}'
  Returns "linux" or "windows" → Confirms daemon type
  Also reveals: storage driver, total memory, OS, architecture
```

### Why `docker version` over `docker info` or `docker ps`

| Command | What it tells you | Speed | Side effects |
|---------|-------------------|-------|-------------|
| `docker version` | Client + server version. Server section fails if daemon is down. | Fast (~100ms) | None |
| `docker info` | Detailed system info (storage driver, plugins, memory, etc.) | Slower (~500ms) | None |
| `docker ps` | Lists running containers | Fast | None, but gives less diagnostic info on failure |

**Best practice:** Use `docker version --format json` for the health check. It's fast, returns structured data, and distinguishes between "not installed," "daemon not running," and "permission denied."

### Common Failure Modes

| Failure | Symptom | Resolution hint for user |
|---------|---------|------------------------|
| Docker not installed | `docker` command not found | Show install prompt |
| Daemon not running | "Cannot connect to the Docker daemon at unix:///var/run/docker.sock" | "Start Docker Desktop" or `sudo systemctl start docker` |
| Permission denied (Linux) | "Got permission denied while trying to connect to the Docker daemon socket" | `sudo usermod -aG docker $USER` then log out/in |
| WSL integration not enabled | Docker works from PowerShell but not from WSL distro | Docker Desktop Settings > Resources > WSL Integration |
| Hyper-V not enabled (Windows) | Docker Desktop fails to start | Enable Hyper-V in Windows Features or switch to WSL2 backend |
| Docker Desktop license expired | GUI shows license warning, CLI may still work | Inform user; or suggest Podman/Colima |

---

## 4. Auto-Install Options

### Docker Desktop

| Platform | Method | Silent? | Permissions needed |
|----------|--------|---------|-------------------|
| Windows | `winget install Docker.DockerDesktop` | Partially (verbose output) | Admin (installs Windows service, Hyper-V/WSL components) |
| Windows | `DockerDesktopInstaller.exe install --quiet --accept-license --backend=wsl-2` | Yes | Admin |
| Windows | MSI installer (Enterprise) | Yes, standard MSI silent flags | Admin |
| macOS | `brew install --cask docker` | Yes | User (may prompt for password) |
| macOS | DMG download + `hdiutil` + `cp` | Scriptable | User |
| Linux | `curl -fsSL https://get.docker.com \| sh` | Yes | Root/sudo |
| Linux | `apt install docker.io` / `dnf install docker-ce` | Yes | Root/sudo |

### Lightweight Alternatives (no Docker Desktop)

| Option | Platform | Install command | What you get |
|--------|----------|----------------|-------------|
| Colima + Docker CLI | macOS/Linux | `brew install colima docker` | Docker daemon in a Lima VM + CLI. No GUI. Free. |
| Podman | All | `brew install podman` / `winget install RedHat.Podman` / `apt install podman` | Daemonless container runtime. Free. |
| OrbStack | macOS | `brew install orbstack` | Full Docker-compatible runtime. Free for personal use. |
| Docker Engine (no Desktop) | Linux | `apt install docker-ce docker-ce-cli containerd.io` | Just the engine + CLI. No GUI. Free. |

### Recommendation for AgentMux

**Do NOT auto-install Docker.** Instead:

1. **Detect** what's available (see section 3).
2. **Guide** the user with a clear UI showing what's missing and platform-appropriate install instructions.
3. **Offer one-click install** only via system package managers (`winget`, `brew`) — these handle permissions and dependencies properly.
4. For a lightweight path, recommend **Colima** (macOS), **Podman** (all platforms), or **Docker Engine** (Linux).

---

## 5. Container Workflow for AI Agent CLIs

### Architecture Decision: Persistent Container

Use a **persistent container** that stays alive between agent turns, not ephemeral containers per invocation.

**Rationale:**
- Agent CLIs (Claude Code, Codex, Gemini) maintain session state, conversation history, and tool caches
- Container startup time (~1-3s) would add latency to every turn with ephemeral approach
- Auth tokens / login state is stored in the filesystem (e.g., `~/.claude/`)
- npm global installs, pip packages, etc. persist across turns

### Recommended Container Lifecycle

```
1. User opens agent pane → AgentMux checks for existing container
2. If no container: create + start (once)
3. Agent turns: docker exec -i <container> claude -p --verbose ...
4. User closes pane: container stays running (for quick re-open)
5. User explicitly stops: docker stop <container>
6. Cleanup: docker rm on workspace close or manual action
```

### Base Image

**Recommended: `node:22-slim` (Debian Bookworm-based)**

```dockerfile
FROM node:22-slim

RUN apt-get update && apt-get install -y \
    git curl jq build-essential python3 \
    && rm -rf /var/lib/apt/lists/*

# Install AI CLIs
RUN npm install -g @anthropic-ai/claude-code@latest
# Add others as needed

# Non-root user
RUN useradd -m -s /bin/bash agent
USER agent
WORKDIR /workspace
```

Why `node:22-slim` over alternatives:
- Claude Code and Gemini CLI require Node.js
- `-slim` variant is ~200MB (vs ~1GB for full `node:22`)
- Debian base has good package availability
- `ubuntu` would require separate Node.js install

### Volume Mounts

```bash
docker run -d \
  --name agentmux-agent-<id> \
  # Mount the user's project directory
  -v "/path/to/project:/workspace" \
  # Mount agent auth/config (read-only where possible)
  -v "$HOME/.claude:/home/agent/.claude" \
  # Mount git config for commits
  -v "$HOME/.gitconfig:/home/agent/.gitconfig:ro" \
  # Mount SSH keys for git operations (read-only)
  -v "$HOME/.ssh:/home/agent/.ssh:ro" \
  agentmux-agent-image
```

### Passing Auth Tokens

**Method 1: Environment variables (preferred for API keys)**
```bash
docker run -e ANTHROPIC_API_KEY=$ANTHROPIC_API_KEY \
           -e OPENAI_API_KEY=$OPENAI_API_KEY \
           -e GEMINI_API_KEY=$GEMINI_API_KEY ...
```

**Method 2: Bind-mount credential directories (for OAuth-based tools)**
```bash
-v "$HOME/.claude:/home/agent/.claude"       # Claude Code OAuth state
-v "$HOME/.codex:/home/agent/.codex"         # Codex config
-v "$HOME/.config/gemini:/home/agent/.config/gemini"  # Gemini
```

**Security rules:**
- NEVER mount `/var/run/docker.sock` into agent containers (container escape risk)
- NEVER mount entire `$HOME` (exposes SSH keys, AWS creds, etc.)
- Use `:ro` (read-only) for credentials the agent should not modify
- API keys via `-e` are preferred over mounted files — they don't persist in the container filesystem

### Network Access

Agent CLIs need outbound HTTPS to reach their APIs. The default Docker network mode (`bridge`) allows this with no extra config.

```bash
# Default — outbound works, no inbound ports exposed
docker run --network bridge ...

# If agent needs to run a local dev server visible to host:
docker run -p 3000:3000 -p 8080:8080 ...
```

No special network setup is required for API-calling agents.

### Executing Agent Commands

From the Rust backend, use Bollard to run commands inside the persistent container:

```rust
use bollard::Docker;
use bollard::exec::{CreateExecOptions, StartExecOptions};

let docker = Docker::connect_with_defaults()?;

// Create exec instance
let exec = docker.create_exec("agentmux-agent-1", CreateExecOptions {
    cmd: Some(vec![
        "claude", "-p", "--verbose",
        "--output-format", "stream-json",
        "--include-partial-messages",
    ]),
    attach_stdout: Some(true),
    attach_stderr: Some(true),
    attach_stdin: Some(true),
    tty: Some(false),
    env: Some(vec!["ANTHROPIC_API_KEY=sk-ant-..."]),
    working_dir: Some("/workspace"),
    ..Default::default()
}).await?;

// Start exec and stream output
let output = docker.start_exec(&exec.id, None).await?;
// output is a Stream of LogOutput — pipe to frontend
```

---

## 6. Dev Containers / devcontainer.json

### Could AgentMux Leverage the Devcontainer Spec?

**Yes, but as an optional advanced feature — not the primary path.**

### How It Would Work

```
.devcontainer/
  devcontainer.json    ← User provides this (or AgentMux generates a default)
  Dockerfile           ← Optional custom image
```

```json
{
  "name": "agentmux-agent",
  "image": "node:22-slim",
  "features": {
    "ghcr.io/devcontainers/features/git:1": {},
    "ghcr.io/devcontainers/features/python:1": {}
  },
  "mounts": [
    "source=${localEnv:HOME}/.claude,target=/home/vscode/.claude,type=bind"
  ],
  "postCreateCommand": "npm install -g @anthropic-ai/claude-code@latest",
  "remoteUser": "vscode"
}
```

AgentMux would use the `@devcontainers/cli` (Node.js package) or invoke it as a subprocess:

```bash
devcontainer up --workspace-folder /path/to/project
devcontainer exec --workspace-folder /path/to/project -- claude -p ...
```

### Benefits

| Benefit | Details |
|---------|---------|
| Standardized config | Users who already have `.devcontainer/` get it for free |
| VSCode compatibility | Same container works in VSCode, Codespaces, and AgentMux |
| Feature system | Install languages/tools declaratively via devcontainer features |
| Lifecycle hooks | `postCreateCommand`, `postStartCommand` for setup |
| Multi-container | `docker-compose.yml` support for complex setups |

### Drawbacks

| Drawback | Details |
|----------|---------|
| Extra dependency | `@devcontainers/cli` is a ~50MB Node.js package that must be installed |
| Startup overhead | `devcontainer up` is slower than raw `docker run` (~5-15s vs ~1-3s) |
| Complexity | Adds another abstraction layer; harder to debug when things go wrong |
| Overkill for simple case | Most users just need "run claude in a container with my project mounted" |

### Recommendation

**Phase 1:** Use raw Docker/Bollard API directly. Build a simple, opinionated container setup that works out of the box.

**Phase 2:** Add devcontainer support as an opt-in feature. If a `.devcontainer/devcontainer.json` exists in the workspace, offer to use it. This gives power users full control while keeping the default path simple.

---

## 7. Recommended Implementation Plan

### Rust Crate: `bollard`

[bollard](https://github.com/fussybeaver/bollard) is the mature Rust crate for Docker API interaction.

**Key features for AgentMux:**
- Async (tokio-based) — fits Tauri's async command model
- Cross-platform: Unix socket, Windows named pipe, HTTP, SSH
- `connect_with_defaults()` respects `DOCKER_HOST` env var — automatically works with Docker Desktop, Colima, OrbStack, Podman's Docker-compatible socket
- Full container lifecycle: create, start, exec, stop, remove
- Streaming: attach to container stdout/stderr as async streams

### Detection Flow (implement in Rust backend)

```
detect_container_runtime() -> ContainerRuntimeStatus {
    // 1. Try bollard connect_with_defaults()
    //    - This checks DOCKER_HOST, then platform default socket/pipe
    //    SUCCESS → check docker.version() for server info
    //    FAIL → try alternate sockets

    // 2. If bollard fails, check for CLI binaries:
    //    which docker → DockerCLI found
    //    which podman → PodmanCLI found
    //    which nerdctl → NerdctlCLI found
    //    None → NoRuntime

    // 3. If CLI found but daemon unreachable:
    //    Return DaemonNotRunning { runtime, suggestion }

    // 4. If connected:
    //    Return Connected { runtime, version, os_type }
}
```

### Container Runtime Status (frontend model)

```typescript
type ContainerRuntimeStatus =
  | { state: "not-installed" }
  | { state: "installed-not-running"; runtime: string; suggestion: string }
  | { state: "permission-denied"; suggestion: string }
  | { state: "running"; runtime: string; version: string }
```

### Image Management

```
1. On first use: docker pull agentmux/agent-base:latest (or build from bundled Dockerfile)
2. Cache image locally — check with docker images before pulling
3. Version tag images: agentmux/agent-base:0.32.0
4. Allow user to specify custom image in settings
```

---

## Sources

- [Docker Desktop WSL 2 backend on Windows](https://docs.docker.com/desktop/features/wsl/)
- [Docker Desktop silent install guide](https://silentinstallhq.com/docker-desktop-silent-install-how-to-guide/)
- [Install Docker Desktop on Windows](https://docs.docker.com/desktop/setup/install/windows-install/)
- [Troubleshooting the Docker daemon](https://docs.docker.com/engine/daemon/troubleshoot/)
- [Linux post-installation steps for Docker Engine](https://docs.docker.com/engine/install/linux-postinstall/)
- [Colima installation](https://colima.run/docs/installation/)
- [OrbStack install docs](https://docs.orbstack.dev/install)
- [OrbStack Docker containers](https://docs.orbstack.dev/docker/)
- [Rancher Desktop - Working with Containers](https://docs.rancherdesktop.io/tutorials/working-with-containers/)
- [Podman vs Docker 2026 comparison](https://www.xurrent.com/blog/podman-vs-docker-complete-2025-comparison-guide-for-devops-teams)
- [bollard - Rust Docker API crate](https://docs.rs/bollard)
- [bollard Docker struct](https://docs.rs/bollard/latest/bollard/struct.Docker.html)
- [Docker Sandboxes for Claude Code](https://www.docker.com/blog/docker-sandboxes-run-claude-code-and-other-coding-agents-unsupervised-but-safely/)
- [Claude Code sandbox - Docker Docs](https://docs.docker.com/ai/sandboxes/agents/claude-code/)
- [Claude Code sandboxing docs](https://code.claude.com/docs/en/sandboxing)
- [Running AI Agents in Devcontainers](https://markphelps.me/posts/running-ai-agents-in-devcontainers/)
- [Docker/Podman for AI CLI tools](https://www.bitdoze.com/docker-podman-ai-cli-tools-safe-environment/)
- [Dev Container CLI](https://github.com/devcontainers/cli)
- [Dev Container specification](https://containers.dev/)
- [nerdctl - Docker-compatible CLI for containerd](https://github.com/containerd/nerdctl)
- [Containers in 2025: Docker vs Podman](https://www.linuxjournal.com/content/containers-2025-docker-vs-podman-modern-developers)
