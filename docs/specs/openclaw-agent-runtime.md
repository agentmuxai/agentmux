# OpenClaw Agent Runtime Integration

**Status:** Draft
**Date:** 2026-03-20
**Author:** AgentY

---

## Overview

This spec defines how OpenClaw becomes a first-class agent runtime in AgentMux — a peer to Claude Code and Gemini CLI, not a subordinate widget. Just as AgentX runs `claude` in a PTY session and AgentZ runs `gemini`, **AgentClaw** runs `openclaw tui` in a PTY session with full identity isolation, AgentBus wiring, and Forge management support.

This is **separate** from the existing `openclaw-widget.md` spec (WebView dashboard at `localhost:18789`). That widget embeds the OpenClaw web UI. This spec covers OpenClaw as a **gateway-backed interactive agent runtime** — a TUI session in a terminal pane connected to the running OpenClaw gateway.

---

## OpenClaw Architecture: Understanding the Model

OpenClaw is fundamentally different from Claude Code and Gemini CLI. Understanding the architecture is essential for correct integration.

### Not a Single Coding Session

Claude Code and Gemini CLI are **single-turn coding agents**: one terminal pane, one LLM session, user types prompts, agent edits files.

OpenClaw is a **gateway-backed orchestrator**: it runs a persistent daemon (`openclaw gateway`) that:
- Receives messages from external channels (WhatsApp, Telegram, Discord, webhooks)
- Routes them to the correct agent identity
- Executes LLM turns that call tools and invoke skills
- Replies back to the originating channel

### The Orchestration Model

**Primary use case:** A user sends a WhatsApp message — "Get all emails from Karen and make a presentation."

The execution flow:
```
WhatsApp → OpenClaw gateway → agent:main → LLM turn
  ├── skill: himalaya (IMAP email fetch)     → reads emails
  ├── skill: gemini (summarization)          → summarizes content
  └── skill: coding-agent (spawn Claude Code) → creates .pptx
         └── Claude Code subprocess (background, non-PTY)
```

This is **all within one LLM session**. Skills are tool calls, not separate visible agents. The `coding-agent` skill is the exception — it spawns an actual subprocess (Claude Code, Codex, Pi) to handle coding work, using `--print --permission-mode bypassPermissions` for non-PTY headless operation.

### Agent Identities vs Orchestration Sub-agents

OpenClaw uses two distinct concepts that look similar but aren't:

| Concept | Command | Purpose |
|---------|---------|---------|
| **Agent identities** | `openclaw agents` | Isolated personas with own workspace, auth, routing rules. Like mailboxes — routes different channels (Telegram personal vs Discord server) to different agent configs. |
| **ACP sub-agents** | `openclaw acp` | Protocol bridge for spawning agent processes (Claude Code, Codex) as sub-tasks within a session. Used internally by the `coding-agent` skill. |

**Current state on this system:** One agent configured: `main` (workspace: `~\.openclaw\workspace`). One session: `agent:main:main` (claude-opus-4-6).

### Interactive Access: `openclaw tui`

The correct terminal pane command for AgentMux is:

```
openclaw tui
```

This opens an interactive TUI connected to the running gateway — shows conversation history, accepts user input, streams agent responses. Equivalent to `claude` or `gemini` in terms of UX.

Compare:
| Provider | Terminal Command | Mode |
|----------|-----------------|------|
| `claude` | `claude` | Interactive REPL (self-contained) |
| `gemini` | `gemini` | Interactive REPL (self-contained) |
| `codex` | `codex` | Interactive REPL (self-contained) |
| `openclaw` | `openclaw tui` | Interactive TUI (gateway-backed) |

**Key difference:** OpenClaw's TUI requires the gateway (`openclaw gateway`) to be running. The TUI is a client that connects over WebSocket to `ws://127.0.0.1:18789`.

---

## Shell Resolution: Host vs Container

**Container agents always run `bash`.** They're Linux Docker containers — the shell is
deterministic regardless of what OS the host runs.

**Host agents use whatever shell the host provides.** The `shell` field in the manifest
reflects the host's available shell — `pwsh` on Windows (PowerShell 7), `bash` or `zsh`
on macOS/Linux. The seed manifest ships with Windows defaults (`pwsh`) because that's the
reference platform; users on other OSes set their preferred shell in Forge or override via
`SHELL` env.

