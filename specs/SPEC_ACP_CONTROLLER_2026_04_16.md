# SPEC: ACP Controller — Universal Agent Client Protocol Support

**Date:** 2026-04-16
**Author:** Agent1
**Status:** Draft
**Priority:** High — strategic differentiator
**Supersedes:** `docs/specs/openclaw-agent-runtime.md` (TUI/PTY approach)
**Related:** `docs/specs/integration-vision.md`, `docs/specs/openclaw-widget.md`

---

## Summary

Add a new `acp` controller type to AgentMux that speaks the [Agent Client Protocol](https://github.com/agentclientprotocol/agent-client-protocol) (ACP), enabling AgentMux to host **any ACP-compatible coding agent** with zero per-agent integration work.

ACP is a JSON-RPC 2.0 protocol over stdio — the "LSP for AI agents." AgentMux currently has custom translators for each provider (Claude, Codex, Gemini). An ACP controller replaces this N-integrations problem with a single universal protocol layer.

### Why ACP Instead of TUI/PTY

The prior spec (`openclaw-agent-runtime.md`) proposed running `openclaw tui` in a PTY terminal pane — the same pattern used for Claude/Codex/Gemini today. This works but has fundamental limitations:

| PTY Approach (Prior Spec) | ACP Approach (This Spec) |
|---------------------------|--------------------------|
| Scrape terminal output, parse ANSI | Structured JSON-RPC messages |
| Per-agent output format parsing | One universal protocol |
| Only works for OpenClaw | Works for any ACP agent |
| No programmatic tool call visibility | First-class tool call events |
| Session management via CLI flags | Protocol-native sessions |
| Gateway must be running first | ACP process is self-contained |

The TUI approach remains valid as a fallback for agents without ACP support, but ACP is the strategic direction. The `openclaw-widget.md` spec (WebView dashboard) is unaffected — the widget and ACP controller serve different purposes and coexist.

---

## Motivation

### Current State

Each agent provider requires:
- A static `ProviderConfig` in Rust (`providers.rs`)
- A `ProviderDefinition` in TypeScript (`providers/index.ts`)
- A custom output translator (`claude-translator.ts`, etc.)
- Custom launch args, auth handling, resume logic
- CEF-layer detection and installation code

**Adding one agent = 5-7 files changed.** This doesn't scale.

### With ACP

Any agent that speaks ACP works automatically:
- One `AcpController` handles all ACP agents
- One `AcpTranslator` converts ACP events to `StreamEvent`
- New agents = config entry only (CLI command + args)

### Industry Adoption

| Agent | ACP Support | Method |
|-------|------------|--------|
| Gemini CLI | Native | `gemini --acp` |
| Kiro CLI | Native | Built-in |
| OpenClaw | Native | Via `acpx` / built-in ACP bridge |
| Claude Code | Community bridge | `claude-agent-acp` (Zed), `claude-code-acp` (PyPI) |
| Codex CLI | Community bridge | `codex-acp` (Zed, cola-io) |
| GitHub Copilot | Public preview | Native |
| Augment Code | Native | Built-in |
| Goose | Native | Built-in |
| Junie (JetBrains) | Native | Built-in |
| Cline | Native | Built-in |

Claude Code and Codex have open feature requests for native ACP (#6686 and #9085 respectively). Native support is likely coming.

---

## ACP Protocol Overview

### Transport

- **Wire format:** JSON-RPC 2.0 over stdin/stdout
- **Lifecycle:** Client spawns agent process, communicates via stdio
- **Framing:** Newline-delimited JSON (NDJSON)

### Core Flow

```
Client (AgentMux)                    Agent (e.g., gemini --acp)
       |                                      |
       |--- initialize ---------------------->|
       |<-- initialize result ----------------|
       |--- initialized notification -------->|
       |                                      |
       |--- session/create ------------------>|
       |<-- session id -----------------------|
       |                                      |
       |--- session/prompt ------------------>|
       |<-- session/update (streaming) -------|  (multiple)
       |<-- session/update (streaming) -------|
       |<-- session/prompt result ------------|
       |                                      |
       |--- session/prompt (turn 2) --------->|
       |<-- session/update (streaming) -------|
       |<-- session/prompt result ------------|
       |                                      |
       |--- shutdown ------------------------>|
       |<-- shutdown result ------------------|
       |--- exit ---------------------------->|
```

### Key Message Types

#### Initialize

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "initialize",
  "params": {
    "clientInfo": {
      "name": "AgentMux",
      "version": "0.33.0"
    },
    "capabilities": {
      "tools": true,
      "fileAccess": true
    },
    "workspaceRoots": ["/path/to/project"]
  }
}
```

#### Session Create

```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "method": "session/create",
  "params": {
    "cwd": "/path/to/project"
  }
}
```

#### Session Prompt

```json
{
  "jsonrpc": "2.0",
  "id": 3,
  "method": "session/prompt",
  "params": {
    "sessionId": "uuid",
    "prompt": {
      "type": "text",
      "text": "Fix the failing test in auth.test.ts"
    }
  }
}
```

#### Session Update (Streaming Notification)

```json
{
  "jsonrpc": "2.0",
  "method": "session/update",
  "params": {
    "sessionId": "uuid",
    "type": "agent_message_chunk",
    "content": "I'll look at the test file..."
  }
}
```

Other update types: `tool_call`, `tool_result`, `agent_thought_chunk`

#### Prompt Result

```json
{
  "jsonrpc": "2.0",
  "id": 3,
  "result": {
    "stopReason": "end_turn",
    "usage": {
      "inputTokens": 1200,
      "outputTokens": 450
    }
  }
}
```

---

## Architecture

### New Controller: `AcpController`

```
┌─────────────────────────────────────────────────┐
│                  AgentMux                        │
│                                                  │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐      │
│  │ Persistent│  │Subprocess│  │   ACP    │ NEW  │
│  │Controller │  │Controller│  │Controller│◄─────│
│  └─────┬─────┘  └─────┬────┘  └─────┬────┘      │
│        │               │             │           │
│   stdin/stdout    per-turn      JSON-RPC 2.0     │
│   raw stream      spawn        over stdio        │
│        │               │             │           │
│  ┌─────┴─────┐  ┌─────┴────┐  ┌─────┴────┐      │
│  │  Claude   │  │  Codex   │  │ Any ACP  │      │
│  │  (native) │  │  (native)│  │  Agent   │      │
│  └───────────┘  └──────────┘  └──────────┘      │
└─────────────────────────────────────────────────┘
```

The ACP controller is a **persistent** process — one spawn per agent pane, multiple turns via `session/prompt`. No need for `--resume` flags or session ID parsing from output.

### Backend (Rust)

#### `agentmux-srv/src/backend/blockcontroller/acp.rs` (NEW)

```rust
pub struct AcpController {
    session_id: Option<String>,
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    stdout_reader: Option<BufReader<ChildStdout>>,
    next_rpc_id: AtomicU64,
}

