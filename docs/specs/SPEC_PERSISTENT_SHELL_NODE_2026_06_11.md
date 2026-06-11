# SPEC: Persistent Shell Node in the Agent Pane

**Date:** 2026-06-11  
**Status:** Draft  
**Scope:** New `ShellNode` document node type + `PersistentShellBlock` UI component + `agentmux-mcp` `Shell` tool

---

## 1. Problem

Long-running processes (build systems, dev servers, watchers, REPL sessions) cannot be represented in the agent conversation today. The `Bash` tool — intercepted by `agentmux-bashwrap` — blocks the entire tool call until the process exits. If the process never exits (e.g. `task dev`, `npm run watch`), the tool call hangs indefinitely and the agent is frozen.

Two downstream failures:

1. **Build progress is invisible.** The agent cannot stream compilation output live to the user while also continuing the conversation.  
2. **No persistent session.** Each Bash call is isolated — no PTY state, no environment carry-over, no way to send input to a running process.

The existing `term` widget (xterm.js) solves this for *manually-operated* terminals, but agents cannot open or interact with a terminal widget programmatically.

---

## 2. Goals

- Agents can launch long-running processes that appear as a **compact, colored, named row** in the conversation document.
- The row expands inline (click-to-reveal) to show the full **live-log** — the same streaming chunk renderer already used by `ToolBlock`.
- The agent can continue the conversation immediately after launching; the process runs independently.
- Processes can be **stopped** (agent-initiated or user-initiated via the UI).
- The representation is **efficient**: one row per shell in the DOM regardless of how much output has streamed.
- Color encodes status at a glance without expanding.

---

## 3. Non-Goals

- Full interactive PTY (keyboard input from the agent pane UI). The shell row is **read-only** from the UI; the agent can send input via a follow-up tool call.
- Replacing the existing `term` widget for human-operated terminals.
- Persistence across app restarts (session-scoped only, same as ToolStreamingLog today).

---

## 4. User-Visible Behavior

### 4.1 Collapsed state (default)

```
┌─────────────────────────────────────────────────────────────┐
│ ⟩  task dev                    [running 0:42]  ↳ Compiling… │
└─────────────────────────────────────────────────────────────┘
```

- **Left border**: colored by status (§6.3).
- **Icon** `⟩`: shell sigil (distinct from tool status icons `⏳ ✓ ✗`).
- **Label**: command or user-supplied `title` prop.
- **Elapsed timer**: counts up from spawn while running; freezes on exit.
- **Live-tail**: last output line, same mechanic as `ToolBlock`'s live-tail.
- **Stop button** `■`: visible on hover while status is `running`; sends SIGTERM.

### 4.2 Expanded state (click to toggle)

Inline panel below the row — identical layout to `ToolBlockOverlay`:
- Header: command + exit code (if done) + timestamp.
- Scrollable log body: `ToolOverlayLog`-compatible chunk list (stdout in foreground, stderr in red, system lines in muted italic).
- Cap: same `MAX_TOOL_OUTPUT_LINES = 1000` rendering cap, `OutputHiddenMarker` for overflow.
- No action bar (no bookmark, no open-in-pane, Phase 2 concern).

### 4.3 Terminal states

| Exit | Border color | Icon | Tail |
|------|--------------|------|------|
| Running | `--term-bright-green` | `⟩` | last line |
| Exited 0 | `--accent-color` (dim) | `✓` | last line |
| Exited non-0 | `--error-color` | `✗` | last line |
| Stopped by agent/user | `--secondary-text-color` | `■` | last line |

---

## 5. Architecture

### 5.1 Overview