```
agent_type: "container" → shell: "bash"   (always — Linux only)
agent_type: "host"      → shell: <host>   (pwsh | bash | zsh | fish | …)
```

---

## Current Runtime Landscape

```
forge-seed.json (version 2)
├── Host Agents  (shell = host default — pwsh on Windows)
│   ├── AgentX  │ provider: "claude"  │ shell: pwsh
│   ├── AgentY  │ provider: "codex"   │ shell: pwsh
│   └── AgentZ  │ provider: "gemini"  │ shell: pwsh
└── Container Agents  (shell = bash, always)
    ├── Agent1  │ provider: "claude"  │ shell: bash
    ├── Agent2  │ provider: "codex"   │ shell: bash
    └── Agent3  │ provider: "gemini"  │ shell: bash
```

**Target state (version 3):**

```
forge-seed.json (version 3)
├── Host Agents  (shell = host default)
│   ├── AgentX    │ provider: "claude"    │ shell: pwsh
│   ├── AgentY    │ provider: "codex"     │ shell: pwsh
│   ├── AgentZ    │ provider: "gemini"    │ shell: pwsh
│   └── AgentClaw │ provider: "openclaw"  │ shell: pwsh   ← NEW
└── Container Agents  (shell = bash, always)
    ├── Agent1    │ provider: "claude"    │ shell: bash
    ├── Agent2    │ provider: "codex"     │ shell: bash
    ├── Agent3    │ provider: "gemini"    │ shell: bash
    └── Agent4    │ provider: "openclaw"  │ shell: bash   ← NEW
```

---

## 1. forge-seed.json Changes

Bump version to `3` and add two new entries:

### AgentClaw (Host — Windows)

```json
{
  "id": "agentclaw",
  "name": "AgentClaw",
  "icon": "🦞",
  "agent_type": "host",
  "environment": "windows",
  "provider": "openclaw",
  "description": "OpenClaw on host — gateway-backed agent with messaging, skills, and memory",
  "working_directory": "~/.claw/agentclaw-workspace",
  "shell": "pwsh",
  "agent_bus_id": "agentclaw",
  "auto_start": false,
  "restart_on_crash": false,
  "content": {
    "env": "AGENT_NAME=agentclaw\nAGENTMUX_AGENT_ID=AgentClaw\nOPENCLAW_AGENT_ID=agentclaw"
  },
  "skills": []
}
```

### Agent4 (Container — Linux)

```json
{
  "id": "agent4",
  "name": "Agent4",
  "icon": "🟤",
  "agent_type": "container",
  "environment": "linux",
  "provider": "openclaw",
  "description": "OpenClaw in container — sandboxed gateway-backed agent",
  "working_directory": "/workspace",
  "shell": "bash",
  "agent_bus_id": "agent4",
  "auto_start": false,
  "restart_on_crash": true,
  "content": {
    "env": "AGENT_NAME=agent4\nAGENTMUX_AGENT_ID=Agent4\nOPENCLAW_AGENT_ID=agent4"
  },
  "skills": []
}
```

---

## 2. OpenClaw CLI Invocation

### Startup Sequence for AgentClaw Terminal Pane

The AgentClaw pane runs `openclaw tui` — an interactive TUI connected to the gateway. The gateway must be running first.

**Host (Windows pwsh):**

```powershell
# 1. Ensure gateway is running (idempotent — no-op if already up)
$health = try { (Invoke-RestMethod http://127.0.0.1:18789/health).ok } catch { $false }
if (-not $health) {
    # Gateway registered as Windows Scheduled Task; start it
    Start-ScheduledTask -TaskName "OpenClaw Gateway" -ErrorAction SilentlyContinue
    Start-Sleep 3
}

# 2. Open interactive TUI session
openclaw tui --session main
```

**Linux container (bash):**

```bash
# Gateway must be started differently in container (not a Windows Scheduled Task)
openclaw gateway --port 18789 --detach &
sleep 2

# Open interactive TUI
openclaw tui --session main
```

### `openclaw agent` vs `openclaw tui`

| Command | Mode | Use case |
|---------|------|---------|
| `openclaw agent --message "..."` | One-shot, exits | Scripted/automated single turns |
| `openclaw tui` | Interactive TUI | Human-in-the-loop terminal pane (AgentMux) |
| `openclaw acp client` | ACP bridge client | Sub-agent protocol sessions |