impl AcpController {
    /// Spawn the ACP agent process and perform initialize handshake
    pub async fn start(&mut self, cmd: &str, args: &[&str], cwd: &str) -> Result<()> {
        // 1. Spawn process with stdin/stdout pipes
        // 2. Send initialize request
        // 3. Wait for initialize result
        // 4. Send initialized notification
        // 5. Create session
        // 6. Store session_id
    }

    /// Send a user prompt and stream back updates
    pub async fn prompt(&mut self, text: &str) -> Result<PromptStream> {
        // 1. Send session/prompt request
        // 2. Return stream that yields session/update notifications
        // 3. Complete when prompt result arrives
    }

    /// Graceful shutdown
    pub async fn stop(&mut self) -> Result<()> {
        // 1. Send shutdown request
        // 2. Send exit notification
        // 3. Wait for process to exit
    }
}
```

#### `agentmux-srv/src/backend/blockcontroller/mod.rs` (MODIFIED)

Add `Acp` variant to controller type enum:

```rust
pub enum ControllerType {
    Persistent,
    Subprocess,
    Acp,          // NEW
}
```

Route to `AcpController` when block metadata has `controller: "acp"`.

### Frontend (TypeScript)

#### `frontend/app/view/agent/providers/acp-translator.ts` (NEW)

Single translator that handles all ACP agents:

```typescript
export class AcpTranslator implements OutputTranslator {
    translate(rawEvent: AcpSessionUpdate): StreamEvent[] {
        switch (rawEvent.type) {
            case "agent_message_chunk":
                return [{ type: "text", content: rawEvent.content }];

            case "agent_thought_chunk":
                return [{ type: "thinking", content: rawEvent.content }];

            case "tool_call":
                return [{
                    type: "tool_use",
                    id: rawEvent.toolCallId,
                    name: rawEvent.toolName,
                    input: rawEvent.input,
                }];

            case "tool_result":
                return [{
                    type: "tool_result",
                    id: rawEvent.toolCallId,
                    content: rawEvent.content,
                }];

            default:
                return [];
        }
    }

