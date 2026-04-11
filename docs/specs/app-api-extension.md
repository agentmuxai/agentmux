# App API Extension Spec

**Status:** Proposed
**Date:** 2026-04-10
**Motivation:** CEF webview content is inaccessible to external automation tools
(Windows MCP, AppleScript, etc.). The only reliable way to programmatically
control AgentMux is through its own API surface.

---

## Problem

AgentMux exposes ~65 CEF IPC commands and 100+ backend RPC commands, but they're
low-level primitives (SetMeta, CreateBlock, ControllerResync). Common high-level
operations require orchestrating 3-5 calls in the correct order with the correct
metadata keys. This makes external automation fragile and tightly coupled to
internal implementation details.

**Example — opening an agent pane today requires:**
1. `CreateBlock` with `{ view: "agent" }`
2. `SetMeta` with agentId, provider, controller, cmd, cmd:args, cmd:env, cmd:cwd, ...
3. `ControllerResync` with forcerestart
4. Wait for CLI resolution + auth check
5. `AgentInput` with the first message

Any change to metadata keys, env var names, or launch flow breaks all callers.

---

## Design Principles

1. **High-level intent, not low-level mechanics.** Callers express what they want
   ("open an agent pane with AgentX"), not how to do it ("create block, set 14
   metadata keys, resync controller").

2. **Stable contract.** Internal metadata keys, env vars, and controller types
   can change without breaking the API. The API translates intent to internals.

3. **Idempotent where possible.** `openAgent` with the same agent ID returns the
   existing pane if one is already open. `sendAgentMessage` auto-spawns if needed.

4. **Observable.** Every action returns enough state to know what happened.
   Streaming endpoints for real-time output.

5. **Transport-agnostic.** Same commands work over CEF IPC (in-process), WebSocket
   RPC (cross-process), and HTTP REST (external tools). Implementation is one
   function called from all three transports.

---

## New Commands

### Tier 1 — Agent Lifecycle (needed now)

#### `agent.open`
Open an agent pane with a registered Forge agent, or return existing pane.

```typescript
// Request
{
  agentId: string;           // Forge agent ID (e.g., "agentx")
  tabId?: string;            // Tab to open in (default: current tab)
  splitDirection?: "horizontal" | "vertical";  // Split existing pane
  splitReferenceBlockId?: string;              // Pane to split from
  focus?: boolean;           // Focus the new pane (default: true)
}

// Response
{
  blockId: string;           // Block ID of the agent pane
  tabId: string;             // Tab containing the pane
  agentId: string;           // Confirmed agent ID
  provider: string;          // Provider (claude, codex, gemini)
  controllerType: string;    // "persistent" | "subprocess"
  status: string;            // "init" | "running" | "done"
  created: boolean;          // true if new pane, false if existing
}
```

#### `agent.send`
Send a message to an agent pane. Auto-spawns the process if not running.

```typescript
// Request
{
  blockId: string;           // Target block ID
  message: string;           // User message text
}

// Response
{
  blockId: string;
  status: string;            // "running" | "queued"
  sessionId?: string;        // CLI session ID if available
}
```

#### `agent.stop`
Stop the agent's running process.

```typescript
// Request
{
  blockId: string;
  signal?: "SIGINT" | "SIGTERM" | "SIGKILL";  // Default: SIGTERM
}

// Response
{
  blockId: string;
  status: string;            // "done"
  exitCode?: number;
}
```

#### `agent.status`
Get the current state of an agent pane.

```typescript
// Request
{
  blockId: string;
}

// Response
{
  blockId: string;
  agentId: string;
  provider: string;
  controllerType: string;
  status: string;            // "init" | "running" | "done" | "crashed"
  sessionId?: string;
  pid?: number;
  exitCode?: number;
  uptime_ms?: number;
}
```

#### `agent.list`
List all active agent panes across all tabs.

```typescript
// Request
{}

// Response
{
  agents: Array<{
    blockId: string;
    tabId: string;
    agentId: string;
    provider: string;
    status: string;
    sessionId?: string;
  }>;
}
```

#### `agent.output`
Read the accumulated output of an agent pane (non-streaming).

```typescript
// Request
{
  blockId: string;
  afterLine?: number;        // Only return lines after this index
  maxLines?: number;         // Limit (default: 1000)
}

// Response
{
  blockId: string;
  lines: string[];           // Raw NDJSON lines from the CLI
  totalLines: number;
  hasMore: boolean;
}
```

#### `agent.stream`
Subscribe to real-time output from an agent pane (streaming RPC).

```typescript
// Request
{
  blockId: string;
}

// Stream Response (one per stdout line)
{
  blockId: string;
  line: string;              // Raw NDJSON line
  lineIndex: number;
}
```

---

### Tier 2 — Pane & Layout Management (needed soon)

#### `pane.open`
Open a new pane with any widget type.

```typescript
// Request
{
  widget: string;            // Widget key: "agent", "term", "sysinfo", etc.
  tabId?: string;            // Default: current tab
  splitDirection?: "horizontal" | "vertical";
  splitReferenceBlockId?: string;
  focus?: boolean;
  meta?: Record<string, any>;  // Initial block metadata
}

// Response
{
  blockId: string;
  tabId: string;
  widget: string;
}
```

#### `pane.close`
Close a pane by block ID.

```typescript
// Request
{
  blockId: string;
}

// Response
{
  closed: boolean;
}
```

#### `pane.focus`
Focus a specific pane.

```typescript
// Request
{
  blockId: string;
}

// Response
{
  focused: boolean;
}
```

#### `pane.list`
List all panes in a tab (or all tabs).

```typescript
// Request
{
  tabId?: string;            // Omit for all tabs
}

// Response
{
  panes: Array<{
    blockId: string;
    tabId: string;
    view: string;            // "agent", "term", "sysinfo", etc.
    focused: boolean;
    meta: Record<string, any>;
  }>;
}
```

#### `pane.resize`
Resize a pane within its layout.

```typescript
// Request
{
  blockId: string;
  size: number;              // Flex size (relative to siblings)
}

// Response
{
  blockId: string;
  size: number;
}
```

---

### Tier 3 — Tab & Window Management (needed for multi-window workflows)

#### `tab.create`
Create a new tab.

```typescript
// Request
{
  name?: string;
  windowId?: string;         // Default: current window
  activate?: boolean;        // Switch to new tab (default: true)
}

// Response
{
  tabId: string;
  name: string;
}
```

#### `tab.close`
Close a tab.

```typescript
// Request
{
  tabId: string;
}

// Response
{
  closed: boolean;
}
```

#### `tab.list`
List all tabs.

```typescript
// Request
{
  windowId?: string;
}

// Response
{
  tabs: Array<{
    tabId: string;
    name: string;
    active: boolean;
    paneCount: number;
  }>;
}
```

#### `tab.activate`
Switch to a tab.

```typescript
// Request
{
  tabId: string;
}

// Response
{
  activated: boolean;
}
```

#### `window.list`
List all windows.

```typescript
// Response
{
  windows: Array<{
    windowId: string;
    label: string;
    focused: boolean;
    tabCount: number;
    position: { x: number; y: number };
    size: { width: number; height: number };
  }>;
}
```

---

### Tier 4 — Terminal Interaction (needed for automation)

#### `terminal.open`
Open a terminal pane.

```typescript
// Request
{
  tabId?: string;
  shell?: string;            // "bash", "pwsh", "cmd" (default: auto-detect)
  cwd?: string;              // Working directory
  env?: Record<string, string>;
  splitDirection?: "horizontal" | "vertical";
  splitReferenceBlockId?: string;
}

// Response
{
  blockId: string;
  tabId: string;
  shell: string;
}
```

#### `terminal.input`
Send text to a terminal.

```typescript
// Request
{
  blockId: string;
  text: string;
  sendEnter?: boolean;       // Append \n (default: true)
}

// Response
{
  sent: boolean;
}
```

#### `terminal.signal`
Send a signal to the terminal process.

```typescript
// Request
{
  blockId: string;
  signal: "SIGINT" | "SIGTERM" | "SIGKILL";
}

// Response
{
  sent: boolean;
}
```

---

### Tier 5 — Workspace & Config (needed for setup automation)

#### `workspace.info`
Get workspace metadata.

```typescript
// Response
{
  version: string;           // AgentMux version
  dataDir: string;
  configDir: string;
  platform: string;
  windowCount: number;
  tabCount: number;
  paneCount: number;
  agentCount: number;        // Running agent panes
}
```

#### `forge.list`
List registered Forge agents (not running panes — agent configs).

```typescript
// Response
{
  agents: Array<{
    id: string;
    name: string;
    provider: string;
    icon: string;
    workingDir: string;
    agentType: string;       // "host" | "worker"
  }>;
}
```

#### `forge.create`
Create a new Forge agent.

```typescript
// Request
{
  name: string;
  provider: string;          // "claude" | "codex" | "gemini"
  workingDir?: string;
  icon?: string;
  agentType?: string;
  providerFlags?: string;
}

// Response
{
  id: string;
  name: string;
  provider: string;
}
```

---

## Implementation Plan

### Phase 1 — Backend Command Handlers (Rust)

**New file:** `agentmux-srv/src/server/app_api.rs`

Single module that registers all `agent.*`, `pane.*`, `tab.*`, `terminal.*`,
`workspace.*`, and `forge.*` commands with the RPC engine. Each handler
orchestrates the low-level operations (CreateBlock, SetMeta, ControllerResync)
internally.

```rust
pub fn register_app_api_handlers(engine: &mut WshRpcEngine) {
    // agent.*
    engine.register_handler("agent.open", ...);
    engine.register_handler("agent.send", ...);
    engine.register_handler("agent.stop", ...);
    engine.register_handler("agent.status", ...);
    engine.register_handler("agent.list", ...);
    engine.register_handler("agent.output", ...);
    engine.register_stream_handler("agent.stream", ...);

    // pane.*
    engine.register_handler("pane.open", ...);
    engine.register_handler("pane.close", ...);
    engine.register_handler("pane.focus", ...);
    engine.register_handler("pane.list", ...);

    // tab.*, terminal.*, workspace.*, forge.*
    ...
}
```

### Phase 2 — CEF IPC Bridge

Expose the same commands via CEF IPC so `getApi()` can call them:

```typescript
// frontend/util/cef-api.ts additions
agentOpen(opts: AgentOpenRequest): Promise<AgentOpenResponse>;
agentSend(opts: AgentSendRequest): Promise<AgentSendResponse>;
agentStop(opts: AgentStopRequest): Promise<AgentStopResponse>;
agentStatus(opts: AgentStatusRequest): Promise<AgentStatusResponse>;
agentList(): Promise<AgentListResponse>;
paneOpen(opts: PaneOpenRequest): Promise<PaneOpenResponse>;
paneClose(opts: PaneCloseRequest): Promise<PaneCloseResponse>;
paneList(opts?: PaneListRequest): Promise<PaneListResponse>;
tabCreate(opts?: TabCreateRequest): Promise<TabCreateResponse>;
tabList(opts?: TabListRequest): Promise<TabListResponse>;
workspaceInfo(): Promise<WorkspaceInfoResponse>;
forgeList(): Promise<ForgeListResponse>;
```

### Phase 3 — HTTP REST Gateway (external access)

**New file:** `agentmux-srv/src/server/rest_api.rs`

Optional HTTP REST endpoints for external tools (curl, MCP servers, scripts):

```
POST /api/v1/agent/open     → agent.open
POST /api/v1/agent/send     → agent.send
POST /api/v1/agent/stop     → agent.stop
GET  /api/v1/agent/status   → agent.status
GET  /api/v1/agent/list     → agent.list
GET  /api/v1/agent/output   → agent.output
WS   /api/v1/agent/stream   → agent.stream

POST /api/v1/pane/open      → pane.open
POST /api/v1/pane/close     → pane.close
GET  /api/v1/pane/list      → pane.list

POST /api/v1/tab/create     → tab.create
GET  /api/v1/tab/list       → tab.list

GET  /api/v1/workspace/info → workspace.info
GET  /api/v1/forge/list     → forge.list
```

Auth: Bearer token from `AGENTMUX_AUTH_KEY` env var (same as WSH auth).

### Phase 4 — MCP Server Integration

Expose App API commands as MCP tools in the `agentmux` MCP server so agents
can programmatically manage their own workspace:

```json
{
  "tools": [
    { "name": "open_agent_pane", "inputSchema": { "agentId": "string", ... } },
    { "name": "send_agent_message", "inputSchema": { "blockId": "string", "message": "string" } },
    { "name": "open_terminal", "inputSchema": { "cwd": "string", ... } },
    { "name": "terminal_input", "inputSchema": { "blockId": "string", "text": "string" } },
    { "name": "list_panes", "inputSchema": {} },
    { "name": "workspace_info", "inputSchema": {} }
  ]
}
```

---

## Error Handling

All commands return structured errors:

```typescript
{
  error: {
    code: string;            // Machine-readable: "AGENT_NOT_FOUND", "BLOCK_NOT_FOUND", etc.
    message: string;         // Human-readable description
    details?: any;           // Optional context
  }
}
```

**Standard error codes:**

| Code | Meaning |
|------|---------|
| `AGENT_NOT_FOUND` | Forge agent ID doesn't exist |
| `BLOCK_NOT_FOUND` | Block ID doesn't exist |
| `TAB_NOT_FOUND` | Tab ID doesn't exist |
| `ALREADY_RUNNING` | Agent process already active |
| `NOT_RUNNING` | Agent process not active (can't send/stop) |
| `CLI_NOT_AVAILABLE` | CLI binary not installed |
| `AUTH_REQUIRED` | CLI not authenticated |
| `INVALID_WIDGET` | Unknown widget type |
| `INVALID_PROVIDER` | Unknown provider |

---

## Versioning

- API version in URL path: `/api/v1/...`
- Version header: `X-AgentMux-API-Version: 1`
- Breaking changes increment version; old versions supported for 2 major releases
- Non-breaking additions (new fields, new commands) don't bump version

---

## Security

- HTTP REST endpoints require Bearer token authentication
- Token source: `AGENTMUX_AUTH_KEY` (same as WebSocket auth)
- Bind to `127.0.0.1` only (no network exposure)
- Rate limiting: 100 req/s per endpoint (prevent accidental loops)
- `agent.send` and `terminal.input` are write operations — logged with caller ID

---

## Observability

Every command logs:
- Command name, caller, timestamp
- Execution time
- Success/error status
- For mutations: before/after state

Format: `[app-api] agent.open agentId=agentx blockId=abc123 created=true 12ms`

---

## Migration Path

1. **Phase 1:** Ship `agent.*` commands (Tier 1) — unblocks persistent process testing
2. **Phase 2:** Ship `pane.*` + `tab.*` (Tiers 2-3) — unblocks automation workflows
3. **Phase 3:** Ship HTTP REST gateway — unblocks external tools (curl, scripts)
4. **Phase 4:** Ship MCP integration — unblocks agent self-management

Existing low-level commands (`CreateBlock`, `SetMeta`, etc.) remain available.
The App API is a convenience layer, not a replacement.

---

## Files to Create

| File | Purpose |
|------|---------|
| `agentmux-srv/src/server/app_api.rs` | All command handlers |
| `agentmux-srv/src/server/rest_api.rs` | HTTP REST gateway (Phase 3) |
| `frontend/types/app-api.d.ts` | TypeScript types for all requests/responses |

## Files to Modify

| File | Change |
|------|--------|
| `agentmux-srv/src/server/mod.rs` | Register app_api module |
| `agentmux-srv/src/server/websocket.rs` | Call `register_app_api_handlers()` |
| `frontend/util/cef-api.ts` | Add `agentOpen()`, `agentSend()`, etc. |
| `agentmux-cef/src/ipc.rs` | Route new IPC commands to backend RPC |