**AgentClaw uses `openclaw tui`** — the interactive mode that stays open and accepts input, equivalent to the `claude` REPL.

### Shell Init Script (content.env)

The `content.env` field injects variables into the shell before the provider starts. For OpenClaw:

```
AGENT_NAME=agentclaw
AGENTMUX_AGENT_ID=AgentClaw
OPENCLAW_AGENT_ID=agentclaw
OPENCLAW_GATEWAY_URL=ws://127.0.0.1:18789
```

---

## 3. Backend Changes (`agentmuxsrv-rs`)

### 3a. forge-seed.rs — No structural changes

The `SeedAgent` struct already accepts any string for `provider`. No Rust changes needed for the manifest parsing.

### 3b. shellexec.rs — OpenClaw provider detection

If `shellexec.rs` has provider-specific spawn logic, add the `openclaw` case:

```rust
match agent.provider.as_str() {
    "claude" => {
        opts.shell_opts = vec!["claude".into()];
    }
    "gemini" => {
        opts.shell_opts = vec!["gemini".into()];
    }
    "codex" => {
        opts.shell_opts = vec!["codex".into()];
    }
    "openclaw" => {
        // Launch openclaw interactive TUI (gateway-backed)
        opts.shell_opts = vec![
            "openclaw".into(),
            "tui".into(),
            "--session".into(),
            "main".into(),
        ];
    }
    _ => {
        opts.shell_opts = vec![agent.provider.clone()];
    }
}
```

> **Note:** If the current backend simply runs the shell (pwsh/bash) and lets the content scripts invoke the provider, no backend changes are needed. The `openclaw tui` command would go in the shell init sequence.

### 3c. Workspace initialization

AgentClaw needs a workspace directory on host. The forge engine should create it if missing:

```
~/.claw/agentclaw-workspace/
├── CLAUDE.md           # generated per-agent context file (Forge content tab)
└── .mcp.json           # MCP server config (optional — openclaw has its own)
```

Note: OpenClaw maintains its own state directory at `~/.openclaw/` (not inside the AgentMux workspace). The `agentclaw-workspace` is the working directory for file operations during coding tasks.

---

## 4. Forge UI Changes

### 4a. Provider badge

The Forge agent detail view shows a provider badge (e.g., "CLAUDE", "GEMINI"). Add:

```typescript
// In forge-view.tsx or wherever provider labels are rendered
const providerLabel = (provider: string) => {
    switch (provider) {
        case "claude":   return { label: "Claude Code", color: "#d97706" };
        case "gemini":   return { label: "Gemini CLI",  color: "#2563eb" };
        case "codex":    return { label: "Codex CLI",   color: "#16a34a" };
        case "openclaw": return { label: "OpenClaw",    color: "#7c3aed" };
        default:         return { label: provider,      color: "#6b7280" };
    }
};
```

### 4b. Create Agent form — provider dropdown

The "Provider" dropdown in the create/edit form should include OpenClaw:

```typescript
const PROVIDERS = [
    { value: "claude",   label: "Claude Code" },
    { value: "gemini",   label: "Gemini CLI" },
    { value: "codex",    label: "Codex CLI" },
    { value: "openclaw", label: "OpenClaw" },   // ← ADD
];
```

### 4c. AgentClaw in Forge agent list

AgentClaw will appear automatically in the Forge list once it's in the seed manifest. The 🦞 icon distinguishes it visually.

---

## 5. OpenClaw Features and AgentMux Integration

### 5a. External Messaging (Telegram, Discord, WhatsApp)

OpenClaw's gateway handles inbound messages from external channels and routes them to the agent session. The TUI pane in AgentMux shows this activity.

**Flow:**
1. User sends WhatsApp message → gateway receives
2. Gateway routes to `main` agent → LLM turn starts
3. Agent invokes skills (tool calls) to fulfill request
4. Agent replies to WhatsApp
5. **AgentClaw TUI pane** shows the full turn: message → tool calls → reply

**AgentMux adds:** AgentBus jekt to the same terminal when cloud events arrive (GitHub reviews, CI failures). Both paths (WhatsApp input + AgentBus jekt) land in the same AgentClaw pane.

**Gateway must be running** for any of this to work. When AgentClaw pane opens, the startup sequence checks gateway health first (see Section 2).