```
Agent calls Shell(cmd, cwd?, title?)
        │
        ▼
agentmux-mcp (new binary)
  ├─ POST /agentmux/shell/create  → srv creates ShellController
  ├─ receives shell_id
  └─ returns { shell_id } to Claude immediately (non-blocking)

agentmux-srv ShellController
  ├─ spawns PTY subprocess
  ├─ reads PTY output line-by-line
  └─ publishes WPS event "shell_chunk" scoped to block:<block_id>
       { shell_id, kind, content, timestamp, exit_code? }

Frontend (useAgentStream.ts)
  ├─ subscribes "shell_chunk" scope block:<id>
  ├─ dispatches ShellNodeCreate on first chunk for a new shell_id
  ├─ dispatches ShellChunkAppend per line
  └─ dispatches ShellStatusUpdate on exit

DocumentRow routes ShellNode → <PersistentShellBlock>
```

### 5.2 New `agentmux-mcp` binary

New workspace member `agentmux-mcp/` (referenced in `.mcp.json` but not yet built).

**Exposed tools:**

| Tool | Purpose |
|------|---------|
| `Shell(cmd, cwd?, title?, env?)` | Start a persistent shell, returns `shell_id` |
| `ShellInput(shell_id, text)` | Send text to stdin of a running shell |
| `ShellStop(shell_id, signal?)` | Send SIGTERM/SIGKILL |
| `ShellStatus(shell_id)` | Query current status + last N lines |

`Shell` is the primary tool. The others are follow-on Phase 2 items (see §10).

**`Shell` tool contract:**
```json
{
  "name": "Shell",
  "description": "Start a long-running shell process. Returns immediately with a shell_id. Output streams live in the conversation. Use for build systems, watchers, dev servers.",
  "input_schema": {
    "type": "object",
    "properties": {
      "cmd":   { "type": "string", "description": "Command to run" },
      "cwd":   { "type": "string", "description": "Working directory" },
      "title": { "type": "string", "description": "Display label (defaults to cmd)" },
      "env":   { "type": "object", "description": "Extra environment variables" }
    },
    "required": ["cmd"]
  }
}
```

**Implementation sketch (Rust):**
- Reads `AGENTMUX_AGENT_ID`, `AGENTMUX_LOCAL_URL`, `AGENTMUX_AUTH_KEY`, `AGENTMUX_AGENT_BUS_ID` from env.
- POSTs `{ agent_id, cmd, cwd, title, env }` to `agentmux-srv` via `POST /api/v1/shell/create`.
- Receives `{ shell_id }` synchronously.
- Emits `StructuredOutput({ shell_id })` and exits.

### 5.3 Backend: shell/create endpoint + ShellController

**New HTTP route:** `POST /api/v1/shell/create`  
**Handler** in `agentmux-srv/src/server/app_api.rs` (alongside existing pane.open handler ~line 773):

```rust
async fn handle_shell_create(
    State(srv): State<Arc<ServerState>>,
    Json(req): Json<ShellCreateRequest>,
) -> Json<ShellCreateResponse>
```

- Generates a `shell_id` (UUID).
- Emits a `ShellNodeCreate` WPS event to `block:<agent_block_id>` immediately (so frontend creates the row before output arrives).
- Spawns a `ShellController` (already exists in `blockcontroller/shell.rs`) wired to publish **`shell_chunk`** WPS events (not `blockfile` events) to scope `block:<agent_block_id>`.
- Returns `{ shell_id }`.

**WPS event: `shell_chunk`**
```json
{
  "event": "shell_chunk",
  "scopes": ["block:<agent_block_id>"],
  "persist": 1024,
  "data": {
    "shell_id": "<uuid>",
    "op": "chunk" | "exit",
    "kind": "stdout" | "stderr" | "system",
    "content": "...",
    "timestamp": 1234567890,
    "exit_code": 0
  }
}
```

**WPS event: `shell_node_create`**
```json
{
  "event": "shell_node_create",
  "scopes": ["block:<agent_block_id>"],
  "persist": 1,
  "data": {
    "shell_id": "<uuid>",
    "cmd": "task dev",
    "cwd": "/workspace/agentmux",
    "title": "task dev",
    "timestamp": 1234567890
  }
}
```

### 5.4 Frontend: new types

**`frontend/app/view/agent/types.ts`** — add `ShellNode`:

```typescript
export interface ShellNode {
    type: "shell";
    id: string;               // shell_id
    cmd: string;
    title: string;
    cwd?: string;
    status: "running" | "exited-ok" | "exited-err" | "stopped";
    exitCode?: number;
    spawnedAt: number;        // Unix ms
    exitedAt?: number;        // Unix ms
    log: ToolStreamingLog;    // reuse exact same type
}
```

`DocumentNode` union gets `| ShellNode`.

### 5.5 Frontend: store commands

**`frontend/app/store/agent-document/types.ts`** — add:

```typescript
| { type: "ShellNodeCreate"; node: ShellNode }
| { type: "ShellChunkAppend"; shellId: string; chunk: ToolLogChunk }
| { type: "ShellStatusUpdate"; shellId: string; status: ShellNode["status"]; exitCode?: number; exitedAt: number }
```

Reducer in `agent-document-store.ts`:
- `ShellNodeCreate`: appends `ShellNode` to `nodes`, registers in `nodeIdSet`.
- `ShellChunkAppend`: finds ShellNode by id, immutably appends chunk to `log.chunks`.
- `ShellStatusUpdate`: updates `status`, `exitCode`, `exitedAt`, sets `log.open = false`.

### 5.6 Frontend: stream subscription

**`useAgentStream.ts`** — alongside existing `tool_chunk` subscription, add:

```typescript
waveEventSubscribe({
    eventType: "shell_node_create",
    scope: `block:${blockId}`,
    handler: (ev) => {
        const { shell_id, cmd, title, cwd, timestamp } = ev.data;
        pendingShellCreates.push({ shell_id, cmd, title, cwd, timestamp });
        scheduleFlush();
    }
});

waveEventSubscribe({
    eventType: "shell_chunk",
    scope: `block:${blockId}`,
    handler: (ev) => {
        const { shell_id, op, kind, content, timestamp, exit_code } = ev.data;
        if (op === "chunk") {
            pendingShellChunks.push({ shellId: shell_id, chunk: { kind, content, timestamp } });
        } else if (op === "exit") {
            pendingShellExits.push({ shellId: shell_id, exitCode: exit_code, exitedAt: timestamp });
        }
        scheduleFlush();
    }
});
```

Flush dispatches `ShellNodeCreate` first, then `ShellChunkAppend`, then `ShellStatusUpdate` — same ordering guarantee as existing tool_chunk batch.

### 5.7 Frontend: routing

**`DocumentRow.tsx`** — add branch:

```tsx
<Show when={props.node().type === "shell"}>
    <PersistentShellBlock
        node={props.node() as ShellNode}
        pinned={documentState.pinnedNodes.has(props.node().id)}
        onTogglePin={() => togglePin(props.node().id)}
    />
</Show>
```

### 5.8 Frontend: `<PersistentShellBlock>` component

New file: `frontend/app/view/agent/components/PersistentShellBlock.tsx`

**Collapsed row (always rendered):**
```tsx
<div class={clsx("agent-shell-block", {
    "running": status === "running",
    "exited-ok": status === "exited-ok",
    "exited-err": status === "exited-err",
    "stopped": status === "stopped",
    "expanded": expanded(),
    "collapsed": !expanded(),
})} onClick={onTogglePin}>
    <span class="agent-shell-sigil">⟩</span>
    <span class="agent-shell-title">{props.node.title}</span>
    <span class="agent-shell-elapsed">[{elapsed()}]</span>
    <Show when={lastLine()}>
        <span class="agent-shell-live-tail">↳ {lastLine()}</span>
    </Show>
    <Show when={props.node.status === "running"}>
        <button class="agent-shell-stop" onClick={handleStop} title="Stop">■</button>
    </Show>
</div>
```

**Elapsed timer:** `createEffect` + `setInterval(1000)` while `status === "running"`. Shows `M:SS` format. Clears on exit.