    reset(): void { /* no state to reset */ }
}
```

#### `frontend/app/view/agent/providers/index.ts` (MODIFIED)

Add ACP provider definitions — these are lightweight since the protocol handles everything:

```typescript
// ACP agents only need: id, displayName, cliCommand, acp flag + args
// OpenClaw — full-featured, gateway daemon backed (detected if preinstalled)
openclaw: {
    id: "openclaw",
    displayName: "OpenClaw",
    cliCommand: "acpx",
    controllerType: "acp",
    launchArgs: ["--agent", "openclaw"],
    styledOutputFormat: "acp",
    icon: "lobster",
    docsUrl: "https://docs.openclaw.ai",
    npmPackage: "@openclaw/acpx",
    // ... auth fields
},

// Pi — lightweight standalone coding agent, no gateway required
pi: {
    id: "pi",
    displayName: "Pi",
    cliCommand: "pi",
    controllerType: "acp",
    launchArgs: ["--json"],
    styledOutputFormat: "acp",
    icon: "terminal",
    docsUrl: "https://github.com/badlogic/pi-mono",
    npmPackage: "@mariozechner/pi-coding-agent",
    // ... auth fields
},

kiro: {
    id: "kiro",
    displayName: "Kiro CLI",
    cliCommand: "kiro",
    controllerType: "acp",
    launchArgs: ["--acp"],
    styledOutputFormat: "acp",
    icon: "kiro",
    docsUrl: "https://kiro.dev/docs/cli/acp/",
    npmPackage: "@anthropic-ai/kiro",
    // ... auth fields
},
```

#### `frontend/app/view/agent/providers/translator-factory.ts` (MODIFIED)

```typescript
case "acp":
    return new AcpTranslator();
```

### Provider Config Changes

#### `agentmux-srv/src/backend/providers.rs` (MODIFIED)

Add `ControllerType::Acp` and ACP-specific provider entries:

```rust
pub enum ControllerType {
    Persistent,
    Subprocess,
    Acp,
}

// OpenClaw — gateway daemon backed (full features: skills, messaging, multi-agent)
static OPENCLAW: ProviderConfig = ProviderConfig {
    id: "openclaw",
    display_name: "OpenClaw",
    cli_command: "acpx",
    controller_type: ControllerType::Acp,
    launch_args: &["--agent", "openclaw"],
    resume_flag: None,
    session_id_field: "sessionId",
    styled_output_format: "acp",
    // ...
};

