# Spec: Agent Pane CLI Isolation (Raw Mode)

**Status:** Ready for implementation
**Date:** 2026-03-08
**Supersedes:** `specs/cli-isolation.md`, `docs/specs/version-isolated-cli-installs.md`

## Problem

When a user clicks a provider button in raw mode, `connectWithProvider()` switches to a terminal and injects the bare CLI command (e.g. `claude\r`). This has three problems:

1. **No isolation** — uses whatever version is on the system PATH, which may be incompatible
2. **No auto-install** — if the CLI isn't installed, the shell prints `command not found` and the user is stuck
3. **Silent Tauri installer is disconnected** — `cli_installer.rs` exists with detect/install logic, but it runs silently via `CREATE_NO_WINDOW` and is never called from the agent pane flow

The previous specs (`cli-isolation.md`, `version-isolated-cli-installs.md`) proposed solutions but were never implemented. This spec consolidates them into a single actionable design.

## Goal

Every provider button click in raw mode results in:

1. **Isolation from system PATH** — AgentMux uses its own installed copy, never the user's global install
2. **Auto-install if missing** — the pinned version is installed automatically on first click
3. **Visible in terminal** — install progress, errors, and output are all shown in the raw terminal pane
4. **Per-backend isolation** — each backend (local, remote SSH) gets its own CLI directory

## Architecture

### Directory Layout

Each AgentMux version gets its own CLI directory, nested under the existing instance directory structure:

```
~/.agentmux/
  instances/
    v0.31.84/
      cli/
        claude/
          node_modules/
            @anthropic-ai/claude-code/
            .bin/
              claude        (Unix)
              claude.cmd    (Windows)
        codex/
          node_modules/
            @openai/codex/
            .bin/
              codex
              codex.cmd
        gemini/
          node_modules/
            @google/gemini-cli/
            .bin/
              gemini
              gemini.cmd
```

**Why version-isolated?** When AgentMux updates pinned versions, the new version gets a clean install. The old version's CLIs remain untouched (still usable if the user rolls back). Old version directories are cleaned up on startup (see "Cleanup" section).

### Provider Definitions (no changes needed)

`frontend/app/view/agent/providers/index.ts` already has all required fields:

| Provider | npmPackage | pinnedVersion | cliCommand |
|----------|-----------|---------------|------------|
| claude | `@anthropic-ai/claude-code` | `latest` | `claude` |
| codex | `@openai/codex` | `0.107.0` | `codex` |
| gemini | `@google/gemini-cli` | `0.31.0` | `gemini` |

## Bootstrap Script Injection

Instead of injecting a bare command name, `connectWithProvider()` injects a **bootstrap one-liner** that handles detection, install, and launch — all visible in the terminal.

### Flow

```
User clicks "Claude Code" (raw mode)
  │
  ▼
agent-model.ts: connectWithProvider()
  │
  ├── 1. Set meta: view=term, controller=shell
  ├── 2. ControllerResyncCommand (start shell)
  └── 3. After 500ms, inject bootstrap script via ControllerInputCommand
        │
        ▼
  Terminal shows:
    $ CLI_DIR=~/.agentmux/instances/v0.31.84/cli/claude
    $ [ -x "$CLI_DIR/node_modules/.bin/claude" ] || npm install --prefix "$CLI_DIR" @anthropic-ai/claude-code@latest
    added 42 packages in 8s
    $ exec "$CLI_DIR/node_modules/.bin/claude"
    ╭──────────────────────────────────╮
    │ Welcome to Claude Code!          │
    ╰──────────────────────────────────╯
```

On subsequent clicks (already installed for this version):

```
  Terminal shows:
    $ CLI_DIR=~/.agentmux/instances/v0.31.84/cli/claude
    $ [ -x "$CLI_DIR/node_modules/.bin/claude" ] (exists, skip install)
    $ exec "$CLI_DIR/node_modules/.bin/claude"
    ╭──────────────────────────────────╮
    │ Welcome to Claude Code!          │
    ╰──────────────────────────────────╯
```

### Bootstrap Scripts

The frontend detects the platform and generates the appropriate one-liner.

**Bash (Unix / WSL):**

```bash
CLI_DIR="$HOME/.agentmux/instances/v<VERSION>/cli/<PROVIDER>" && CLI_BIN="$CLI_DIR/node_modules/.bin/<BINARY>" && { [ -x "$CLI_BIN" ] || { echo "Installing <DISPLAY_NAME>..."; npm install --prefix "$CLI_DIR" <NPM_PACKAGE>@<PINNED_VERSION> --no-fund --no-audit; }; } && <ENV_PREFIX>"$CLI_BIN" <ARGS>
```

