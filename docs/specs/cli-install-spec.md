# CLI Install Spec — Per-Version Isolated Installs

## Principle

AgentMux **never** uses a system-installed CLI. Every AgentMux release gets a fresh
CLI install into a directory stamped with the **AgentMux version**. Multiple AgentMux
versions coexist without interfering.

## Version Source

The AgentMux version is compiled into the Rust backend at build time:

```rust
const AGENTMUX_VERSION: &str = env!("CARGO_PKG_VERSION"); // e.g. "0.32.9"
```

This is the single source of truth. No version tracking files, no runtime lookups.
When the binary starts, it already knows its version.

## Directory Layout

```
~/.agentmux/<agentmux_version>/cli/<provider>/
    bin/claude[.exe]           (Claude — from official installer)
    node_modules/.bin/codex    (npm-based providers)
```

Example for AgentMux v0.32.9:

```
~/.agentmux/0.32.9/cli/claude/
    bin/claude.exe
```

When user upgrades AgentMux to v0.33.0, a **new** install happens:

```
~/.agentmux/0.33.0/cli/claude/
    bin/claude.exe
```

The old `0.32.9/` directory remains (safe to garbage-collect later).
The entire `~/.agentmux/<version>/` tree is self-contained per release.

## Resolution Algorithm (ResolveCliCommand)

The backend handler receives `provider_id`, `cli_command`, etc. from the frontend.
It appends its own compiled-in version to build the path — the frontend does NOT
need to know the version.

```
1. Build expected path:
     ~/.agentmux/<AGENTMUX_VERSION>/cli/<provider>/bin/<cmd>[.exe]

2. If binary exists at that path:
     → return { cli_path, version, source: "local_install" }

3. If binary does NOT exist:
     → create the versioned directory
     → run official installer (see below)
     → copy/move binary into versioned dir if installer uses a different target
     → verify binary exists, return { cli_path, version, source: "installed" }
     → on failure (network, etc.), return error with manual install instructions

4. NEVER fall back to system PATH. NEVER use `where`/`which`.
```

## Install Method Per Provider

| Provider | Windows | Unix |
|----------|---------|------|
| Claude | `irm https://claude.ai/install.ps1 \| iex` | `curl -fsSL https://claude.ai/install.sh \| bash` |
| Codex | `npm install @openai/codex@<version>` in versioned dir | same |
| Gemini | `npm install @google/gemini-cli@<version>` in versioned dir | same |

### Claude Install Details

The official Claude installer installs to `~/.local/bin/claude` (Unix) or similar
on Windows. After the installer runs, we **copy** the binary into our versioned
directory:

```
source: ~/.local/bin/claude[.exe]  (or wherever the installer puts it)
target: ~/.agentmux/<AGENTMUX_VERSION>/cli/claude/bin/claude[.exe]
```

Steps:
1. Create `~/.agentmux/<AGENTMUX_VERSION>/cli/claude/bin/`
2. Run official installer
3. Locate installed binary (`~/.local/bin/claude` or check common paths)
4. Copy to versioned dir
5. Return versioned path as `cli_path`

> **TODO:** Verify exact install locations of `install.ps1` / `install.sh` to
> know where to copy from. May also check if `CLAUDE_INSTALL_DIR` env var is
> respected to skip the copy step.

### npm-based Providers (Codex, Gemini)

```bash
cd ~/.agentmux/<AGENTMUX_VERSION>/cli/codex/
npm init -y && npm install @openai/codex@0.107.0
# binary at: node_modules/.bin/codex[.cmd]
```

## Changes Required

### 1. `websocket.rs` — ResolveCliCommand handler

- Add `const AGENTMUX_VERSION: &str = env!("CARGO_PKG_VERSION");`
- Remove Step 2 (PATH fallback) entirely
- Change install dir from `~/.agentmux/<version>/cli/<provider>/` to
  `~/.agentmux/<AGENTMUX_VERSION>/cli/<provider>/`
- Binary subpath: `bin/<cmd>[.exe]` for Claude, `node_modules/.bin/<cmd>[.cmd]` for npm
- For Claude: run official installer → copy binary to versioned dir
- For npm: run `npm install` inside versioned dir

### 2. `rpc_types.rs` — No changes needed
`pinned_version` stays for npm providers (specific package version to install).
The directory key is always `AGENTMUX_VERSION`, not `pinned_version`.

### 3. `providers/index.ts` — Minor
`pinnedVersion` continues to specify which npm package version to install.
For Claude, it can stay `"latest"` since the official installer handles versioning
and we just copy whatever it installs.

### 4. Frontend — No changes needed
Frontend doesn't need to know the AgentMux version. The backend handles it.

## Offline / Error Handling

- If install fails due to network: return error
  `"no internet — cannot install <cmd>. Connect and try again."`
- If install fails for other reasons: return first 500 chars of output
- Frontend displays the error in the launch flow log

## Cleanup (Future)

Optional: on startup, scan `~/.agentmux/<version>/cli/<provider>/` and delete directories
for AgentMux versions older than the current one. Not required for MVP.
