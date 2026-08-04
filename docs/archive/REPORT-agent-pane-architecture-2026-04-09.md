# Agent Pane — Comprehensive Architecture Breakdown

**Date:** 2026-04-09
**Purpose:** Refactoring planning document

---

## 1. Frontend Components

### 1.1 File Structure

**Location:** `frontend/app/view/agent/`

| File | Purpose |
|------|---------|
| `agent-model.ts` | ViewModel: launch orchestration, block metadata, RPC coordination |
| `agent-view.tsx` | Top-level component: AgentPicker + AgentPresentationView + runLaunchFlow() |
| `state.ts` | SolidJS signal factory: createAgentAtoms() |
| `types.ts` | TypeScript types: DocumentNode, StreamEvent, ToolNode, ProviderDefinition |
| `stream-parser.ts` | NDJSON parser: stream events -> DocumentNodes |
| `useAgentStream.ts` | SolidJS hook: subscribes to WPS blockfile, translates, parses |
| `index.ts` | Re-exports |
| `init-monitor.ts` | Initialization state tracking |

### 1.2 Components Subdirectory

**Location:** `frontend/app/view/agent/components/`

| Component | Responsibility |
|-----------|----------------|
| `AgentDocumentView.tsx` | Renders document: log lines at top, then DocumentNode list. Auto-scroll, collapse/expand |
| `AgentFooter.tsx` | Textarea input + Enter/Shift+Enter handling + loading spinner |
| `ToolBlock.tsx` | Tool execution display: Bash, Edit (diff), Read, Write, Grep/Glob. 50KB truncation |
| `AgentMessageBlock.tsx` | Agent-to-agent message display (mux vs ject) |
| `MarkdownBlock.tsx` | Markdown rendering with streaming support |
| `SubagentLinkBlock.tsx` | Clickable link to spawn/open subagent pane |
| `BashOutputViewer.tsx` | Specialized bash stdout/stderr rendering |
| `DiffViewer.tsx` | Side-by-side diff for Edit tool |

### 1.3 AgentViewModel (`agent-model.ts`)

**Key Atoms** (via `createAgentAtoms()`):
- `documentAtom` — rendered DocumentNode[]
- `documentStateAtom` — collapsed nodes, scroll, selection, filters
- `streamingStateAtom` — active, bufferSize, lastEventTime
- `processAtom` — pid, status (idle/running/paused/failed), canRestart
- `authAtom`, `sessionIdAtom`, `rawOutputAtom`

**Key Methods:**
- `launchAgent(agentId)` — simple provider launch: validate Node.js, build CLI args, set block metadata, ControllerResync
- `launchForgeAgent(agent)` — full forge setup: load content blobs, build CLAUDE.md, write skill commands, inject MCP server, per-agent GitHub/auth isolation

---

## 2. Launch Flow (Detailed Step-by-Step)

**Location:** `agent-view.tsx` L168-372 — `runLaunchFlow()`

### Phase 0: Container Runtime Check (if applicable)
- If `agentMode === "container"`, call `ResolveCliCommand` with provider_id="docker"

### Phase 1: CLI Resolution / Installation
- **RPC:** `ResolveCliCommand(TabRpcClient, {...})` — 5min timeout
- Input: provider_id, cli_command, npm_package, pinned_version, install_command
- Output: `ResolveCliResult { cli_path, version, source }`
- Subscribe to `install_progress` events for npm output streaming
- Block metadata `cmd` updated with resolved path

### Phase 2: Auth Check -> Auto-login if Needed
- **RPC:** `CheckCliAuthCommand(TabRpcClient, { cli_path, auth_check_args, auth_env })`
- Claude fast path: reads `~/.claude/.credentials.json` directly (no subprocess)
- If not authenticated:
  - `getApi().runCliLogin(cli_path, login_args, authEnv)` — spawns browser
  - Polling loop: `CheckCliAuthCommand` every 2s, 5min deadline
  - Cancel via `getApi().cancelCliLogin()`
  - On timeout: return "auth_failed", show Retry button

### Phase 3: Controller Registration
- **RPC:** `ControllerResyncCommand(TabRpcClient, { tabid, blockid, forcerestart: false })`
- Creates SubprocessController on backend
- `agentReady` signal set to true

