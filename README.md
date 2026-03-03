<p align="center">
  <img src="./assets/agentmux-logo.svg" alt="AgentMux Logo" width="120">
</p>

# AgentMux

**AI-Native Terminal Multiplexer** — a fork of [Wave Terminal](https://github.com/wavetermdev/waveterm) rebuilt on a 100% Rust backend.

[![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)

> Developed by **[AgentMux Corp.](./LEGAL.md)** — Delaware corporation, California registered.

## What is AgentMux?

AgentMux is a desktop terminal multiplexer with native AI agent integration. It runs as a Tauri v2 application with a fully async Rust backend, giving you:

- **Multiple pane types** — Terminal, AI Agent, Code Editor, System Info, Web, and more
- **AI Agent pane** — Connect to Claude (or other CLI providers) directly inside a pane
- **Real PTY support** — Authentic terminal emulation via xterm.js and portable-pty
- **Shell integration** — `wsh` binary deployable to remote hosts for multiplexed sessions
- **Lightweight** — 100% Rust backend: ~5.5 MB total vs ~25 MB with the old Go backend

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
npm run dev           # → task dev
npm run package       # → task package
npm run build:backend # → task build:backend
npm test              # → vitest
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

> No Go or Zig required — the backend is 100% Rust since v0.31.0.

## Architecture

```
AgentMux.exe  (Tauri v2 — Rust + WebView2)
    └── agentmuxsrv-rs  (Rust async backend — Tokio + Axum, auto-spawned)
        └── wsh-rs      (Rust shell integration binary, deployed to remotes)
```

**Stack:**
- **Frontend:** React 19 + TypeScript + Vite + Jotai
- **Backend:** Rust (Tokio + Axum + SQLite + portable-pty)
- **Desktop:** Tauri v2
- **Terminal:** xterm.js + Monaco Editor

## Development

```bash
# Hot reload — frontend auto-reloads, Tauri auto-rebuilds on Rust changes
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

Always use `bump-version.sh` — never edit version numbers manually:

```bash
./bump-version.sh patch --message "Description"
bash scripts/verify-version.sh   # verify consistency
```

See [BUILD.md](./BUILD.md) for the full version management workflow.

## License

Apache-2.0 — Originally forked from [Wave Terminal](https://github.com/wavetermdev/waveterm)
