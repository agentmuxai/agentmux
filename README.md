<p align="center">
  <img src="./assets/agentmux-logo.svg" alt="AgentMux Logo" width="120">
</p>

# AgentMux

**Watch Your Agents. Stay in Control.**

A rich monitoring and orchestration UI for AI agents. See every tool call, catch regressions mid-task, and tune your agent system in real time.

[![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)
[![Website](https://img.shields.io/badge/Website-agentmux.ai-blue)](https://agentmux.ai)

> Developed by **[AgentMux Corp.](./LEGAL.md)** - Delaware corporation, California registered.

## The Problem

Knowledge workers running AI agents across long-horizon tasks are blind while it happens. You can't see which agent found something important. You can't see which one went off-track. You can't redirect mid-task. You find out when it's done, or when something is wrong.

- **Agents regress.** An agent fixes a bug and then undoes its own work in a later step. By the time you notice, the context is cold and the decision chain is opaque.
- **Guardrails are tuned blind.** No live signal on which constraints are firing, which are too tight, which agents are working around.
- **Multi-agent conflicts are invisible.** Two agents reach conflicting conclusions. The synthesis picks one. You never know the conflict happened.

## What AgentMux Does

AgentMux is an open-source desktop application that surfaces what agents are doing in real time: tool calls, reasoning steps, source citations, output streams, and conflicts between agents. The human role is observer and supervisor, not driver.

Cross-platform (Windows, macOS, Linux). 100% Rust backend (Tokio + Axum). Tauri v2. Apache 2.0.

- **Live agent monitoring** - Watch every tool call and decision step as it happens. Catch an agent undoing correct work mid-task and redirect it before the damage compounds.
- **Multi-agent orchestration** - Run parallel agents and see all of them at once. Spot conflicts before synthesis. Redirect any agent without killing the others.
- **Guardrail observability** - See which constraints are active and firing. Tune your agent system from live signal, not post-mortem guesswork.
- **Built-in Claude integration** - Agent sessions are first-class citizens alongside terminals, editor, and system metrics.
- **Multiple pane types** - Terminal, AI Agent, Code Editor, System Info, Web, and more
- **Real PTY support** - Authentic terminal emulation via xterm.js and portable-pty
- **Shell integration** - `wsh` binary deployable to remote hosts for multiplexed sessions

## Quick Start

```bash
# Install dependencies
npm install

# Development mode (hot reload)
task dev

# Production build
task package
```

## Pane Types

| View | Description |
|------|-------------|
| `term` | Terminal with xterm.js and real PTY |
| `agent` | Claude AI agent (multi-provider CLI support) |
| `codeeditor` | Monaco-based code editor |
| `sysinfo` | System metrics (CPU, memory, network) |
| `webview` | Embedded web browser |
| `chat` | Multi-user chat widget |
| `tsunami` | Network protocol visualization |
| `vdom` | Virtual DOM component renderer |
| `help` | Built-in documentation viewer |
| `launcher` | Application launcher |

## Architecture

```
AgentMux.exe  (Tauri v2 - Rust + WebView2)
    +-- agentmuxsrv-rs  (Rust async backend - Tokio + Axum, auto-spawned)
        +-- wsh-rs      (Rust shell integration binary, deployed to remotes)
```

**Stack:**
- **Frontend:** React 19 + TypeScript + Vite + Jotai
- **Backend:** Rust (Tokio + Axum + SQLite + portable-pty)
- **Desktop:** Tauri v2
- **Terminal:** xterm.js + Monaco Editor

## Build Commands

AgentMux uses [Task](https://taskfile.dev/) for build orchestration.

| Command | Description |
|---------|-------------|
| `task dev` | Start development mode with hot reload |
| `task package` | Build production installer (NSIS) |
| `task package:portable` | Build installer + portable ZIP |
| `task build:backend` | Build Rust binaries (agentmuxsrv-rs + wsh-rs) |
| `task build:frontend` | Build frontend only |
| `task test` | Run all tests |
| `task clean` | Clean build artifacts |

### npm Aliases

```bash
npm run dev           # task dev
npm run package       # task package
npm run build:backend # task build:backend
npm test              # vitest
```

### Build Outputs

- **Installer:** `src-tauri/target/release/bundle/nsis/AgentMux_*.exe`
- **Portable:** `dist/agentmux-*-portable.zip`
- **Standalone:** `src-tauri/target/release/agentmux.exe`

## Prerequisites

| Tool | Version | Purpose |
|------|---------|---------|
| **Node.js** | 22 LTS | Frontend build |
| **Rust** | 1.77+ | Backend + Tauri |
| **Task** | Latest | Build orchestration |

**Windows-specific:**
- WebView2 (pre-installed on Windows 10/11)
- Visual Studio Build Tools (required by Rust)

> No Go or Zig required - the backend is 100% Rust since v0.31.0.

## Development

```bash
# Hot reload - frontend auto-reloads, Tauri auto-rebuilds on Rust changes
task dev

# After modifying Rust backend code
task build:backend
# Then restart: task dev

# Run tests
npm test

# Run with coverage
npm run coverage
```

## Version Management

Always use `bump-version.sh` - never edit version numbers manually:

```bash
./bump-version.sh patch --message "Description"
bash scripts/verify-version.sh   # verify consistency
```

See [BUILD.md](./BUILD.md) for the full version management workflow.

## License

Apache-2.0 - Originally forked from [Wave Terminal](https://github.com/wavetermdev/waveterm)