**PowerShell (Windows — default shell):**

```powershell
$d="$HOME/.agentmux/instances/v<VERSION>/cli/<PROVIDER>"; $b="$d/node_modules/.bin/<BINARY>.cmd"; if(!(Test-Path $b)){Write-Host "Installing <DISPLAY_NAME>..."; npm install --prefix $d <NPM_PACKAGE>@<PINNED_VERSION> --no-fund --no-audit}; <ENV_PREFIX>& $b <ARGS>
```

Variables:
- `<VERSION>` — from `getApi().getAboutModalDetails().version`
- `<PROVIDER>` — provider id (`claude`, `codex`, `gemini`)
- `<BINARY>` — provider cliCommand (`claude`, `codex`, `gemini`)
- `<NPM_PACKAGE>` — provider npmPackage
- `<PINNED_VERSION>` — provider pinnedVersion
- `<ENV_PREFIX>` — env cleanup from provider.unsetEnv (e.g. `unset CLAUDECODE;` or `$env:CLAUDECODE=$null;`)
- `<ARGS>` — provider defaultArgs (joined with spaces, empty for all current providers)

### Claude: npm Instead of Native Installer

The current `cli_installer.rs` uses Claude's native installer (`irm`/`curl`) which installs to system-wide locations, defeating isolation. **All three providers will use `npm install --prefix` for consistency.** Claude's npm package (`@anthropic-ai/claude-code`) works fine and includes the full CLI.

## Changes Required

### 1. `frontend/app/view/agent/agent-model.ts`

Replace the current `connectWithProvider()` implementation. The key change is replacing the bare command injection with a bootstrap script.

**Current code (lines 73-96):**
```typescript
setTimeout(async () => {
    let envPrefix = "";
    // ... env unset logic ...
    const cliCmd = provider.defaultArgs.length > 0
        ? `${cliPath} ${provider.defaultArgs.join(" ")}`
        : cliPath;
    const cmdText = `${envPrefix}${cliCmd}\r`;
    // ... inject into terminal ...
}, 500);
```

**New code:**
```typescript
setTimeout(async () => {
    const version = getApi().getAboutModalDetails().version;
    const isWindows = getApi().getPlatform() === "win32";
    const script = buildBootstrapScript({
        version,
        provider,
        isWindows,
    });
    const b64data = stringToBase64(script + "\r");
    await RpcApi.ControllerInputCommand(TabRpcClient, {
        blockid: blockId,
        inputdata64: b64data,
    });
}, 500);
```

### 2. New utility: `frontend/app/view/agent/bootstrap.ts`

```typescript
import type { ProviderDefinition } from "./providers";

interface BootstrapOptions {
    version: string;
    provider: ProviderDefinition;
    isWindows: boolean;
}

/**
 * Build a one-liner bootstrap script that:
 * 1. Checks if the CLI binary exists in the version-isolated directory
 * 2. If missing, runs npm install (visible in terminal)
 * 3. Launches the CLI with env cleanup and args
 */
export function buildBootstrapScript(opts: BootstrapOptions): string {
    const { version, provider, isWindows } = opts;
    const cliDir = `$HOME/.agentmux/instances/v${version}/cli/${provider.id}`;
    const pkg = `${provider.npmPackage}@${provider.pinnedVersion}`;

    // Build env cleanup prefix
    let envPrefix = "";
    if (provider.unsetEnv?.length) {
        if (isWindows) {
            envPrefix = provider.unsetEnv.map((v) => `$env:${v}=$null`).join("; ") + "; ";
        } else {
            envPrefix = provider.unsetEnv.map((v) => `unset ${v}`).join("; ") + "; ";
        }
    }

    // Build args suffix
    const argsSuffix = provider.defaultArgs.length > 0
        ? " " + provider.defaultArgs.join(" ")
        : "";

    if (isWindows) {
        const bin = `$d/node_modules/.bin/${provider.cliCommand}.cmd`;
        // PowerShell one-liner
        return [
            `$d="${cliDir}"`,
            `$b="${bin}"`,
            `if(!(Test-Path $b)){Write-Host "Installing ${provider.displayName}..."`,
            `npm install --prefix $d ${pkg} --no-fund --no-audit}`,
            `${envPrefix}& $b${argsSuffix}`,
        ].join("; ");
    } else {
        const bin = `$CLI_DIR/node_modules/.bin/${provider.cliCommand}`;
        // Bash one-liner
        return [
            `CLI_DIR="${cliDir}"`,
            `CLI_BIN="${bin}"`,
            `{ [ -x "$CLI_BIN" ] || { echo "Installing ${provider.displayName}..."`,
            `npm install --prefix "$CLI_DIR" ${pkg} --no-fund --no-audit; }; }`,
            `${envPrefix}"$CLI_BIN"${argsSuffix}`,
        ].join(" && ");
    }
}
```

