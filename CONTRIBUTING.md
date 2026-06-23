# Contributing to AgentMux

We welcome contributions to AgentMux! There are several ways to get involved:

- Report bugs or request features via [GitHub Issues](https://github.com/agentmuxai/agentmux/issues)
- Fix outstanding [issues](https://github.com/agentmuxai/agentmux/issues) in the existing code
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
- For major changes, [create an issue](https://github.com/agentmuxai/agentmux/issues/new) first to discuss the approach.
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

AgentMux is a **four-process Rust desktop application** running a bundled Chromium (CEF) host.

```
agentmux/
├── agentmux-cef/         # CEF host app (Rust + cef-rs + bundled Chromium)
├── agentmux-launcher/    # Portable launcher (325 KB)
├── agentmux-srv/         # Rust async backend server (Tokio + Axum)
├── agentmux-common/      # Shared utilities across the Rust crates
├── frontend/             # SolidJS + TypeScript UI (Vite)
├── docs/                 # Architecture docs, specs, guides
├── scripts/              # Build and version management scripts
└── Taskfile.yml          # Build task definitions
```

### Frontend (`frontend/`)

Written in **SolidJS + TypeScript**, bundled by Vite. Entry point is [`frontend/app/app.tsx`](./frontend/app/app.tsx).

When running `task dev`, the frontend loads via Vite with Hot Module Reloading — most styling and component changes reload automatically. For state-level changes (reducer stack, layout), force-reload with `Ctrl+Shift+R`.

Key subdirectories:
- `frontend/app/view/` — pane view types (agent, terminal, webview, etc.)
- `frontend/app/store/` — signals + 4-layer reducer stack state management
- `frontend/app/element/` — reusable UI components

### CEF Host (`agentmux-cef/`)

The native desktop layer — handles window management, system tray, browser pane lifecycle, and IPC between the frontend and backend. Bundles its own Chromium via CEF; no system WebView is used.

Changes here require rebuilding: `task build:host` followed by restarting `task dev`.

### Rust Backend (`agentmux-srv/`)

The async backend server — auto-spawned by the launcher, never launched manually. Handles:

- Block/pane lifecycle and controller execution
- WebSocket server for real-time frontend communication (JSON-RPC 2.0)
- SQLite persistence (blocks, tabs, windows, metadata)
- Shell execution with real PTY via `portable-pty`
- AI provider integration (Claude API, multi-provider CLI)
- Event pub-sub system
- File operations and remote connections

Changes here require `task build:backend` followed by restarting `task dev`.

### Launcher (`agentmux-launcher/`)

The portable entry-point process (~325 KB). It owns process and single-instance
lifecycle so the host and backend can stay focused on the app:

- **Single-instance enforcement** keyed on `(channel, version)` — a second launch
  of the same instance forwards an "open new window" request to the running host
  instead of starting a duplicate.
- **Job Object (Windows)** — the launcher creates the job that owns the whole
  process tree (`KILL_ON_JOB_CLOSE`), so closing the app cleanly reaps the host
  and backend with no orphans.
- **Named-pipe / socket IPC** and the **saga coordinator** for crash-safe startup
  and shutdown.
- **Splash** while the host warms up, then **spawns the host** from `runtime/`.
- Owns the **`agentmux-srv` lifecycle** (spawns and supervises the backend).

On Windows, `task dev` exercises the launcher exactly as a packaged build does
(production-parallel layout). See the isolation invariants (I1–I6) in
[`CLAUDE.md`](./CLAUDE.md) for the contract that keeps parallel instances safe.

### Communication Flow

```
Frontend (SolidJS)
    ↕  CEF IPC (window/platform commands)
agentmux-cef (CEF host)
    ↕  WebSocket / JSON-RPC 2.0
agentmux-srv (Rust backend)
```
