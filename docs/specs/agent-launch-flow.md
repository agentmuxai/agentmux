# Spec: Agent Launch Flow — First Run to First Response

**Goal:** Define the complete, production-quality flow from clicking an agent card to receiving the first streamed response. Covers CLI detection, installation, authentication, subprocess spawn, and error recovery. Every state transition must be visible to the user as terminal-style log lines.

**Status:** Design phase. Replaces the current broken flow where spawn failures are silent.

---

## Current State (Broken)

The current flow has multiple failure points that are invisible to the user:

1. **CLI path:** `resolveCliDir()` constructs a versioned path (`~/.agentmux/instances/v0.32.8/cli/claude/node_modules/.bin/claude`) that doesn't exist unless a prior install ran. Spawn fails silently.
2. **Tilde expansion:** Rust `Command::current_dir("~/...")` treats `~` literally. Fixed in this session but symptomatic of the broader issue.
3. **Auth:** No auth check before spawn. If Claude CLI isn't logged in, the subprocess exits immediately with a non-obvious error.
4. **No feedback:** The user sees "Waiting for output..." or static log lines with no indication of what's happening.
5. **Controller lost on restart:** After app restart, the subprocess controller doesn't exist. Fixed with `ControllerResyncCommand` on mount, but the UX is still poor.

---

## Design Principles

1. **Terminal-style feedback:** Every step emits a log line. The user sees a scrolling boot sequence, not spinners or cards.
2. **Fail fast, fail loud:** Errors appear as red log lines with actionable messages.
3. **Idempotent:** Re-running the flow (e.g., after a failed install) picks up where it left off.
4. **No hidden state:** Every decision (which CLI path, whether auth is needed) is logged.
5. **Provider-agnostic:** The flow works for Claude, Codex, Gemini, or any future provider. Provider-specific logic lives in `ProviderDefinition`.

---

## Complete Flow

### Phase 0: Agent Selection

```
User clicks agent card in Forge picker
  → [agent] AgentX selected (provider: Claude Code)
  → [agent] loading agent configuration...
```

**Actions:**
- Load forge agent record from DB
- Load all forge content (soul, agentmd, mcp, env)
- Load skills
- Resolve provider from `PROVIDERS[agent.provider]`

**Failure:** Provider not found → `[error] unknown provider "xxx" — check forge agent configuration`

---

### Phase 1: CLI Detection

```
  → [cli] checking for claude...
  → [cli] found: /c/Users/area54/.local/bin/claude (v2.1.76)
```

OR if not found:

```
  → [cli] claude not found in PATH
  → [cli] installing @anthropic-ai/claude-code...
  → [cli] npm install -g @anthropic-ai/claude-code
  → [cli] installed claude v2.1.76
```

**Strategy — Tiered Resolution:**

1. **PATH lookup:** Run `which <cmd>` (Unix) or `where <cmd>` (Windows) via a lightweight backend RPC. If found, use it. Log the resolved path and version (`<cmd> --version`).

2. **Version-isolated install (optional):** If PATH lookup fails, install to `~/.agentmux/cli/<provider>/node_modules/.bin/<cmd>` using `npm install --prefix ~/.agentmux/cli/<provider> <npmPackage>@<pinnedVersion>`. This is a one-time operation per provider.

3. **Fallback:** If both fail, show error with install instructions.

**New RPC: `ResolveCliCommand`**

```rust
struct CommandResolveCliData {
    provider_id: String,      // "claude", "codex", "gemini"
    cli_command: String,      // "claude"
    npm_package: String,      // "@anthropic-ai/claude-code"
    pinned_version: String,   // "latest" or "2.1.76"
}

struct ResolveCliResult {
    cli_path: String,         // absolute path to binary
    version: String,          // "2.1.76"
    source: String,           // "path" | "installed" | "cached"
}
```

**Backend implementation:**
1. Try `which`/`where` → if found, run `<cmd> --version`, return result
2. Check `~/.agentmux/cli/<provider>/node_modules/.bin/<cmd>` → if exists, use it
3. Run `npm install --prefix ~/.agentmux/cli/<provider> <pkg>@<version>`
4. Verify install succeeded
5. Return resolved path

**Log lines emitted via WPS events** so the frontend can display them in real-time during install.

---

### Phase 2: Auth Check

```
  → [auth] checking claude authentication...
  → [auth] logged in as user@example.com (Pro plan)
```

OR if not authenticated:

```
  → [auth] not authenticated
  → [auth] opening browser for login...
  → [auth] waiting for authentication... (press Ctrl+C to cancel)
  → [auth] authenticated as user@example.com
```

**Implementation:**

Each provider defines `authCheckCommand` and `authLoginCommand` in `ProviderDefinition`:

| Provider | Auth Check | Auth Login |
|----------|-----------|------------|
| Claude | `claude auth status --json` | `claude auth login` |
| Codex | `codex login status` | `codex login` |
| Gemini | `gemini auth status` | `gemini auth login` |

**Auth check flow:**

1. Run `<cli_path> <authCheckCommand...>` as subprocess
2. Parse output (JSON for Claude: `{ loggedIn: bool, email, subscriptionType }`)
3. If authenticated → log and proceed
4. If not → run auth login flow

**Auth login flow (Claude-specific):**

1. Run `claude auth login` — it prints an OAuth URL to stdout
2. Capture URL via regex: `https://claude.ai/oauth/authorize.*`
3. Call `getApi().openExternal(url)` to open browser
4. Wait for subprocess to exit (login completes when user authorizes in browser)
5. Re-run auth check to confirm
6. If timeout (60s) → show error with manual instructions

**Auth login flow (generic):**

1. Run `<cli_path> <authLoginCommand...>` with stdin/stdout piped
2. Monitor stdout for URLs (open in browser)
3. Wait for exit code 0

---

### Phase 3: Environment Setup

```
  → [env] working directory: C:\Users\area54\.claw\agentx-workspace
  → [env] writing CLAUDE.md (2.4KB)
  → [env] writing .mcp.json (340B)
  → [env] environment: AGENT_NAME=agentx, AGENTBUS_AGENT_ID=agentx
```

**Actions:**
- Expand `~` in working directory path
- Create working directory if it doesn't exist
- Write config files (CLAUDE.md, .mcp.json) via `WriteAgentConfigCommand`
- Build environment variables from forge content + provider.unsetEnv
- Log each file written and env var set

---

### Phase 4: Controller Registration

```
  → [controller] registering subprocess controller
  → [controller] status: init
  → [controller] ready — type a message below to start
```

**Actions:**
- Set block metadata: `agentId`, `agentProvider`, `cmd`, `cmd:args`, `cmd:cwd`, `cmd:env`, `controller: "subprocess"`
- Call `ControllerResyncCommand` with `forcerestart: true`
- Subscribe to `controllerstatus` events

---

### Phase 5: User Sends Message

```
  → [input] sending message to AgentX...
  → [subprocess] spawning: claude -p --verbose --output-format stream-json
  → [subprocess] pid: 12345
  → [subprocess] waiting for response...
```

**Actions:**
- Append user message to document as `user_message` node
- Call `AgentInputCommand` RPC
- Backend spawns subprocess with message on stdin
- Log PID and command line

**On error:**

```
  → [error] failed to spawn subprocess: The system cannot find the file specified. (os error 2)
  → [error] CLI path: /c/Users/area54/.agentmux/cli/claude/node_modules/.bin/claude
  → [error] try: npm install -g @anthropic-ai/claude-code
```

---

### Phase 6: Streaming Response

```
  → [stream] connected, receiving output...
  (document nodes render below as they arrive)
```

Once the first `DocumentNode` arrives, the status log area is replaced by the actual document view (markdown blocks, tool calls, etc.).

---

### Phase 7: Turn Complete

```
  (after last document node)
  → [subprocess] exit code: 0
  → [agent] turn complete — send another message to continue
```

---

## State Machine

```
IDLE → RESOLVING_CLI → CHECKING_AUTH → AUTH_LOGIN → SETTING_UP_ENV → REGISTERING → READY → SPAWNING → STREAMING → DONE
                ↓              ↓            ↓              ↓                                    ↓
             ERROR          ERROR        ERROR          ERROR                                ERROR
```

Each state maps to log lines. The `ERROR` state shows the error and allows retry.

### State Storage

Add to `AgentProcessState`:

```typescript
interface AgentProcessState {
    pid?: number;
    agentId: string;
    status: "idle" | "resolving_cli" | "checking_auth" | "auth_login" | "setting_up" | "ready" | "running" | "done" | "error";
    canRestart: boolean;
    canKill: boolean;
    errorMessage?: string;
    cliPath?: string;
    cliVersion?: string;
    authEmail?: string;
}
```

---

## Log Line Format

All status output uses a consistent terminal-style format:

```
[tag] message
```

Tags and colors:
- `[agent]` — default text color
- `[cli]` — default text color
- `[auth]` — default text color
- `[env]` — default text color
- `[controller]` — default text color
- `[subprocess]` — default text color
- `[stream]` — default text color
- `[input]` — default text color
- `[error]` — red (`var(--error-color)`)
- `[warn]` — amber (`var(--warning-color)`)

Lines are monospace, left-aligned, no centering, no cards, no spinners. Like a terminal boot sequence.

---

## Implementation Plan