### 5b. Skill-Based Orchestration (the real sub-agent story)

OpenClaw's orchestration is **skill-driven**, not multi-agent. When a user request needs complex work:

```
User: "Get emails from Karen and make a presentation"
  ├── himalaya skill → IMAP fetch (tool call, inline)
  ├── gemini skill   → summarize emails (subprocess, returns text)
  └── coding-agent skill → spawn Claude Code headless
        openclaw agent → claude --print --permission-mode bypassPermissions
        (creates presentation file, returns when done)
```

**51 bundled skills** available; 8 ready on this system:
- `coding-agent` ✅ — spawns Claude Code/Codex/Pi for coding work
- `gemini` ✅ — one-shot Gemini Q&A
- `gh-issues` ✅ — GitHub issue management + sub-agent PR creation
- `github` ✅ — `gh` CLI operations
- `weather` ✅ — weather queries
- `skill-creator` ✅ — creates new skills
- `video-frames` ✅ — ffmpeg frame extraction
- `healthcheck` ✅ — system security checks

**AgentMux visibility for skill execution:** Currently none — skills run invisibly within the TUI session. Phase 3 could add a status bar showing the active skill name.

### 5c. ACP Sub-agent Surfacing (Phase 3)

The `coding-agent` skill spawns Claude Code or Codex as background subprocesses via ACP. These are headless (non-PTY) today — they run, complete, and report back.

**Future integration:** When a coding sub-agent runs via ACP, surface it as a new read-only AgentMux pane showing its output stream. This requires:
1. ACP bridge hook to emit a `CreateForgeAgentCommand` via AgentBus when a sub-agent starts
2. Backend creates an ephemeral Forge agent entry (`is_seeded: 0`, `is_ephemeral: 1`)
3. Pane auto-opens, streams output, auto-closes when sub-agent exits

**Design:** AgentClaw gateway webhook → AgentBus → backend → ephemeral pane.

**Later milestone** — not blocking for Phase 1/2.

### 5d. Memory and Context Engine

OpenClaw maintains persistent memory across sessions (vector search + compaction at `~/.openclaw/agents/main/memory/`). Compaction runs automatically based on config.

**AgentMux integration (Phase 3):**
- Forge content tab for OpenClaw agents shows memory summary (read-only)
- New content type: `memory` populated by `openclaw memory search --json`

### 5e. AgentBus Jekt Delivery (Cloud → AgentClaw)

Like all agents, AgentClaw receives jekts from cloud services (reagent reviews, CI failures, etc.). The `agent_bus_id: "agentclaw"` wires it into the existing agentbus-github-consumer mapping.

**Add to `agent-mapping.ts` in `a5af/agentbus`:**

```typescript
{ pattern: /^agentclaw-workflow\[bot\]$/, agentId: "agentclaw" },
{ pattern: /^AgentClaw-asaf$/,            agentId: "agentclaw" },
```

---

## 6. Relationship to Existing openclaw-widget.md Spec

These two specs are **complementary, not conflicting**:

| Aspect | openclaw-widget.md (existing) | openclaw-agent-runtime.md (this spec) |
|--------|-------------------------------|---------------------------------------|
| Entry point | Widget in launcher (like "New Tab") | Forge agent card (like AgentX/AgentZ) |
| View type | WebView embedding `localhost:18789` | Terminal TUI session (`openclaw tui`) |
| Use case | Configure channels, view conversation history, manage skills | Live interaction with the agent, see tool execution, receive jekts |
| Runtime | OpenClaw HTTP gateway (daemon) | `openclaw tui` → connects to the same gateway |
| Both run? | Yes — they coexist | Yes — both connect to the same gateway instance |

**The gateway is shared.** The widget and the TUI pane both talk to `localhost:18789`. A message sent via the TUI appears in the widget's history, and vice versa. They are two views of the same agent session.

**Combined UX flow:**
1. User opens OpenClaw widget → sees channel config, history, skills
2. User opens Forge → clicks AgentClaw → TUI pane opens, connects to gateway
3. WhatsApp message arrives → gateway processes → TUI pane shows the turn live
4. GitHub review jekt arrives → AgentBus delivers → TUI pane shows it as injected input

---

## 7. Implementation Phases

### Phase 1 — Seed Manifest (minimal, shippable now)