### Signals During Flow:
- `flowRunning` — true during entire flow
- `agentReady` — true after successful completion
- `loginWaiting` — true during auth polling (shows Cancel button)
- `canRetry` — true after auth_failed (shows Retry button)
- `authUrl` — OAuth URL string (shown with copy button)
- `isLoading()` — derived: `flowRunning() || !agentReady()`

---

## 3. Provider System

**Location:** `frontend/app/view/agent/providers/index.ts`

### ProviderDefinition Interface
```
id, displayName, cliCommand, launchArgs, outputFormat,
authType, authCheckCommand, authLoginCommand,
npmPackage, pinnedVersion,
authConfigDirEnvVar, authDirName, authExtraEnv,
resumeFlag, sessionIdField
```

### Provider Configs

| Field | Claude | Codex | Gemini |
|-------|--------|-------|--------|
| CLI | claude | codex | gemini |
| Auth | OAuth | OAuth | OAuth |
| npm | @anthropic-ai/claude-code | @openai/codex | @google/gemini-cli |
| Version | latest | 0.116.0 | 0.32.1 |
| Resume | --resume | null | -r |
| Session field | session_id | thread_id | session_id |
| Launch args | -p --output-format stream-json --verbose --include-partial-messages --dangerously-skip-permissions | exec --json --dangerously-bypass-approvals-and-sandbox - | --output-format stream-json --yolo -p "" |

---

## 4. Subprocess Controller (Backend)

**File:** `agentmux-srv/src/backend/blockcontroller/subprocess.rs`

### Architecture
- **Per-turn model:** Fresh `claude -p` per user message
- **Multi-turn:** Session ID captured from init event, `--resume <sid>` on next turn
- **State machine:** INIT -> RUNNING -> DONE -> RUNNING (re-spawn)
- **I/O:** Two async tasks per turn: stdout_reader + process_waiter

### spawn_turn() Steps (L189-515)
1. Lock check (prevent concurrent spawns)
2. Resume flag handling (append --resume + session_id if available)
3. Status update -> RUNNING, publish via WPS
4. Build Command via `make_cli_cmd()` (handles .cmd wrappers)
5. Set args, working dir (~expansion), env vars, piped stdio
6. Spawn process, store PID
7. **stdin writer task:** write message bytes, close stdin
8. **stdout_reader task:** line-by-line NDJSON, session ID capture, health monitoring, WPS blockfile publication
9. **stderr_reader task:** non-blocking logging
10. **Health watchdog:** every 5s timeout check
11. **process_waiter:** select on exit OR kill signal, update status to DONE

### Session Resumption
1. First turn: CLI emits init event with session_id
2. Extract and store in inner state + block metadata
3. Next turn: append `--resume <sid>` to CLI args

---

## 5. CLI Handlers (Backend)

**File:** `agentmux-srv/src/server/cli_handlers.rs`

### ResolveCliCommand (L18-429)
Resolution order:
1. Check versioned dir: `~/.agentmux/<version>/cli/<provider>/`
2. System scan: known paths + where/which
3. Copy from system to versioned dir (fast, no network)
4. npm install: `npm install --prefix <dir> <pkg>@<version>` (with Windows raw_arg handling)
5. Official installer: PowerShell/bash with 120s timeout

### CheckCliAuthCommand (L432-564)
- Claude fast path: read .credentials.json (isolated dir -> global fallback)
- Others: run CLI auth check command (25s timeout)

### RunCliLogin (L566-595)
- Spawn login process with null stdio (browser opens)
- Return immediately, frontend polls

### make_cli_cmd (agentmux-common crate)
- Parse .cmd wrapper, extract %dp0% relative JS path
- Invoke `node <script>` directly, bypass cmd.exe /C

---

## 6. Data Flow

### User Message -> CLI stdin
```
AgentFooter textarea
  -> handleSendMessage(message)
    -> RpcApi.AgentInputCommand(TabRpcClient, { blockid, message })
      -> WebSocket -> RPC router -> AgentInputCommand handler (websocket.rs L714)
        -> Load block metadata (cmd, args, env, cwd)
        -> Create SubprocessSpawnConfig
        -> subprocess_ctrl.spawn_turn(config)
          -> Spawn process, write message to stdin as JSON
```