### Step 1: Fix immediate spawn failure (this session)

| Change | File |
|--------|------|
| Use bare `provider.cliCommand` instead of versioned path | `agent-model.ts` |
| Log errors from `AgentInputCommand` to document | `agent-view.tsx` |
| Expand `~` in working dir and config paths | `subprocess.rs`, `websocket.rs` |

### Step 2: CLI detection RPC

| Change | File |
|--------|------|
| New `ResolveCliCommand` RPC handler | `websocket.rs` |
| New `CommandResolveCliData` / `ResolveCliResult` types | `rpc_types.rs` |
| Frontend types | `gotypes.d.ts`, `wshclientapi.ts` |
| Call in `launchForgeAgent` before SetMeta | `agent-model.ts` |

### Step 3: Auth check + login

| Change | File |
|--------|------|
| New `CheckCliAuthCommand` RPC handler | `websocket.rs` |
| Auth login subprocess with URL capture | `websocket.rs` |
| Frontend auth state + log lines | `agent-model.ts`, `agent-view.tsx` |

### Step 4: Verbose log lines

| Change | File |
|--------|------|
| Replace static `statusLines()` with accumulated log signal | `AgentDocumentView.tsx` |
| Emit log lines from each phase of `launchForgeAgent` | `agent-model.ts` |
| Backend emits log events during spawn | `subprocess.rs` |

### Step 5: Auto-install on first launch

| Change | File |
|--------|------|
| `npm install` logic in `ResolveCliCommand` | `websocket.rs` |
| Progress events during install | WPS events |
| Frontend subscribes and displays install progress | `agent-view.tsx` |

---

## Provider Configuration Reference

All provider-specific behavior is driven by `ProviderDefinition` in `providers/index.ts`:

```typescript
interface ProviderDefinition {
    id: string;                  // "claude"
    displayName: string;         // "Claude Code"
    cliCommand: string;          // "claude"
    defaultArgs: string[];       // []
    styledArgs: string[];        // ["--output-format", "stream-json", "--verbose"]
    outputFormat: string;        // "raw" | "claude-stream-json"
    styledOutputFormat: string;  // "claude-stream-json"
    authType: string;            // "oauth" | "api-key"
    authCheckCommand: string[];  // ["auth", "status", "--json"]
    authLoginCommand: string[];  // ["auth", "login"]
    npmPackage: string;          // "@anthropic-ai/claude-code"
    pinnedVersion: string;       // "latest"
    docsUrl: string;
    icon: string;
    unsetEnv?: string[];         // ["CLAUDECODE"]
}
```

No hardcoded provider logic anywhere in the flow. Adding a new provider = adding an entry to `PROVIDERS`.

---

## Edge Cases

### App restart with existing agent pane
- `AgentPresentationView` mounts → calls `ControllerResyncCommand` (no force) → controller re-registered
- If user sends message → works because controller exists
- Previous session context preserved via `agent:sessionid` in block meta → `--resume` appended

### CLI updated externally
- User updates `claude` via `npm install -g` outside AgentMux
- Next spawn uses updated binary (PATH resolution is per-spawn)
- Version shown in log lines reflects current version

### Multiple agents, different providers
- Each agent pane resolves its own CLI independently
- Different providers can coexist (Claude + Codex + Gemini)
- Auth is per-provider (separate OAuth flows)

### Network offline
- CLI detection works (local binary)
- Auth check works if already authenticated (cached token)
- `npm install` fails → show error with offline message
- Subprocess spawn works if CLI and auth are already set up

### Windows PATH issues
- Rust `Command::new("claude")` searches PATH but backend sidecar inherits Tauri's environment
- Tauri on Windows may not have user-modified PATH entries (e.g., `~/.local/bin`)
- Solution: `ResolveCliCommand` runs `where claude` in a shell context that has full PATH
- Fallback: check common install locations (`%USERPROFILE%\.local\bin`, `%APPDATA%\npm`)

---

## Testing Matrix

| Scenario | Expected Behavior |
|----------|-------------------|
| CLI installed, auth valid | Fast path: detect → check auth → setup → ready (< 2s) |
| CLI installed, auth expired | Detect → auth fail → open browser → wait → ready |
| CLI not installed | Detect fail → install → detect → auth check → ready |
| CLI not installed, npm missing | Detect fail → install fail → error with instructions |
| Wrong provider in DB | Error: "unknown provider" |
| Working dir doesn't exist | Create it, log creation |
| Working dir has `~` prefix | Expand to home dir |
| App restart, agent pane persisted | Resync controller → ready (no re-auth needed) |
| Subprocess exits non-zero | Show exit code + stderr in log lines |
| Subprocess hangs | User can send SIGINT or close pane |