- [ ] Add AgentClaw + Agent4 entries to `forge-seed.json`
- [ ] Bump version to `3`
- [ ] Add `"openclaw"` to Forge UI provider dropdown
- [ ] Add OpenClaw provider badge color/label
- [ ] Add `agentclaw` to `agent-mapping.ts` in agentbus

**Result:** AgentClaw appears in Forge. Opening it launches a pwsh terminal. User runs `openclaw tui` manually until the startup sequence is refined.

### Phase 2 — Provider Launch Integration

- [ ] Add `"openclaw"` case to `shellexec.rs` spawn logic (runs `openclaw tui --session main`)
- [ ] Shell init sequence: gateway health check → start if needed → `openclaw tui`
- [ ] Auto-create `~/.claw/agentclaw-workspace/` on first launch

**Result:** AgentClaw auto-starts the TUI on pane open, same as claude/gemini auto-starts their REPLs.

### Phase 3 — Feature Integration

- [ ] ACP sub-agent surfacing — ephemeral panes for `coding-agent` spawned sub-agents
- [ ] OpenClaw memory display in Forge content tab
- [ ] Forge status indicator: active skill name during tool execution
- [ ] Link AgentClaw pane ↔ OpenClaw widget (same session, shared gateway)

---

## 8. Known Facts (from live system inspection)

- **Installed:** `openclaw v2026.3.8 (3caab92)` at `C:\Users\asafe\AppData\Roaming\npm\openclaw` — in `$PATH` ✅
- **Gateway running:** `localhost:18789` responds `{"ok":true,"status":"live"}` ✅; registered as Windows Scheduled Task ✅
- **API keys:** None needed separately. Configured providers are `amazon-bedrock` (uses existing AWS SDK credentials via `aws-sdk` auth) and `github-copilot`. Zero extra setup required. ✅
- **`openclaw tui`** is the correct interactive command for a terminal pane. Connects over WebSocket to the running gateway. Accepts user input, streams agent responses, shows history.
- **`openclaw agent --message "text"`** is one-shot only — runs one turn and exits. NOT for terminal panes.
- **Agent model:** Single agent identity (`main`) configured. `openclaw agents` manages routing profiles (mailboxes), not orchestration sub-agents.
- **Skill model:** 51 bundled skills; 8 ready on this system. Skills are tool calls within the LLM session. `coding-agent` skill spawns actual subprocesses (Claude Code/Codex/Pi) headlessly via ACP.
- **ACP:** Agent Control Protocol bridge. `openclaw acp client` runs an interactive ACP session. Used internally by `coding-agent` skill for sub-agent delegation.
- **Sessions:** `agent:main:main` — direct session, claude-opus-4-6, 9 days old.

## 9. Open Questions

1. **`openclaw tui` in container:** Does `openclaw tui` work correctly in a headless Linux container (no TTY allocated)? Or does Agent4 need a different entry command (e.g., `openclaw agent --message` in a loop)?
2. **Gateway in container:** The gateway is a Windows Scheduled Task on host. Container needs its own gateway instance. How should it be started in the container init sequence?
3. **Container image:** Should the Docker image for Agent4 include `openclaw` pre-installed, or installed on first run via `npm install -g openclaw`?
4. **ACP surfacing spec:** What events does the ACP bridge emit when a sub-agent starts/stops? Is there a webhook or WebSocket event from the gateway we can hook for ephemeral pane creation?
5. **Shell on host:** For non-Windows hosts (macOS/Linux), should the AgentClaw seed entry ship without a `shell` field and let the backend fall back to `$SHELL`?
6. **Multi-agent routing:** Should AgentMux surface OpenClaw's agent routing rules (which channels route to which agent identity) in the Forge UI? This would require reading `openclaw agents bindings`.

---

## Files to Change

| File | Change |
|------|--------|
| `agentmuxsrv-rs/forge-seed.json` | Add AgentClaw + Agent4, bump version to 3 |
| `agentmuxsrv-rs/src/backend/shellexec.rs` | Add `"openclaw"` match arm → `openclaw tui --session main` (Phase 2) |
| `frontend/app/view/forge/forge-view.tsx` | Add openclaw to provider dropdown + badge |
| `a5af/agentbus` → `consumers/github/agent-mapping.ts` | Add agentclaw identity mappings |
