# Spec: Provider Auth Isolation per AgentMux Version

**Status:** Draft
**Author:** AgentA
**Date:** 2026-03-21

---

## Goal

Each AgentMux version gets its own isolated auth space for every provider (Claude, Codex, Gemini, and future OAuth providers). Auth state is never shared with the user's personal CLI installations, and never automatically migrated between AgentMux versions.

**Benefits:**
- **QA:** Each release starts with clean auth — re-login always exercises the full auth flow
- **Isolation:** AgentMux agents never interfere with the user's personal `claude`/`codex`/`gemini` sessions
- **Predictability:** Auth bugs can't be masked by stale tokens from a previous version

---

## Directory Structure

```
~/.agentmux/
  instances/
    v0.32.62/
      auth/
        claude/          ← CLAUDE_CONFIG_DIR
        codex/           ← CODEX_HOME  (must be pre-created)
        gemini/          ← GEMINI_CLI_HOME  (.gemini/ lives inside)
```

Each provider gets a subdirectory under the version-specific auth dir. On upgrade, the new version starts with empty auth dirs — user re-auths once per provider.

---

## Per-Provider Isolation

| Provider | Env Var | Credentials Location | Notes |
|----------|---------|----------------------|-------|
| Claude   | `CLAUDE_CONFIG_DIR` | `{dir}/.credentials.json` | macOS uses Keychain, namespaced by dir path hash |
| Codex    | `CODEX_HOME` | `{dir}/.credentials.json` (MCP OAuth) | **Dir must pre-exist** — Codex errors if missing |
| Gemini   | `GEMINI_CLI_HOME` | `{dir}/.gemini/oauth_creds.json` | Set `GEMINI_FORCE_FILE_STORAGE=true` to skip Keychain on macOS |

### Claude
```
CLAUDE_CONFIG_DIR=~/.agentmux/instances/v{version}/auth/claude
```
Fully isolates `.credentials.json`, settings, sessions. macOS Keychain entry is namespaced by hash of the dir path — no collision with personal `~/.claude/`.

### Codex
```
CODEX_HOME=~/.agentmux/instances/v{version}/auth/codex
```
Dir must be pre-created before spawning Codex (error if missing). Isolates `config.toml`, session history, MCP OAuth tokens.

### Gemini
```
GEMINI_CLI_HOME=~/.agentmux/instances/v{version}/auth/gemini
GEMINI_FORCE_FILE_STORAGE=true
```
`GEMINI_CLI_HOME` shifts the home base — `.gemini/` ends up at `{dir}/.gemini/`. `GEMINI_FORCE_FILE_STORAGE=true` disables Keychain on macOS, ensuring credentials go to `{dir}/.gemini/oauth_creds.json` (file-based, consistent cross-platform).

---

## Shared vs Per-Agent Auth

Auth dirs are **shared across all agents of the same provider within a version**. There is no per-agent auth isolation.

```
AgentX (claude provider)  ─┐
AgentB (claude provider)  ─┤── CLAUDE_CONFIG_DIR = v0.32.62/auth/claude/
AgentC (claude provider)  ─┘

AgentD (gemini provider)  ── GEMINI_CLI_HOME = v0.32.62/auth/gemini/
```

Per-agent isolation (project config, CLAUDE.md, .mcp.json, working dir) is handled via `cmd:cwd`, not auth dirs.

---

## Implementation Changes

### 1. Provider Definition — Add `authConfigDirEnvVar` and `authDirName`

In `frontend/app/view/agent/providers/index.ts`:

```typescript
export interface ProviderDefinition {
    // ... existing fields ...
    authConfigDirEnvVar: string;   // env var that sets the config/auth dir
    authDirName: string;           // subdir name under auth/  (e.g. "claude")
    authExtraEnv?: Record<string, string>;  // e.g. GEMINI_FORCE_FILE_STORAGE=true
}

// claude
authConfigDirEnvVar: "CLAUDE_CONFIG_DIR",
authDirName: "claude",

// codex
authConfigDirEnvVar: "CODEX_HOME",
authDirName: "codex",

// gemini
authConfigDirEnvVar: "GEMINI_CLI_HOME",
authDirName: "gemini",
authExtraEnv: { GEMINI_FORCE_FILE_STORAGE: "true" },
```

### 2. Auth Dir Resolution — New Tauri Command

Add `get_auth_dir(provider_id: string) -> Result<String>` Tauri command:

