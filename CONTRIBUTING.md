# Contributing to AgentMux

We welcome contributions to AgentMux! There are several ways to get involved:

- Report bugs or request features via [GitHub Issues](https://github.com/agentmuxhq/agentmux/issues)
- Fix outstanding [issues](https://github.com/agentmuxhq/agentmux/issues) in the existing code
- Improve [documentation](./docs)
- Star the repository to show your appreciation

Please be mindful and respect our [code of conduct](./CODE_OF_CONDUCT.md).

## Before You Start

We accept patches as GitHub pull requests. If you're new to GitHub PRs, see the [GitHub pull request guide](https://docs.github.com/en/pull-requests/collaborating-with-pull-requests/proposing-changes-to-your-work-with-pull-requests/about-pull-requests).

### Contributor License Agreement

Contributions must be accompanied by a Contributor License Agreement (CLA). You (or your employer) retain the copyright to your contribution — this simply gives us permission to use and redistribute it as part of the project.

> On submission of your first pull request you will be prompted to sign the CLA.

### Style Guide

- The project uses American English.
- We use [Prettier](https://prettier.io) and [EditorConfig](https://editorconfig.org) for formatting — please use the recommended VS Code extensions.

## How to Contribute

- For minor changes, open a pull request directly.
- For major changes, [create an issue](https://github.com/agentmuxhq/agentmux/issues/new) first to discuss the approach.
- Branch naming: `agenta/feature-name` (e.g., `agenta/fix-terminal-scroll`)

### Development Environment

To build and run AgentMux locally, see [BUILD.md](./BUILD.md).

### UI Component Library

We use [Storybook](https://storybook.js.org/docs) to document and test UI components in isolation. Run it with:

```bash
task storybook
```

### Create a Pull Request

Guidelines:

- Check existing PRs and issues before starting — avoid duplicating work.
- Develop features on a branch — do not work directly on `main`.
- For anything but minor fixes, include tests and documentation updates.
- Reference the relevant issue in the PR body.

## Project Structure

AgentMux is a **Tauri v2** desktop application with a **100% Rust backend**.

```
agentmux/
├── src-tauri/          # Tauri v2 shell (Rust + WebView2)
├── agentmuxsrv-rs/     # Rust async backend server (Tokio + Axum)
├── wsh-rs/             # Rust shell integration binary
├── frontend/           # React 19 + TypeScript UI (Vite)
├── docs/               # Architecture docs, specs, guides
├── schema/             # JSON schema definitions
├── scripts/            # Build and version management scripts
└── Taskfile.yml        # Build task definitions
```

### Frontend (`frontend/`)

Written in React 19 + TypeScript, bundled by Vite. Entry point is [`frontend/wave.ts`](./frontend/wave.ts), React root is [`frontend/app/app.tsx`](./frontend/app/app.tsx).

When running `task dev`, the frontend loads via Vite with Hot Module Reloading — most styling and component changes reload automatically. For state-level changes (Jotai atoms, layout), force-reload with `Ctrl+Shift+R`.

Key subdirectories:
- `frontend/app/view/` — 10 pane view types (agent, term, codeeditor, sysinfo, webview, etc.)
- `frontend/app/block/` — Block/pane rendering and registry
- `frontend/app/store/` — Jotai atom state management
- `frontend/app/element/` — 40+ reusable UI components
- `frontend/app/aipanel/` — AI panel chat interface

Each view type implements the `ViewModel` interface and is registered in the block registry:

```typescript
// frontend/app/block/block.tsx
BlockRegistry.set("myview", MyViewModel);
```

### Tauri Shell (`src-tauri/`)

The native desktop layer — handles window management, system tray, native menus, file dialogs, and spawning the backend sidecar. Uses Tauri IPC commands (defined in `src-tauri/src/commands/`) to communicate with the frontend.

Changes here do not hot-reload — Tauri auto-rebuilds in `task dev` when Rust files change, but the process restarts.

### Rust Backend (`agentmuxsrv-rs/`)

The async backend server — auto-spawned by Tauri, never launched manually. Handles:

- Block/pane lifecycle and controller execution
- WebSocket server for real-time frontend communication (JSON-RPC 2.0)
- SQLite persistence (blocks, tabs, windows, metadata)
- Shell execution with real PTY via `portable-pty`
- AI provider integration (Claude API, multi-provider CLI)
- Event pub-sub system
- File operations and remote connections

Changes here require `task build:backend` followed by restarting `task dev`.

### Shell Helper (`wsh-rs/`)

A small Rust binary (1.1 MB) deployed to remote hosts for multiplexed terminal sessions and file streaming. Communicates with the backend via WebSocket.

Changes here require `task build:wsh`.

### Communication Flow

```
Frontend (React)
    ↕  Tauri IPC (window/platform commands)
Tauri Shell (src-tauri)
    ↕  WebSocket / JSON-RPC 2.0
agentmuxsrv-rs (Rust backend)
    ↕  WebSocket / wshrpc
wsh-rs (remote hosts)
```