**Expanded panel:** `<Show when={expanded()}>` renders `<ToolBlockOverlay>` with the ShellNode's log passed in — reuse the existing overlay without modification (it operates on the `log` field, not the node type).

**`lastLine()`:** `createMemo` returning the last chunk's content (trimmed).

---

## 6. Visual Design

### 6.1 Component anatomy

```
╔════════════════════════════════════════════════════════════╗
║ ║ ⟩  task dev                 [running 1:23]  ↳ Linking… ║
╚════════════════════════════════════════════════════════════╝
  ▲
  left border (4px, status color)
```

### 6.2 Collapsed row layout

| Slot | Element | Notes |
|------|---------|-------|
| left border | 4px solid (status color) | same mechanic as `.agent-tool-block` |
| sigil | `⟩` | monospace, `--accent-color` while running |
| title | `props.node.title` | truncated with ellipsis |
| elapsed | `[M:SS]` | right-aligned, `--secondary-text-color` |
| live-tail | `↳ last line` | hidden below 400px container width |
| stop button | `■` | hover-reveal only, right edge |

### 6.3 Status colors (left border)

Applied via CSS class on `.agent-shell-block` using the same `data-tool`-style approach:

```scss
.agent-shell-block {
    &.running  { border-left-color: var(--term-bright-green); }
    &.exited-ok { border-left-color: var(--accent-color); opacity: 0.7; }
    &.exited-err { border-left-color: var(--error-color); }
    &.stopped  { border-left-color: var(--secondary-text-color); opacity: 0.6; }
}
```

Running state also gets a subtle left-border pulse animation (CSS keyframe, same period as the existing spinner animations):

```scss
&.running {
    animation: shell-running-pulse 2s ease-in-out infinite;
}
@keyframes shell-running-pulse {
    0%, 100% { border-left-color: var(--term-bright-green); }
    50%       { border-left-color: color-mix(in srgb, var(--term-bright-green) 40%, transparent); }
}
```

### 6.4 Expanded panel

Reuses `.agent-tool-panel` layout and `ToolOverlayLog` rendering exactly. No new SCSS required for the panel body — only the collapsed row needs new styles.

### 6.5 Container query breakpoints

Follows the Tier 3/4/5 breakpoints from `SPEC_AGENT_PANE_RESPONSIVE_AUX_INFO_2026_06_09`:

| Container width | Change |
|----------------|--------|
| < 400px | Hide live-tail |
| ≥ 600px | Show elapsed timer |
| ≥ 900px | Show stop button without requiring hover |

---

## 7. `agentmux-bashwrap` integration (Phase 2)

Currently, the `PreToolUse` hook rewrites **all** Bash calls through `agentmux-bashwrap exec`. Phase 1 introduces the explicit `Shell` tool. Phase 2 adds **auto-promotion**: if bashwrap detects a command that's still running after a threshold (e.g. 30s), it can:

1. Detach the process into a new ShellController.
2. Publish a `shell_node_create` event retroactively.
3. Return from the tool call immediately with a message: `"Process promoted to persistent shell <shell_id>. Continuing in background."`.

This is optional; the explicit `Shell` tool is the primary interface.

---

## 8. Files to Create / Modify

### New files
| File | Purpose |
|------|---------|
| `agentmux-mcp/` | New workspace member — MCP server binary |
| `agentmux-mcp/src/main.rs` | Tool definitions + HTTP client to srv |
| `agentmux-mcp/Cargo.toml` | Crate manifest |
| `frontend/app/view/agent/components/PersistentShellBlock.tsx` | Collapsed row + overlay wiring |
| `frontend/app/view/agent/styles/_shell-node.scss` | Shell-specific CSS |