```rust
// Returns: ~/.agentmux/instances/v{version}/auth/{provider}/
// Creates the directory if it doesn't exist.
pub async fn get_auth_dir(app: tauri::AppHandle, provider_id: String) -> Result<String, String>
```

Backend resolves the current version from `CARGO_PKG_VERSION`, builds the path, pre-creates it (required by Codex).

### 3. Auth Env Vars — Injected at Spawn Time

In `agent-model.ts` `launchForgeAgent` and `launchAgent`:
- **Remove** per-agent `CLAUDE_CONFIG_DIR` from env vars
- **Add** auth dir env vars via `getApi().getAuthDir(provider.id)`

```typescript
const authDir = await getApi().getAuthDir(provider.id);
envVars[provider.authConfigDirEnvVar] = authDir;
if (provider.authExtraEnv) {
    Object.assign(envVars, provider.authExtraEnv);
}
```

Also inject into `authCheckCommand` invocation (the CLI auth check must also use the same isolated dir).

### 4. Auth Check — Use Isolated Dir

The `CheckCliAuth` backend command must pass the auth dir env var when running `claude auth status --json` (and equivalents). Otherwise the check passes (using personal `~/.claude/`) but the subprocess fails (using the isolated empty dir).

In `websocket.rs` `CheckCliAuth` handler: accept optional `auth_env` map and pass to the spawned auth-check process.

### 5. Auth Login Flow

When auth check fails (not authenticated), the agent pane shows a login prompt. The login command (`claude auth login`) must be run with the same isolated `CLAUDE_CONFIG_DIR` set, so tokens land in the right place.

Add `RunAuthLogin` Tauri command (or reuse `open_external` for OAuth URL interception):
```rust
// Spawns: claude auth login with CLAUDE_CONFIG_DIR set to the version-isolated dir
// Emits oauth URL via stdout for the frontend to open in browser
pub async fn run_auth_login(provider_id: String, auth_dir: String) -> Result<(), String>
```

---

## Auth Flow (User Experience)

1. User opens an agent pane for the first time on a new AgentMux version
2. Launch flow runs auth check → not authenticated (clean dir)
3. Pane shows: **"Log in to Claude Code to continue"** + **[Log In]** button
4. User clicks → AgentMux runs `claude auth login` with isolated `CLAUDE_CONFIG_DIR`, opens browser
5. User completes OAuth → credentials written to `v{version}/auth/claude/.credentials.json`
6. Auth check re-runs → authenticated → agent ready
7. All subsequent agents on this version use the same credentials silently

On upgrade to v0.32.63:
- New empty auth dir → step 1 repeats (intentional for QA)
- Old auth dir remains at `v0.32.62/auth/` (not deleted)

---

## Exit Code 1 Fixes (Immediate, before full auth flow)

While the full auth isolation spec is being implemented, fix the current exit 1 with:

1. **Share one auth dir across all agents** — remove per-agent `CLAUDE_CONFIG_DIR` from `launchForgeAgent`, use `~/.agentmux/instances/v{version}/auth/claude/` for all
2. **Add `--dangerously-skip-permissions`** to `styledArgs` — required for non-interactive `-p` mode
3. **Add `CLAUDE_CODE_EXIT_AFTER_STOP_DELAY=30000`** to env vars — prevents process hang after `result` event

---

## Future: Version-to-Version Auth Migration (Optional)

If version-per-auth proves too disruptive for non-QA users, add an opt-in migration path:

```
~/.agentmux/auth/            ← stable shared auth (post-migration)
  claude/
  codex/
  gemini/

~/.agentmux/instances/v*/auth/  ← version-isolated (QA / dev builds)
```

Controlled by a setting: `auth.isolationMode = "version" | "shared"`. Default `"version"` for dev builds, `"shared"` for production. Out of scope for now.

---

## Open Questions

1. **Token refresh across sessions** — does `CLAUDE_CONFIG_DIR` isolation break token refresh if the user's personal Claude session refreshes first? (Probably no, each dir manages its own refresh token independently.)
2. **Codex API key vs OAuth** — Codex currently uses `--full-auto` which expects `OPENAI_API_KEY`. Should it get the same OAuth isolation treatment, or remain API-key-only?
3. **Gemini `GEMINI_FORCE_FILE_STORAGE`** — does this have any macOS Keychain permission side effects on first launch?
