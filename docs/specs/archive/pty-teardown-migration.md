# PTY Teardown & Subprocess Transport Migration

> Complete removal of PTY-based agent I/O. Replace with stateless subprocess invocations using Claude Code's `-p` mode and `--resume` for multi-turn continuity.

**Status:** Migration spec
**Date:** 2026-03-14
**Depends on:** `presentation-layer.md`, `subprocess-transport-impl.md`

---

## 1. Architecture Shift

### Current: Long-Running PTY Process

AgentMux runs each agent CLI inside a **PTY** (pseudo-terminal). The process stays alive for the entire session. User input is written as terminal keystrokes to PTY stdin. CLI output (NDJSON mixed with ANSI codes, shell prompts, and npm progress) is read from PTY stdout.

```
User msg 1 ──► base64 ──► PTY stdin ──► process handles it
User msg 2 ──► base64 ──► PTY stdin ──► same process handles it
User msg 3 ──► base64 ──► PTY stdin ──► same process handles it
                              │
                    process stays alive
                    throughout session
```

This requires:
- A shell (bash/pwsh/cmd) to host the PTY
- Bootstrap scripts to install and launch the CLI inside the shell
- Heredoc injection to write config files (`CLAUDE.md`, `.mcp.json`)
- Shell env var `export` commands
- A 500ms `setTimeout` hack waiting for the shell to be ready
- Cross-platform shell detection and syntax generation
- Filtering noisy terminal output from structured NDJSON

### New: Stateless Per-Turn Subprocess Invocations

Claude Code's `-p` flag runs in **non-interactive mode** — it accepts a JSON message on stdin, runs the full agentic loop (including tool calls), streams NDJSON output on stdout, then **exits**. It is one-shot by design.

Multi-turn conversation works via `--resume <session-id>`. Claude Code persists full conversation context in its local session store. On resume, it loads that context and continues as if the conversation never stopped.

```
User msg 1 ──► spawn `claude -p` ──► stream NDJSON ──► process exits
                                                          │
                                              capture session_id from
                                              system/init message
                                                          │
User msg 2 ──► spawn `claude -p --resume <sid>` ──► stream NDJSON ──► process exits
User msg 3 ──► spawn `claude -p --resume <sid>` ──► stream NDJSON ──► process exits
```

No PTY, no shell, no bootstrap, no long-running process. Each turn is a clean subprocess with piped stdin/stdout.

### Why This Is Better

| Concern | PTY (current) | Subprocess (new) |
|---------|---------------|-------------------|
| **Input** | Base64 text → PTY stdin | JSON → piped stdin |
| **Output** | NDJSON mixed with ANSI/shell noise | Clean NDJSON only |
| **Process lifecycle** | Long-running, must monitor health | One-shot, clean exit |
| **Crash recovery** | Context lost — must restart from scratch | `--resume` restores full context |
| **Shell dependency** | Requires bash/pwsh/cmd detection + bootstrap | No shell involved |
| **Cross-platform** | Different bootstrap per shell | `Command::new("claude")` everywhere |
| **Accumulated state** | Memory leaks, stale MCP connections | Fresh process every turn |
| **Config injection** | Heredoc injection into PTY stdin | `WriteAgentConfigCommand` RPC writes files directly |

### User Experience: Identical

Every user-visible interaction works the same way:

| User Action | PTY (current) | Subprocess (new) |
|------------|---------------|-------------------|
| Type message, hit send | base64 text → PTY stdin | JSON → spawn process stdin |
| See streaming response | NDJSON → WPS events → document view | NDJSON → WPS events → document view |
| See tool calls | StreamEvent → ToolBlock | StreamEvent → ToolBlock |
| Send follow-up | Text → same running PTY | JSON → new process with `--resume` |
| Stop/cancel | Signal to PTY process | SIGTERM to subprocess |
| Refresh browser | Replay from `.jsonl` file | Replay from `.jsonl` file |
| Recover from crash | Re-launch — **context lost** | `--resume` — **context preserved** |