### 3. Same changes for `connectStyled()`

The styled mode also injects a bare CLI command. Apply the same bootstrap pattern but with `provider.styledArgs` instead of `provider.defaultArgs`:

```typescript
// In connectStyled(), replace the setTimeout callback:
const version = getApi().getAboutModalDetails().version;
const isWindows = getApi().getPlatform() === "win32";
const script = buildBootstrapScript({
    version,
    provider: { ...provider, defaultArgs: provider.styledArgs },
    isWindows,
});
```

### 4. `src-tauri/src/commands/cli_installer.rs` — Update or deprecate

Two options:

**Option A (recommended): Keep as backend API, update paths**

Update `get_provider_install_dir()` to use version-isolated paths. Keep the Tauri commands available for future use (e.g. pre-warming installs on startup, health checks):

```rust
fn get_provider_install_dir(provider: &str, version: &str) -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or("Could not determine home directory")?;
    Ok(home
        .join(".agentmux")
        .join("instances")
        .join(format!("v{}", version))
        .join("cli")
        .join(provider))
}
```

Remove the `install_claude_native()` function — Claude now uses npm like everyone else.

Update `get_cli_path()` to no longer check system PATH first (isolation means we always use our own copy).

**Option B: Deprecate entirely**

The bootstrap script handles everything. Mark `cli_installer.rs` as deprecated. Remove Tauri commands from `main.rs`. Simpler but loses the backend API for future use.

## Remote Backend Support

When connecting to a remote backend (SSH), the bootstrap script runs in the remote shell. This works automatically because:

1. The shell controller already supports remote shells (the PTY is on the remote machine)
2. `$HOME` resolves to the remote user's home directory
3. `npm` must be available on the remote machine (same requirement as today)

The install directory on the remote machine will be:
```
/home/remote-user/.agentmux/instances/v0.31.84/cli/claude/
```

Each remote backend gets its own isolated install because each backend has its own filesystem.

## Edge Cases

| Scenario | Behavior |
|----------|----------|
| npm not installed | `npm: command not found` visible in terminal |
| Network offline | npm install fails visibly in terminal |
| CLI already installed for this version | Skips install, launches directly |
| Same CLI version across AgentMux upgrades | Reinstalls per version (disk tradeoff for isolation) |
| User has CLI on system PATH | Ignored — always uses version-isolated install |
| Disk space | Each provider ~50-150MB; 3 providers per version ~300-450MB |
| PowerShell 5.1 vs 7 | Script uses `Test-Path` which works in both |
| Remote backend (SSH) | Script runs in remote shell, installs on remote filesystem |
| Concurrent installs | npm handles concurrent prefix installs safely |

## Cleanup (Future)

Not in scope for initial implementation, but planned:

On startup, scan `~/.agentmux/instances/` and delete `cli/` directories for versions older than N releases back (e.g. keep last 3 versions). This prevents unbounded disk growth.

## What NOT to Change

- **Shell controller** (`shell.rs`) — just spawns a shell and passes input. No changes needed.
- **Provider definitions** (`providers/index.ts`) — already have all required fields.
- **Backend sidecar** (`sidecar.rs`) — already creates `instances/v<version>/` directories.
- **System PATH** — never modified. All isolation is via absolute paths in the bootstrap script.

## Files Modified

| File | Change |
|------|--------|
| `frontend/app/view/agent/agent-model.ts` | Replace bare command injection with bootstrap script in both `connectWithProvider()` and `connectStyled()` |
| `frontend/app/view/agent/bootstrap.ts` | **New file** — `buildBootstrapScript()` function |
| `src-tauri/src/commands/cli_installer.rs` | Update paths to version-isolated directories, remove `install_claude_native()`, remove system PATH detection |

## Verification

1. Delete `~/.agentmux/instances/v<current>/cli/` if it exists
2. Launch AgentMux, open agent pane
3. Click "Claude Code" (raw mode) → terminal opens, shows `Installing Claude Code...`, npm progress, then launches claude
4. Close terminal, click "Claude Code" again → no install, launches directly (fast)
5. Click "Codex CLI" → installs codex independently, doesn't affect claude install
6. Click "Gemini CLI" → same pattern
7. Verify each provider's directory exists under `~/.agentmux/instances/v<version>/cli/`
8. Verify system PATH was never modified
9. Test styled mode — same bootstrap, different args
10. Test with npm not installed — error visible in terminal
