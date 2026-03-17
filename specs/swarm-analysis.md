# Swarm Observability — Subagent Watcher Analysis

**Date:** 2026-03-16
**Issue:** [#101 — Swarm Orchestration: Multi-Agent Parallel Execution](https://github.com/agentmuxai/agentmux/issues/101)
**Author:** AgentX

---

## 1. Executive Summary

AgentMux's swarm feature is **not** about orchestrating agents or managing task queues. Claude Code already handles subagent spawning, delegation, and result aggregation through its built-in Task tool. The swarm feature is about **observability** — watching subagents work in real-time as they're spawned by the CLI environment.

**The killer feature:** When a Claude Code agent spawns subagents (via Task tool), AgentMux detects them and surfaces their activity as live panes. The user watches parallel work unfold across multiple panes without switching contexts.

**What we're NOT building:**
- Task queues or filesystem coordination protocols
- Git worktree management or merge coordination
- Planner/executor/reviewer role assignment
- Custom orchestration daemons

**What we ARE building:**
- Real-time detection of Claude Code subagent spawning
- Live pane creation to observe subagent activity
- Activity feed showing tool calls, outputs, and progress
- Swarm analytics widget: cross-agent status, token usage, activity heatmaps
- Conversation history search across all agents and sessions

---

## 2. Agent Model

### Human-Facing Agents

Every agent in the system is **human-facing**. The user directly interacts with each one:

| Agent | Environment | Access |
|-------|-------------|--------|
| **AgentX** | Host (Windows) | Claw deployment, host operations |
| **AgentY** | Host (Windows) | Secondary host agent |
| **Agent1-5** | Docker containers | Code, PRs, reviews (isolated workspaces) |

These are not "workers" managed by a planner. Each agent has its own terminal pane in AgentMux and takes direct instructions from the user.

### Production Deployment Model

**Starter pack:** 1 host agent + 3 container agents.

Users can expand from there. The system is designed so a single user can manage multiple agents concurrently, giving each distinct tasks and watching them all work in parallel.

### How Subagents Fit

When a user asks an agent (e.g., Agent1) to do something complex, Claude Code's Task tool spawns **subagents** internally. These subagents:

- Are created and managed entirely by Claude Code
- Run autonomously within the parent agent's session
- Produce JSONL logs alongside the parent session
- Complete and return results to the parent

**AgentMux does not create, manage, or orchestrate these subagents.** It only watches them.

---

## 3. Subagent JSONL Data Source

### File Structure

Claude Code stores session data at:
```
~/.config/claude-{agentId}/projects/{encoded-workspace-path}/
├── {sessionUuid}.jsonl                    # Parent session
└── {sessionUuid}/
    └── subagents/
        ├── agent-{shortId}.jsonl          # Subagent 1
        ├── agent-{shortId}.jsonl          # Subagent 2
        └── ...
```

- Parent session: `{uuid}.jsonl` (full conversation including Task tool calls)
- Subagent directory: `{uuid}/subagents/` (created when Task tool spawns agents)
- Subagent files: `agent-{7-char-hex}.jsonl` (e.g., `agent-ac1e917.jsonl`)

### JSONL Entry Schema

Every line in a subagent JSONL file is a JSON object:

```json
{
  "parentUuid": "uuid-or-null",
  "isSidechain": true,
  "userType": "external",
  "cwd": "C:\\Users\\asafe\\.claw\\agentx-workspace",
  "sessionId": "uuid",
  "version": "2.1.34",
  "agentId": "ac1e917",
  "slug": "toasty-zooming-grove",
  "type": "user|assistant|progress",
  "uuid": "uuid",
  "timestamp": "2026-02-07T20:01:29.390Z"
}
```

**Key fields for observability:**
- `agentId` — Unique 7-char hex identifier for this subagent
- `slug` — Human-readable session name
- `type` — Event type (user, assistant, progress)
- `timestamp` — When the event occurred
- `isSidechain` — Always `true` for subagents

### Event Types

**Assistant messages** (subagent thinking and acting):
```json
{
  "type": "assistant",
  "message": {
    "model": "claude-haiku-4-5-20251001",
    "content": [
      { "type": "text", "text": "Let me search for..." },
      { "type": "tool_use", "name": "Grep", "input": { "pattern": "..." } }
    ],
    "usage": { "input_tokens": 3, "output_tokens": 1 }
  }
}
```

**Tool results** (feedback from tool execution):
```json
{
  "type": "user",
  "message": {
    "content": [{
      "tool_use_id": "toolu_01...",
      "type": "tool_result",
      "content": "file contents or command output",
      "is_error": false
    }]
  }
}
```

**Progress events** (real-time execution tracking):
```json
{
  "type": "progress",
  "data": {
    "type": "bash_progress",
    "output": "partial output",
    "elapsedTimeSeconds": 5
  },
  "toolUseID": "bash-progress-0"
}
```

---

## 4. Existing AgentMux Infrastructure

AgentMux already has the building blocks for subagent watching:

| Component | Status | Relevance |
|-----------|--------|-----------|
| **File watcher** (`notify` crate) | Production | Watch `subagents/` directories for new JSONL files |
| **Block/pane creation** (`wcore::create_block`) | Production | Create new panes for subagent activity |
| **Agent registration** (`/wave/reactive/register`) | Production | Register subagents in the reactive system |
| **EventBus** | Production | Push subagent events to frontend in real-time |
| **OSC 16162 protocol** | Production | Metadata extraction from terminal output |
| **Forge widget** | Production | Agent creation/management (separate concern from Swarm) |
| **Layout tree** (Solid.js signals) | Production | Split panes, insert subagent views |
| **Block metadata** (`MetaMapType`) | Production | Store `subagent:id`, `subagent:path`, etc. |
| **Tab management** | Production | Group subagent panes under parent agent tab |

### Key Code Paths

- **Block creation:** `agentmuxsrv-rs/src/backend/wcore.rs` → `create_block()`
- **Agent registration:** `agentmuxsrv-rs/src/server/reactive.rs` → `handle_reactive_register()`
- **File watching:** `agentmuxsrv-rs/src/backend/config_watcher_fs.rs` → `spawn_settings_watcher()` (pattern to follow)
- **Event broadcasting:** `agentmuxsrv-rs/src/backend/eventbus.rs` → `broadcast_event()`
- **OSC handling:** `frontend/app/view/term/termosc.ts` → `handleOsc16162Command()`
- **Layout manipulation:** `frontend/layout/lib/layoutModel.ts` → `insertNode()`, `splitHorizontal()`

---

## 5. Architecture: Subagent Watcher

### Overview

```
┌────────────────────────────────────────────────────────┐
│  AgentMux UI                                           │
│                                                        │
│  ┌─────────────────┐  ┌─────────────────────────────┐  │
│  │  Agent1 (PTY)   │  │  Subagent ac1e917           │  │
│  │                 │  │  "Searching for API..."     │  │
│  │  User: "Find    │  │  > Grep: pattern="endpoint" │  │
│  │  all API errors │  │  > Read: src/api/errors.ts  │  │
│  │  and fix them"  │  │  > Edit: line 42            │  │
│  │                 │  ├─────────────────────────────┤  │
│  │  Spawned 2      │  │  Subagent a07d705           │  │
│  │  subagents...   │  │  "Running test suite..."    │  │
│  │                 │  │  > Bash: npm test            │  │
│  │                 │  │  > Progress: 12/48 passing   │  │
│  └─────────────────┘  └─────────────────────────────┘  │
└────────────────────────────────────────────────────────┘
```

The left pane is the real terminal with the parent agent. The right panes are **read-only activity feeds** for each subagent, opened on demand when the user clicks a subagent link in the parent agent's pane.

### Detection Flow

```
1. User gives task to Agent1 in terminal pane
2. Claude Code spawns subagent via Task tool
3. New file appears: {session}/subagents/agent-{id}.jsonl
4. AgentMux file watcher detects the new file
5. Backend parses JSONL, emits "subagent:spawned" event
6. Agent pane renders a clickable subagent link/badge inline
7. User clicks the link → new pane opens (split from parent)
8. JSONL tail follows the file, streaming events to the pane
9. When subagent completes (no new events), pane shows "completed"
```

### Backend Components

#### SubagentWatcher Service

New Rust module: `agentmuxsrv-rs/src/backend/subagent_watcher.rs`

Responsibilities:
- Watch Claude Code session directories for `subagents/` subdirectories
- Detect new `agent-*.jsonl` files using `notify` crate (same pattern as `config_watcher_fs.rs`)
- Tail-follow each JSONL file, parsing new entries as they're appended
- Emit events via EventBus: `subagent:spawned`, `subagent:activity`, `subagent:completed`
- Track active subagents per parent session

```rust
struct SubagentWatcher {
    // Map of session_id -> Vec<SubagentInfo>
    sessions: HashMap<String, Vec<SubagentInfo>>,
    // File watchers per session directory
    watchers: HashMap<String, RecommendedWatcher>,
    // JSONL readers (tail -f style)
    readers: HashMap<String, JonslReader>,
}

struct SubagentInfo {
    agent_id: String,        // 7-char hex
    slug: String,            // human-readable name
    jsonl_path: PathBuf,     // full path to JSONL file
    block_id: Option<String>,// AgentMux block if pane created
    last_event: Instant,     // for completion detection
    status: SubagentStatus,  // active, completed, error
}
```

#### Session Directory Resolution

To watch the right directories, the watcher needs to know each agent's Claude Code config path:

```
Host agents:    ~/.config/claude-agentx/projects/{encoded-path}/
Container agents: (inside container) ~/.config/claude/projects/{encoded-path}/
```

The encoded path is the agent's workspace path with `/` replaced by `-` and `:` removed.

For container agents, the path inside the container maps to a Docker volume that the host can read.

#### JSONL Parsing

Each JSONL line is parsed into a simplified event for the frontend:

```rust
enum SubagentEvent {
    Text { content: String },
    ToolUse { name: String, input_summary: String },
    ToolResult { tool_name: String, output_preview: String, is_error: bool },
    Progress { tool_name: String, output: String, elapsed: f64 },
}
```

Only `assistant` and `progress` events are interesting for the activity feed. `user` events (tool results) provide context but can be collapsed.

### Frontend Components

#### SubagentPane View

New React component: `frontend/app/view/subagent/subagent-view.tsx`

A read-only scrolling activity feed showing:
- Subagent identity (agent ID, slug, model)
- Real-time stream of actions (text output, tool calls, progress)
- Token usage counter
- Status indicator (active/completed/error)
- Elapsed time

This is NOT a terminal emulator. It's a structured log view, similar to a CI pipeline output.

#### Subagent Links in Agent Pane

When the backend emits `subagent:spawned`, the parent agent's pane renders a clickable subagent indicator inline. This could be:
- A banner/badge embedded in the terminal output area (e.g., "Subagent ac1e917 spawned — click to observe")
- An entry in a collapsible sidebar or footer within the agent pane
- A sticker overlay (similar to existing terminal stickers)

When the user clicks a subagent link:

1. A new Block is created with metadata:
   ```json
   {
     "view": "subagent",
     "subagent:id": "ac1e917",
     "subagent:slug": "toasty-zooming-grove",
     "subagent:parent_agent": "Agent1",
     "subagent:session": "{sessionUuid}",
     "subagent:jsonl_path": "/path/to/agent-ac1e917.jsonl"
   }
   ```
2. The parent pane is split horizontally (subagent pane appears to the right)
3. The SubagentPane view mounts and subscribes to `subagent:activity` events

#### Pane Layout Strategy

For 1 subagent: Split parent pane horizontally (parent left, subagent right)
For 2 subagents: Parent left, two subagents stacked right
For 3+ subagents: Parent left (40%), subagents in a grid right (60%)

The user controls which subagents to observe. Not all subagent panes need to be open simultaneously — the links remain clickable even after completion for reviewing history.

### Event Flow

```
JSONL file append
  → notify crate detects modification
  → SubagentWatcher reads new lines
  → Parses JSONL entries into SubagentEvents
  → Publishes via EventBus
  → WebSocket push to frontend
  → SubagentPane re-renders with new events
```

Debounce: Batch JSONL reads at 200ms intervals to avoid thrashing on rapid writes.

---

## 6. Implementation Plan

### Phase 1: JSONL Watcher (Backend)

**Deliverables:**
- `SubagentWatcher` Rust module that watches session directories
- Detects new `subagents/agent-*.jsonl` files
- Tail-follows JSONL files and parses entries
- Emits events via EventBus: `subagent:spawned`, `subagent:activity`, `subagent:completed`
- RPC endpoints: `ListActiveSubagents`, `GetSubagentHistory`

**Technical approach:**
- Reuse `notify` crate pattern from `config_watcher_fs.rs`
- Use `tokio::io::BufReader` with file position tracking for tail-follow
- 200ms debounce on file change events
- Completion detection: no new JSONL entries for 60 seconds + last entry is assistant message without tool_use

### Phase 2: Subagent Pane (Frontend)

**Deliverables:**
- `SubagentPane` React component (read-only activity feed)
- Event subscription via `waveEventSubscribe` for `subagent:activity`
- Structured rendering of tool calls, text output, progress events
- Status badge, token counter, elapsed timer
- Scrollback with auto-scroll (pause on manual scroll)

**Technical approach:**
- New view type: `"subagent"` in block metadata
- Virtualized list for performance (subagents can produce thousands of events)
- Syntax highlighting for code in tool results (reuse Monaco's tokenizer)

### Phase 3: Subagent Links in Agent Pane

**Deliverables:**
- When `subagent:spawned` fires, render a clickable link/badge in the parent agent's pane
- Track active and completed subagents per agent pane
- Clicking a link creates a new subagent pane (split from parent)
- Show subagent status inline (active/completed) with summary info

**Technical approach:**
- Agent pane subscribes to `subagent:spawned` and `subagent:completed` events
- Maintain a list of subagents associated with the current agent
- Render clickable subagent indicators (banner, sidebar entry, or sticker)
- On click: use `createBlockSplitHorizontally()` to open the subagent view
- Use `layoutModel.splitVertical()` for stacking multiple open subagent panes

### Phase 4: Swarm Widget

The **Swarm** widget is a standalone top-level view — separate from Forge and the Agent interaction panes.

**Three widgets, three concerns:**

| Widget | Purpose | Scope |
|--------|---------|-------|
| **Forge** | Create and manage agents | Agent config, soul, MCP, env, skills |
| **Agent** | Interact with an agent | Terminal pane, direct conversation |
| **Swarm** | Agent analytics and search | Cross-agent status, history, tokens |

**Deliverables:**
- Standalone Swarm widget (own view type, not a Forge tab)
- Cross-agent overview: status, activity, token usage for all agents at a glance
- **Conversation history search** across all agents and sessions
- Session timeline: what each agent worked on, when, and what subagents it spawned
- Token usage breakdown per agent, per session, per subagent
- Activity heatmap: when are agents most active, which agents are idle

**Search capabilities:**
- Full-text search across all JSONL conversation histories
- Filter by agent, date range, tool type, model
- Search subagent conversations (find which subagent worked on what)
- Search tool outputs (find specific error messages, file paths, commands)
- Results link directly to the relevant agent pane or session

**Swarm widget layout:**
```
┌────────────────────────────────────────────────────┐
│  SWARM                                             │
├────────────────────────────────────────────────────┤
│  Search all conversations...                       │
│                                                    │
│  Agent Overview                                    │
│  AgentX  * active   12 sessions today  45k tokens  │
│  Agent1  * active    8 sessions today  32k tokens  │
│  Agent2  - idle      3 sessions today  18k tokens  │
│  Agent3  - offline   --                            │
│                                                    │
│  Active Subagents (3)                              │
│  AgentX > ac1e917  "Exploring codebase"  2m ago   │
│  Agent1 > a07d705  "Running tests"       30s ago  │
│  Agent1 > aeb1420  "Editing files"       15s ago  │
│                                                    │
│  Today's Activity          Token Usage (7d)        │
│  ----######-- 10:00        AgentX  ######## 340k  │
│  ############ 11:00        Agent1  ######   220k  │
│  ##########-- 12:00        Agent2  ###      110k  │
│  ##---------- 13:00        Agent3  #         40k  │
└────────────────────────────────────────────────────┘
```

**Technical approach:**
- New view type: `"swarm"` in block metadata (like `"term"` or `"forge"`)
- Index JSONL files into SQLite FTS5 for fast conversation search
- Background indexer runs on startup, watches for new session files
- RPC endpoints: `SearchConversations`, `GetAgentAnalytics`, `GetTokenUsage`
- Frontend: `frontend/app/view/swarm/swarm-view.tsx` with search bar, agent cards, activity charts
- Live updates via EventBus subscriptions (`subagent:spawned`, `subagent:completed`, agent registration events)

---

## 7. User Experience

### What the User Sees

1. **Normal workflow:** User opens AgentMux, has Agent1 in a terminal pane. Types a complex task like "Refactor the authentication module and update all tests."

2. **Subagent spawning:** Claude Code internally decides this needs parallel work. It spawns 2 subagents — one for refactoring, one for tests. Clickable subagent links appear in Agent1's pane showing their names and status.

3. **User-driven observation:** The user clicks a subagent link to open its activity pane. A new pane splits from the parent showing a live activity feed. The user can open one, both, or neither — they choose what to watch.

4. **Live observation:** The user watches the subagent work in real-time. Grep searches, file edits, tool results stream in. They can click the other subagent link to open it too.

5. **Completion:** Subagents finish. Their links in the parent pane update to show "completed" status. Open subagent panes show a green "completed" badge. The links remain clickable for reviewing history.

6. **Multi-agent:** The user has Agent2 in another tab doing something else. It also spawns subagents. Those links appear in Agent2's pane, not Agent1's. Everything is cleanly separated.

### What the User Does NOT Have to Do

- Configure subagent behavior or roles
- Manage task queues or assignment
- Resolve merge conflicts between subagents
- Monitor filesystem state or JSONL files
- Set up git worktrees or branches for subagents

Claude Code handles all of that internally. AgentMux just shows you what's happening.

---

## 8. Container Agent Considerations

For container agents (Agent1-5), the subagent JSONL files live inside the Docker container's filesystem. To watch them from the host:

**Option A: Docker volume mount** (preferred)
Mount the Claude Code config directory as a volume visible to the host:
```yaml
volumes:
  - agent1-claude-config:/home/agent/.config/claude
```
The host-side `SubagentWatcher` reads from the mounted volume.

**Option B: Backend inside container**
Each container runs its own `agentmuxsrv-rs` instance that watches locally and forwards events to the host AgentMux via WebSocket/HTTP.

**Option C: Log streaming**
The container streams JSONL lines to a host-side socket/pipe. Lightweight but requires custom plumbing.

**Recommended:** Option A for MVP. Docker volumes are already used by Claw for workspace isolation. Adding the Claude config directory to the mount list is a small config change.

---

## 9. Open Questions

1. **Subagent pane density.** For agents that spawn many subagents (5+), should we show all panes simultaneously or use a tabbed/carousel view within the subagent area?

2. **Nested subagents.** Can subagents spawn their own subagents? If so, how deep do we visualize? Suggestion: flat list (all subagents shown equally, regardless of nesting depth).

3. **Completed subagent retention.** How long do we keep completed subagent JSONL files in the watch list? Memory pressure vs. historical browsing. Suggestion: keep last 24 hours in memory, older ones accessible via Forge history.

4. **Container volume performance.** Docker volume mounts on Windows (via WSL2) can have filesystem event latency. Need to test `notify` crate behavior on cross-filesystem mounts. Fallback: periodic polling at 1-second interval.

5. **Privacy/filtering.** Should the subagent activity feed filter out sensitive content (secrets in tool results, file contents)? Or is the assumption that the user already has access to everything the agent sees?

---

## 10. Sources

- [Issue #101 — Swarm Orchestration Spec](https://github.com/agentmuxai/agentmux/issues/101)
- [Issue #102 — OpenClaw Spec](https://github.com/agentmuxai/agentmux/issues/102)
- [tmux-background-agents — Lightweight Parallel Agent Management](https://github.com/m13v/tmux-background-agents)
- Claude Code JSONL subagent files (direct analysis of session data)
- AgentMux source code analysis (file watcher, block creation, reactive system, Forge)