// Pi — standalone coding agent (no gateway, pure read/write/bash/edit tools)
static PI: ProviderConfig = ProviderConfig {
    id: "pi",
    display_name: "Pi",
    cli_command: "pi",
    controller_type: ControllerType::Acp,
    launch_args: &["--json"],
    resume_flag: None,
    session_id_field: "sessionId",
    styled_output_format: "acp",
    // ...
};
```

---

## Relationship to Existing Specs

### `docs/specs/integration-vision.md`

The integration vision already identified three messaging layers including ACP sub-agents, OpenClaw as a first-class runtime, and the ContextEngine interface. This spec implements the ACP layer as a universal controller rather than an OpenClaw-specific feature.

Key items from the vision that this spec enables:
- **"Agent spawning/streaming — OpenClaw ACP"** → generalized to any ACP agent
- **"OpenClaw replaces a5af/claw"** → still true, but via ACP not TUI
- **Context engine integration** → deferred to a separate spec, not blocked by controller choice

### `docs/specs/openclaw-agent-runtime.md`

This spec **supersedes** the TUI/PTY approach for OpenClaw. Specifically:
- `forge-seed.json` entries (AgentClaw, Agent4) → still needed, but `provider: "openclaw"` now routes to ACP controller
- `shellexec.rs` changes → not needed; ACP controller handles spawn
- Phase 3 "ACP sub-agent surfacing" → becomes Phase 1 since we're ACP-native from the start

### `docs/specs/openclaw-widget.md`

**Unaffected.** The widget (WebView at `localhost:18789`) and ACP controller serve different purposes:
- Widget = dashboard/config UI for OpenClaw's gateway, channels, skills
- ACP controller = structured agent interaction in an agent pane

Both coexist and can connect to the same OpenClaw gateway instance.

---

## Migration Path

ACP doesn't replace existing controllers — it runs alongside them. Existing providers keep working as-is.

### Phase 1: ACP Controller + OpenClaw + Pi (This Spec)

- Implement `AcpController` in Rust
- Implement `AcpTranslator` in TypeScript
- Add **two** ACP providers from the OpenClaw ecosystem:
  - **OpenClaw** — full-featured agent orchestrator backed by the OpenClaw gateway daemon.
    Detected if `openclaw` is installed and gateway is running at `ws://127.0.0.1:18789`.
    Uses `acpx` (`@openclaw/acpx`) as ACP bridge. Provides skills, external messaging
    channels, memory, multi-agent orchestration.
  - **Pi** — lightweight standalone coding agent (`@mariozechner/pi-coding-agent`).
    No gateway required. Pure coding agent with read/write/bash/edit tools.
    Ideal for users who want a fast, self-contained coding agent without the full OpenClaw stack.
- Add Kiro CLI as third ACP provider
- Update `forge-seed.json` with AgentClaw entries (from openclaw-agent-runtime.md)

### Phase 2: Migrate Gemini to ACP

Gemini CLI has native ACP (`gemini --acp`). Switch Gemini from `Subprocess` controller to `Acp`:
- Remove `gemini-translator.ts`
- Remove Gemini-specific launch args / resume logic
- Change `controllerType: "acp"`, `launchArgs: ["--acp"]`
- Validate feature parity

### Phase 3: Migrate Claude + Codex to ACP (When Native Support Ships)

Once Claude Code and Codex CLI ship native ACP:
- Switch to ACP controller
- Remove custom translators
- Simplify provider configs

At that point, all providers use the same controller and translator. Adding a new agent = one config entry.

### Phase 4: ACP Agent Discovery + Sub-Agent Surfacing