### Modified files
| File | Change |
|------|--------|
| `frontend/app/view/agent/types.ts` | Add `ShellNode` to `DocumentNode` union |
| `frontend/app/store/agent-document/types.ts` | Add `ShellNodeCreate`, `ShellChunkAppend`, `ShellStatusUpdate` commands |
| `frontend/app/store/agent-document-store.ts` | Reducer cases for new commands |
| `frontend/app/view/agent/virtualization/DocumentRow.tsx` | Add `<Show>` branch for `shell` type |
| `frontend/app/view/agent/useAgentStream.ts` | Add `shell_node_create` + `shell_chunk` subscriptions |
| `frontend/app/view/agent/styles/index.scss` | Import `_shell-node.scss` |
| `agentmux-srv/src/server/app_api.rs` | Add `POST /api/v1/shell/create` handler |
| `agentmux-srv/src/backend/rpc_types.rs` | Add `ShellCreateRequest` / `ShellCreateResponse` |
| `agentmux-srv/src/backend/blockcontroller/shell.rs` | Add `shell_chunk` WPS publication mode (alongside existing `blockfile` model) |
| `Cargo.toml` (workspace) | Add `agentmux-mcp` member |
| `Taskfile.yml` | Add `build:mcp` + `task dev` dependency |

---

## 9. Implementation Phases

### Phase 1 — Frontend skeleton (no backend wiring)
- Add `ShellNode` type to `types.ts`.
- Add store commands + reducer stubs.
- Add `<PersistentShellBlock>` component with mock data.
- Wire `DocumentRow.tsx` router.
- Ship `_shell-node.scss` styles.
- **Deliverable:** Hardcoded ShellNode renders correctly in all states.

### Phase 2 — Backend + MCP tool
- Implement `agentmux-mcp` workspace member with `Shell` tool.
- Add `POST /api/v1/shell/create` to agentmux-srv.
- Wire `ShellController` to publish `shell_chunk` WPS events.
- Wire `useAgentStream.ts` subscriptions.
- **Deliverable:** Agent can call `Shell("task dev")` and see live build progress in the conversation.

### Phase 3 — Input + lifecycle tools
- `ShellInput(shell_id, text)` — send stdin.
- `ShellStop(shell_id)` — SIGTERM.
- `ShellStatus(shell_id)` — query.
- Stop button in UI wired to `ShellStop`.
- **Deliverable:** Full interactive shell lifecycle from agent.

### Phase 4 — Bashwrap auto-promotion (optional)
- Detect long-running Bash calls and auto-promote to ShellNode.
- Configurable timeout threshold.

---

## 10. Open Questions

1. **Output durability**: Should shell output persist across app restarts? Today `ToolStreamingLog` is session-only. If yes, wire into `FileStore` (like the existing `term` widget). If no, WPS replay buffer (1024 events) is sufficient.

2. **Multiple shells per agent**: Should the agent be able to run multiple concurrent persistent shells? Architecture supports it (each has its own `shell_id`), but UX for managing several shells in one conversation needs design.

3. **Shell vs. PTY**: Phase 1 uses a non-interactive PTY (no stdin). Phase 3 adds stdin. Full interactive PTY (xterm.js rendering) is out of scope — use the `term` widget for that.

4. **Changeset type**: Phase 1 is `minor` (new feature, non-breaking). Phase 2+ are `minor` each. No breaking changes to existing document schema (additive `| ShellNode`).

---

## 11. Relation to Existing Work

- **`SPEC_AGENT_PANE_RESPONSIVE_AUX_INFO_2026_06_09`** — ShellBlock uses the same container query breakpoints (Tier 2–5). Result pill concept (§600px) maps to elapsed timer + exit code pill.
- **`SPEC_TOOL_HOVER_CONSOLIDATION_2026_05_28`** — ShellBlock follows the same pin-to-expand model (no hover-expand). The `userHolding` anti-collapse guard from PR #1320 should be applied to ShellBlock.
- **`SPEC_TOOL_OUTPUT_CAP_2026_05_30`** — same `MAX_TOOL_OUTPUT_LINES` cap applies to shell log rendering.
- **`RETRO_REPLACECHILD_CRASH_2026-06-06`** — ShellChunkAppend must be dispatched inside `batch()` alongside ShellNodeCreate to avoid the same reconciler race that affected tool_chunk.