The downstream pipeline (translator → parser → DocumentNode → component rendering) is completely unchanged. It consumes the same NDJSON events regardless of transport.

---

## 2. What Gets REMOVED

### 2.1 Frontend: Delete Entirely

| File | Why |
|------|-----|
| `bootstrap.ts` | Shell-specific bootstrap scripts (pwsh/cmd/bash). No shell exists in the new model. |

### 2.2 Frontend: Remove from `agent-model.ts`

| Symbol | What It Does | Why It's Gone |
|--------|-------------|---------------|
| `buildConfigPreamble()` | Writes config files via heredoc injection into PTY stdin | Replaced by `WriteAgentConfigCommand` RPC |
| `writeHeredoc()` | Helper for heredoc generation | No heredocs — files written atomically via backend |
| `shellQuote()` | Shell escaping for heredoc values | No shell |
| `shellEscape()` | Shell escaping for paths | No shell |
| `ControllerResyncCommand` calls | Spawns `ShellController` + PTY | Replaced by `SubprocessSpawnCommand` |
| `ControllerInputCommand` call (bootstrap) | Injects bootstrap script into PTY | Replaced by direct spawn with CLI args |
| `setTimeout(500)` hack | Waits for PTY shell to be ready | Subprocess spawn is immediate — no wait |
| `import { buildBootstrapScript, guessShellType }` | Bootstrap imports | No bootstrap |
| `import { stringToBase64 }` | Base64-encoding for PTY input | Subprocess uses JSON |

### 2.3 Frontend: Remove from Other Files

| File | What to Remove | Replacement |
|------|---------------|-------------|
| `useAgentStream.ts` | `const TermFileName = "term"` | Change to `"output"` |
| `useAgentStream.ts` | JSDoc references to "PTY output" | Update to "subprocess output" |
| `agent-view.tsx` | `ControllerInputCommand` for user input | `AgentInputCommand` (JSON) |
| `AgentFooter.tsx` | `stringToBase64(message)` + `ControllerInputCommand` | JSON message + `AgentInputCommand` |

### 2.4 Frontend: Concepts Eliminated

| Concept | Where | Why |
|---------|-------|-----|
| Shell type detection | `guessShellType()` | No shell involved |
| Platform-specific script generation | `buildPowerShellBootstrap()`, `buildCmdBootstrap()`, `buildBashBootstrap()` | `Command::new("claude")` is platform-agnostic |
| Heredoc config writing | `buildConfigPreamble()` → `writeHeredoc()` | `WriteAgentConfigCommand` writes files atomically |
| Shell env var `export` | `export KEY='VALUE'` lines in preamble | Env vars passed via `cmd.envs()` |
| `controller: "shell"` metadata | `SetMetaCommand` in launch functions | Changed to `controller: "subprocess"` |
| Base64-encoded text input | `stringToBase64(script + "\r")` → `ControllerInputCommand` | JSON messages via `AgentInputCommand` |
| Long-running process assumption | Input sent to existing process | Each turn spawns a new process |

### 2.5 Backend (Rust)

| Area | What to Remove | Why |
|------|---------------|-----|
| `ShellController` usage for agents | Agent blocks no longer use `ShellController` | Agents use `SubprocessController` |
| `ControllerInputCommand` for agents | Agent blocks no longer accept raw text input | Agent input uses `AgentInputCommand` |
| Shell detection for agents | `shell:type` block meta | No shell in subprocess model |

**Note:** `ShellController` is NOT deleted. It stays for regular terminal panes (`view: "term"`). Only its use for agent panes (`view: "agent"`) is removed.

### 2.6 Specs to Archive

| Spec | Reason |
|------|--------|
| `cross-shell-bootstrap.md` | Bootstrap concept eliminated entirely. Move to `specs/archive/`. |
| `agent-pane-terminal-switch.md` | PTY-based agent launch superseded. Move to `specs/archive/`. |

---

## 3. What Gets ADDED