### CLI stdout -> Frontend
```
Subprocess stdout (NDJSON lines)
  -> stdout_reader task (line-by-line BufReader)
    -> WPS blockfile publication: handle_append_block_file(broker, blockid, "output", line)
      -> Frontend subscription: getFileSubject(blockId, "output")
        -> Decode base64 -> lineBuffer -> split on \n
          -> JSON.parse each line
            -> Provider translator: rawEvent -> StreamEvent[]
              -> Stream parser: StreamEvent -> DocumentNode[]
                -> Update documentAtom signal
```

### WPS Event Types
- `install_progress` — npm install output lines
- `blockfile` — subprocess stdout streaming (subject: "output")
- `controllerstatus` — state changes (init/running/done + exit code)
- `subagent:spawned` / `subagent:completed` — inter-agent events

---

## 7. Block/Layout Integration

### Block Metadata Fields (Agent Pane)
```
view: "agent"
controller: "subprocess"
agentId: "claude"
agentProvider: "claude"
agentName: "My Agent"
agentOutputFormat: "claude-stream-json"
cmd: "/path/to/claude"
cmd:args: ["-p", "--output-format", "stream-json", ...]
cmd:cwd: "~/.agentmux/agents/my-agent"
cmd:env: { CLAUDE_CONFIG_DIR, GH_CONFIG_DIR, AGENTMUX_AGENT_ID, ... }
agent:resume_flag: "--resume"
agent:session_id_field: "session_id"
agent:sessionid: "abc123"  (captured from CLI init event)
```

### Creation Flow
```
AgentPicker -> click card
  -> model.launchAgent(agentId)
    -> Set block metadata via RpcApi.SetMetaCommand()
    -> Create controller via RpcApi.ControllerResyncCommand()
    -> AgentViewWrapper switches to AgentPresentationView
```

---

## 8. Known Issues & State

### Working End-to-End
- Claude Code: resolve, auth, launch, single-turn, streaming, tool rendering
- Forge agents: content loading, skill injection, CLAUDE.md assembly, MCP auto-inject
- Multi-turn: session ID capture, --resume appending
- Subagent linking: event subscription, click-to-open

### Incomplete / Issues
- **Codex & Gemini:** Provider definitions exist, NOT tested end-to-end
- **Container agents:** Docker detection code present, NOT tested
- **Loading spinner:** Shows during launch but flow completes in ~560ms (barely visible)
- **API key auth:** No UI for Codex/Gemini API key entry
- **System scanner in resolver:** Still falls back to `where`/`which` + copies system binaries (should be npm-only)

### Recent Fixes (PR #318, branch `agenta/fix-cmd-wrapper-node-resolve`)
- .cmd wrapper -> node resolution (agentmux-common shared crate)
- Auth check fallback to global ~/.claude/.credentials.json
- Layout self-healing on startup + delete (prune orphaned block nodes)
- Loading spinner in agent footer (agentReady signal)

---

## Key Files Reference

| File | Lines | Role |
|------|-------|------|
| `frontend/app/view/agent/agent-model.ts` | L13-412 | ViewModel |
| `frontend/app/view/agent/agent-view.tsx` | L59-675 | Wrapper + Picker + Presentation |
| `frontend/app/view/agent/state.ts` | L46-87 | Atoms factory |
| `frontend/app/view/agent/types.ts` | L14-347 | Type definitions |
| `frontend/app/view/agent/stream-parser.ts` | L27-200+ | NDJSON parser |
| `frontend/app/view/agent/useAgentStream.ts` | L31-184 | Stream hook |
| `frontend/app/view/agent/providers/index.ts` | L4-134 | Provider configs |
| `frontend/app/view/agent/components/AgentDocumentView.tsx` | L32-200 | Document renderer |
| `frontend/app/view/agent/components/AgentFooter.tsx` | L16-57 | Input + spinner |
| `frontend/app/view/agent/components/ToolBlock.tsx` | L23-163 | Tool rendering |
| `agentmux-srv/src/backend/blockcontroller/subprocess.rs` | L1-530+ | Subprocess controller |
| `agentmux-srv/src/server/cli_handlers.rs` | L12-596 | CLI resolution, auth, login |
| `agentmux-srv/src/server/websocket.rs` | L714-783 | AgentInput RPC handler |
| `agentmux-common/src/cli.rs` | L1-55 | Shared .cmd wrapper parser |