- Scan PATH for known ACP-compatible CLIs
- Auto-populate agent picker with discovered agents
- Allow user-defined ACP agents via Forge config
- Surface ACP sub-agents (e.g., OpenClaw's `coding-agent` skill spawning Claude Code) as ephemeral panes

---

## Files Changed

### New Files

| File | Purpose |
|------|---------|
| `agentmux-srv/src/backend/blockcontroller/acp.rs` | ACP controller (Rust) — JSON-RPC client over stdio |
| `frontend/app/view/agent/providers/acp-translator.ts` | Universal ACP event translator |
| `frontend/app/view/agent/commands/providers/openclaw.ts` | OpenClaw slash commands (initially empty) |

### Modified Files

| File | Change |
|------|--------|
| `agentmux-srv/src/backend/blockcontroller/mod.rs` | Add `Acp` variant, route to `AcpController` |
| `agentmux-srv/src/backend/providers.rs` | Add `ControllerType::Acp`, add OpenClaw + Pi + Kiro providers |
| `agentmux-cef/src/commands/providers.rs` | Add OpenClaw + Pi + Kiro npm packages, versions, auth checks |
| `frontend/app/view/agent/providers/index.ts` | Add OpenClaw + Pi + Kiro `ProviderDefinition` entries |
| `frontend/app/view/agent/providers/translator-factory.ts` | Add `"acp"` case returning `AcpTranslator` |
| `frontend/app/view/agent/commands/providers/index.ts` | Register OpenClaw slash commands |
| `frontend/app/view/forge/forge-constants.ts` | Add OpenClaw + Pi + Kiro to Forge provider list |

---

## Dependencies

### Rust

- `serde_json` — already in use for JSON parsing
- `tokio` — already in use for async I/O
- No new crates needed; JSON-RPC 2.0 is simple enough to implement inline

### TypeScript

- No new dependencies; ACP messages are plain JSON objects

### External

- `acpx` (npm: `@openclaw/acpx`) — for OpenClaw ACP bridge (gateway-backed)
- `pi` (npm: `@mariozechner/pi-coding-agent`) — standalone coding agent (no gateway needed)
- `kiro` (npm: `@anthropic-ai/kiro`) — for Kiro CLI
- `gemini` already installed — just needs `--acp` flag in Phase 2

---

## ACP vs Custom Translators — Comparison

| Aspect | Custom Translators (Current) | ACP Controller (Proposed) |
|--------|------------------------------|---------------------------|
| New agent effort | 5-7 files, custom translator | 1 config entry |
| Output format | Different per agent | Standardized JSON-RPC |
| Session management | Per-agent (resume flags, session IDs) | Protocol-native |
| Streaming | Custom NDJSON parsing per agent | Standardized `session/update` |
| Tool calls | Different schemas per agent | Standardized `tool_call` / `tool_result` |
| Auth | Custom per agent | Still per agent (ACP doesn't cover auth) |
| Thinking/reasoning | Custom per agent | Standardized `agent_thought_chunk` |
| Multi-turn | Custom (resume flag vs persistent stdin) | Protocol-native (same session) |

---

## Risks & Mitigations

| Risk | Mitigation |
|------|------------|
| ACP spec is still evolving | Pin to a specific ACP version; abstract behind internal types |
| Claude/Codex may never ship native ACP | Community bridges exist today; custom controllers remain as fallback |
| ACP agents may have different capability levels | Capability negotiation is part of the `initialize` handshake |
| Performance overhead of JSON-RPC vs raw streams | Negligible — we're already parsing JSON streams |

---

## Success Criteria

- [ ] `AcpController` can spawn, initialize, and prompt an ACP agent
- [ ] Streaming `session/update` events render in real-time in agent pane
- [ ] Multi-turn conversations work within a single ACP session
- [ ] OpenClaw works as ACP agent in AgentMux (gateway daemon + acpx)
- [ ] Pi works as standalone ACP coding agent (no gateway needed)
- [ ] Kiro CLI works as ACP agent
- [ ] Tool calls display correctly in agent pane
- [ ] Graceful shutdown on pane close
- [ ] No regressions on existing Claude/Codex/Gemini providers

---

## Landing Page Impact

This is a headline feature for agentmux.ai:

> **"Any Agent. One Protocol."**
> AgentMux now supports the Agent Client Protocol — connect any ACP-compatible coding agent with zero configuration. OpenClaw, Pi, Kiro, Gemini, and more work out of the box.

Update the comparison table to show ACP support as a differentiator no competitor has.

---

## References

- [Agent Client Protocol Specification](https://github.com/agentclientprotocol/agent-client-protocol)
- [ACP Compatible Agents](https://agentclientprotocol.com/get-started/agents)
- [Gemini CLI ACP Mode](https://geminicli.com/docs/cli/acp-mode/)
- [Kiro CLI ACP Docs](https://kiro.dev/docs/cli/acp/)
- [OpenClaw acpx](https://github.com/openclaw/acpx)
- [Claude Code ACP Request #6686](https://github.com/anthropics/claude-code/issues/6686)
- [Codex CLI ACP Request #9085](https://github.com/openai/codex/issues/9085)
- [Zed External Agents (ACP)](https://zed.dev/docs/ai/external-agents)
- [JetBrains ACP Agent Registry](https://blog.jetbrains.com/ai/2026/01/acp-agent-registry/)