### 3.1 Backend: SubprocessController

New file: `blockcontroller/subprocess.rs`

Manages per-turn subprocess lifecycle. Unlike `ShellController` which maintains a long-running PTY, `SubprocessController` spawns a fresh process for each user turn and waits for it to exit.

Key behaviors:
- **First turn:** Spawn `claude -p --input-format stream-json --output-format stream-json`
- **Subsequent turns:** Spawn with `--resume <session-id>` appended
- **Each spawn:** Write one JSON user message to stdin, close stdin, read NDJSON from stdout until exit
- **Session capture:** Extract `session_id` from the first `system/init` message, store in block metadata

### 3.2 Backend: stdout_reader + process_waiter

New file: `blockcontroller/subprocess_reader.rs`

- `stdout_reader` — reads NDJSON lines from piped stdout, persists to `.jsonl`, publishes WPS `blockfile` events on `"output"` subject
- `process_waiter` — waits for process exit, publishes lifecycle event, updates block state

No `stdin_writer` task needed in the per-turn model — stdin receives one JSON message at spawn time, then is closed.

### 3.3 Backend: New RPC Commands

| Command | Route | Purpose |
|---------|-------|---------|
| `SubprocessSpawnCommand` | `subprocessspawn` | Spawn agent CLI for a single turn. Accepts `blockid`, `cli_command`, `cli_args`, `working_dir`, `env_vars`, `message` (the user's JSON message). |
| `AgentInputCommand` | `agentinput` | Send a follow-up message. Triggers a new spawn with `--resume`. Accepts `blockid` + `message` (JSON string). |
| `AgentStopCommand` | `agentstop` | Send SIGTERM/SIGKILL to running subprocess. Accepts `blockid` + `force` flag. |
| `WriteAgentConfigCommand` | `writeagentconfig` | Write config files (`CLAUDE.md`, `.mcp.json`) atomically to agent working directory. Replaces heredoc injection. |

**Note on `AgentInputCommand`:** In the per-turn model, this command doesn't write to an existing stdin pipe. It spawns a **new** subprocess with `--resume <session-id>` and the new message. The frontend doesn't need to know this — it sends `AgentInputCommand` for every user message and the backend handles whether to spawn fresh or resume.

### 3.4 Backend: Controller Registration

```rust
// blockcontroller/mod.rs
match controller_type {
    "shell" => ShellController::new(block_id, ...),
    "subprocess" => SubprocessController::new(block_id, ...),
    _ => return Err(...)
}
```

### 3.5 Frontend: Updated Launch Path

| File | Changes |
|------|---------|
| `agent-model.ts` | Rewrite `launchAgent()` and `launchForgeAgent()`: write config via `WriteAgentConfigCommand`, spawn via `SubprocessSpawnCommand`, set `controller: "subprocess"` in block meta. No shell, no bootstrap, no setTimeout. |
| `useAgentStream.ts` | Subscribe to `getFileSubject(blockId, "output")` instead of `"term"`. Pipeline unchanged. |
| `agent-view.tsx` / `AgentFooter.tsx` | User input sends JSON via `AgentInputCommand` instead of base64 via `ControllerInputCommand`. |
| `state.ts` | `rawOutputAtom` becomes optional/debug-only. |

### 3.6 Frontend: New RPC Client Bindings

```typescript
// wshclientapi.ts
SubprocessSpawnCommand(client, { blockid, cli_command, cli_args, working_dir, env_vars, message })
AgentInputCommand(client, { blockid, message })
AgentStopCommand(client, { blockid, force })
WriteAgentConfigCommand(client, { agent_id, files: [{ path, content }] })
```

---

## 4. What Stays AS-IS

These components require **zero changes**. They are transport-agnostic — they consume the same NDJSON events regardless of whether the source is a PTY or a subprocess.

### 4.1 Frontend Pipeline

| Component | File | Why |
|-----------|------|-----|
| Translator layer | `providers/translator.ts`, `claude-translator.ts`, `codex-translator.ts`, `gemini-translator.ts`, `translator-factory.ts` | Consumes raw JSON events, emits `StreamEvent[]`. Format is identical. |
| Stream parser | `stream-parser.ts` | Converts `StreamEvent` → `DocumentNode`. Input-agnostic. |
| Type definitions | `types.ts` | `DocumentNode`, `StreamEvent`, `StreamingState` — transport-agnostic. |
| State management | `state.ts` | `createAgentAtoms()` and signals — transport-agnostic. |
| Provider registry | `providers/index.ts` | Provider definitions (CLI command, args). Consumer changes, definitions don't. |

### 4.2 Frontend Components

All rendering components are transport-agnostic:

`AgentDocumentView`, `MarkdownBlock`, `ToolBlock`, `DiffViewer`, `BashOutputViewer`, `AgentMessageBlock`, `AgentHeader`, `FilterControls`, `ProcessControls`, `ConnectionStatus`, `InitializationPrompt`, `SetupWizard`

### 4.3 Backend

| Component | Why |
|-----------|-----|
| WPS pub/sub broker (`wps.rs`) | Same event mechanism — just `"output"` subject instead of `"term"`. |
| FileStore (SQLite) | Same `.jsonl` persistence for reconnection/replay. |
| `ShellController` | Stays for regular terminal panes (`view: "term"`). |
| Forge RPC commands | Agent discovery and content loading are unchanged. |

### 4.4 Tests

| Test | File |
|------|------|
| Provider index tests | `providers/index.test.ts` |
| State tests | `state.test.ts` |

---

## 5. File-by-File Change List

### DELETE

| File | Action |
|------|--------|
| `frontend/app/view/agent/bootstrap.ts` | Delete entirely |

### MODIFY

| File | Changes |
|------|---------|
| `frontend/app/view/agent/agent-model.ts` | Remove: `buildConfigPreamble()`, `writeHeredoc()`, `shellQuote()`, `shellEscape()`, all bootstrap/PTY imports, `setTimeout(500)` hack. Rewrite: `launchAgent()` and `launchForgeAgent()` to use `WriteAgentConfigCommand` + `SubprocessSpawnCommand`. Change `controller: "shell"` → `"subprocess"`. |
| `frontend/app/view/agent/useAgentStream.ts` | Change `TermFileName` from `"term"` to `"output"`. Update JSDoc. |
| `frontend/app/view/agent/agent-view.tsx` | User input via `AgentInputCommand` (JSON) instead of `ControllerInputCommand` (base64). |
| `frontend/app/view/agent/components/AgentFooter.tsx` | `handleSendMessage` formats JSON and calls `AgentInputCommand`. |
| `frontend/app/store/wshclientapi.ts` | Add bindings for new RPC commands. |
| `agentmuxsrv-rs/src/backend/blockcontroller/mod.rs` | Register `SubprocessController` alongside `ShellController`. |

### CREATE

| File | Purpose |
|------|---------|
| `agentmuxsrv-rs/src/backend/blockcontroller/subprocess.rs` | `SubprocessController` — per-turn spawn, session tracking, process lifecycle. |
| `agentmuxsrv-rs/src/backend/blockcontroller/subprocess_reader.rs` | `stdout_reader` + `process_waiter` async tasks. |
| Server route handlers | `subprocessspawn`, `agentinput`, `agentstop`, `writeagentconfig`. |

### ARCHIVE

| File | Action |
|------|--------|
| `specs/cross-shell-bootstrap.md` | Move to `specs/archive/` |
| `specs/agent-pane-terminal-switch.md` | Move to `specs/archive/` |

### NO CHANGES

| File | Why |
|------|-----|
| `providers/translator.ts` | Transport-agnostic interface |
| `providers/claude-translator.ts` | Same JSON format |
| `providers/codex-translator.ts` | Same JSON format |
| `providers/gemini-translator.ts` | Same JSON format |
| `providers/translator-factory.ts` | Dispatches on output format, not transport |
| `providers/index.ts` | Provider definitions unchanged |
| `stream-parser.ts` | StreamEvent → DocumentNode regardless of source |
| `types.ts` | Type definitions are transport-agnostic |
| `state.ts` | Signal factory is transport-agnostic |
| `init-monitor.ts` | Monitors CLI initialization prompts |
| `api-client.ts` | API client |
| `components/*.tsx` | Rendering is transport-agnostic |
| `agent-view.scss` | Styles are transport-agnostic |
| `providers/index.test.ts` | Tests provider definitions |
| `state.test.ts` | Tests signal factory |

---

## 6. Migration Sequence

### Phase 1: Backend SubprocessController (P0)

1. Create `subprocess.rs` with `SubprocessController` struct
2. Implement per-turn `spawn()` — pipe stdin/stdout, write user message, close stdin
3. Implement `stdout_reader` task — read NDJSON lines, persist to `.jsonl`, publish WPS events on `"output"` subject
4. Implement `process_waiter` task — wait for exit, update block state
5. Capture `session_id` from `system/init` message, store as `agent:sessionid` block meta
6. Register in `blockcontroller/mod.rs`

### Phase 2: Backend RPC Commands (P0)

1. `SubprocessSpawnCommand` — first turn: spawn `claude -p`, write message to stdin
2. `AgentInputCommand` — follow-up turns: spawn `claude -p --resume <sid>`, write message to stdin
3. `WriteAgentConfigCommand` — write config files atomically to working directory
4. `AgentStopCommand` — SIGTERM/SIGKILL to running subprocess

### Phase 3: Frontend Launch Path (P0)

1. Rewrite `launchAgent()` to use `WriteAgentConfigCommand` + `SubprocessSpawnCommand`
2. Rewrite `launchForgeAgent()` similarly
3. Update `useAgentStream.ts` to subscribe to `"output"` subject
4. Update `AgentFooter` to use `AgentInputCommand` for user input
5. Delete `bootstrap.ts`
6. Remove all PTY helpers from `agent-model.ts`

### Phase 4: Cleanup (P1)

1. Archive superseded specs (`cross-shell-bootstrap.md`, `agent-pane-terminal-switch.md`)
2. Remove dead PTY references from comments/docs
3. Update `CLAUDE.md` / `README.md` if they reference PTY bootstrapping
4. Remove `init-monitor.ts` if no longer needed (interactive prompts don't exist in `-p` mode)

---

## 7. Verification

```bash
# Verify single-turn subprocess transport:
echo '{"type":"user","message":{"role":"user","content":[{"type":"text","text":"What is 2+2?"}]}}' | \
  claude -p --input-format stream-json --output-format stream-json --verbose

# Expected: clean NDJSON on stdout, no ANSI codes, no shell artifacts
# {"type":"system","subtype":"init","session_id":"550e8400-...",...}
# {"type":"stream_event","event":{"type":"message_start",...},...}
# ...
# {"type":"result","result":"4",...}
# Process exits with code 0

# Verify multi-turn with resume:
# (capture session_id from first run, then:)
echo '{"type":"user","message":{"role":"user","content":[{"type":"text","text":"Now multiply that by 3"}]}}' | \
  claude -p --input-format stream-json --output-format stream-json --resume 550e8400-...

# Expected: Claude has full context from turn 1, responds "12"
```

### Manual Test Plan

1. Click agent → subprocess spawns (no PTY, no bootstrap script, no shell)
2. Type prompt → JSON sent via `SubprocessSpawnCommand` → structured output renders in document view
3. Process exits after response completes → UI remains in ready state
4. Send follow-up → `AgentInputCommand` triggers new spawn with `--resume` → context preserved
5. Refresh browser → document replays from `.jsonl` file
6. Kill subprocess externally mid-response → UI shows interrupted state → send new message resumes session
7. Regular terminal panes (`view: "term"`) still work via `ShellController` (regression check)
