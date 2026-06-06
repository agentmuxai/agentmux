# SPEC: Multi-Session Agent Fork

**Status:** Draft  
**Date:** 2026-06-06  
**Author:** AgentA  
**Based on:** Code introspection of actual implementation (see §2)

---

## 1. Problem

When a user has an active agent session running and opens the same agent definition
again from the agent pane picker, there is no mechanism to run it as an independent
parallel session. The current session-zone model keys on `definition_id`, so a second
open of the same definition reattaches to the same conversation rather than starting a
fresh parallel one.

Users want to run the same agent configuration concurrently on different tasks — e.g.
two independent coding tasks using the same "Senior Dev" agent — each with an isolated
working directory and independent conversation history.

---

## 2. Actual Architecture (code-verified)

### 2.1 The Agent Pane Picker Has Two Lists

**File:** `frontend/app/view/agent/components/AgentPicker.tsx`

**List A — My Agents (session instances)**  
Component: `MyAgentsList` (`frontend/app/view/agent/components/MyAgentsList.tsx`)  
Data: `ListRecentSessionsCommand` → `RecentSessionRow[]`  
Each row represents a **session instance** — an `AgentInstance` (concrete run) joined with
its definition. A single definition with two past runs appears as **two rows** (truly
per-instance, not per-definition). Clicking a row calls `onReattach(row)` →
`launchAgentDefinition({ continueOfInstanceId: row.instance_id })`.

**List B — Agent definitions (seeded, "+ New from template")**  
Rendered directly in `AgentPicker.tsx` as a card grid below the "+" header.  
Data: `ListAgentDefinitionsCommand({ is_seeded: 1 })` → `AgentDefinition[]`  
These are seeded agent definitions (blueprints — "Claude Code", "Codex CLI", etc.).
Clicking opens the create-from-template modal, which calls `AgentDefCreateFromTemplateCommand`
to create a NEW user-owned definition (`is_seeded=0`, `parent_id = template.id`), then
launches it fresh. Seeded definitions never appear in List A (they have no session instances).

### 2.2 Core Types (from `frontend/types/gotypes.d.ts`)

```typescript
// Blueprint / template
type AgentDefinition = {
    id: string;
    name: string;
    is_seeded: number;        // 1=template, 0=user-owned
    parent_id?: string;       // Forked-from definition id (v6+)
    branch_label?: string;    // Fork branch label (v6+)
    working_directory: string;
    provider: string;
    // ...model, tools, env, etc.
};

// A concrete run/session
type AgentInstance = {
    id: string;
    definition_id: string;    // FK → AgentDefinition
    parent_instance_id?: string;
    block_id?: string;        // UI pane this instance lives in
    session_id?: string;      // CLI session id (for --resume)
    status: string;           // "running" | "paused" | "stopped" | "crashed" | "detached"
    instance_name?: string;   // User-visible label
    working_directory?: string;
    identity_id?: string;
    memory_id?: string;
    // ...
};

// Combined view for List A
type RecentSessionRow = {
    instance_id: string;
    instance_name: string;
    definition_id: string;
    definition_name: string;
    working_directory: string;
    preview: string;           // Last user message text
    node_count: number;
    last_active_at: number;
    has_snapshot: boolean;
    // ...identity, memory fields
};
```

### 2.3 Session Zone Model (Option E)

Session state (conversation snapshot) is stored in a **zone keyed on `definition_id`**:

```
agent:<definition_id>:current/output.state.json   ← UI snapshot (for restore)
agent:<definition_id>:current/output              ← raw NDJSON stream (crash recovery)
agent:<definition_id>:archive:<unix_ms>/...       ← archived past sessions
```

**This is the critical constraint:** one active session zone per definition. A second
pane opening the same `definition_id` reads the same zone — no isolation.

### 2.4 No "Already Running" Detection Today

There is no check in `launchAgentDefinition()` (`agent-model.ts:276`) or anywhere in
the picker for whether a definition already has an active `AgentInstance`. Two panes
can silently reattach to the same session zone.

---

## 3. Proposed Solution: Definition Fork

### 3.1 Core Insight

The session zone is keyed on `definition_id`. The cleanest path to session isolation —
without restructuring the entire zone model — is to give each parallel run its **own
definition copy** (`is_seeded=0`, `parent_id` = original). The existing
`parent_id`/`branch_label` fields on `AgentDefinition` were added for exactly this
purpose (v6+) but never wired to a user-facing flow.

Each forked definition:
- Gets a new `definition_id` (new session zone key)
- Inherits all config from the parent (system prompt, model, tools)
- Gets an auto-incremented `branch_label` ("Senior Dev #2", "Senior Dev #3")
- Gets its own `working_directory` (separate workspace)
- Has `is_seeded=0` so it appears in List A (My Agents) as a new session instance after first run

### 3.2 When Fork Is Triggered

**Trigger point:** User clicks a `RecentSessionRow` in List A (My Agents) while that
definition already has an active instance in another open pane (block).

Detection in `launchAgentDefinition()` (`agent-model.ts`):
1. Query WOS for all open blocks with `view: "agent"` and `meta.agentId === definition_id`
2. If any such block exists and the instance's status is `"running"` → show fork prompt

**Fork prompt** (inline, non-modal, appears below the row in List A):

```
┌──────────────────────────────────────────────────────────┐
│ "Senior Dev" is already open in another pane.            │
│                                                          │
│  [Open new session]    [Switch to existing]              │
└──────────────────────────────────────────────────────────┘
```

- **"Open new session"** → fork flow (§3.3)
- **"Switch to existing"** → focus the pane that already has the agent open

### 3.3 Fork Flow

```
User clicks "Open new session"
  │
  ├─ 1. Backend: AgentDefinitionForkCommand({ source_id, branch_label })
  │      - Creates new AgentDefinition row:
  │          id = new UUID
  │          parent_id = source_id
  │          branch_label = auto-generated (§3.4)
  │          is_seeded = 0
  │          working_directory = auto-allocate new dir
  │          (all other fields copied from source)
  │      - Returns: new_definition_id
  │
  ├─ 2. Frontend: launchAgentDefinition({ definitionId: new_definition_id })
  │      - Normal launch flow: allocate workdir, spawn CLI, create AgentInstance
  │      - New session zone: agent:<new_definition_id>:current/...
  │
  └─ 3. List A (MyAgentsList) auto-refreshes via `agents:changed` broadcast
         - New fork appears as its own row with branch_label as display name
```

### 3.4 Auto-Naming

The `branch_label` for a fork is assigned by the backend at creation time:

```
parent definition name: "Senior Dev"
existing non-archived fork count under parent_id: N

  N=0 (no prior forks): branch_label = "Senior Dev #2"
  N=1:                  branch_label = "Senior Dev #3"
  ...
```

The original definition keeps its original name ("Senior Dev"). Only forks get the
`#N` suffix. Users can rename any definition from its context menu; the `branch_label`
is then overwritten with the user's choice.

### 3.5 Workspace Isolation

Each fork gets its own working directory. Options:

| Approach | Pros | Cons |
|----------|------|------|
| New dir under parent's workspace root: `<parent_workdir>/forks/<branch_label>/` | Easy to find related forks | Requires parent to have a workdir set |
| Fresh auto-allocated dir (same as new agent): `~/.agentmux/workspaces/<new_def_id>/` | Always works, clean isolation | Disconnected from parent |

**Default:** fresh auto-allocated dir (Option B). Users who want sibling forks can
configure workdir manually after creation. A `"Copy working directory contents"` checkbox
in the fork prompt (optional) lets users seed the new workspace from the parent's current
files.

---

## 4. UX Changes

### 4.1 List A — My Agents (session instances)

Each `RecentSessionRow` gets a context menu (right-click / "..." button on hover):
- **Fork → New session** — triggers §3.2 flow
- **Rename** — edit `instance_name`
- **Archive** — soft-delete (hides from list)
- **Open in new pane** — same as click but forces new pane

Forked sessions appear as their own rows in List A. They are visually linked to their
parent by showing the `parent_id`'s name in small text below the row name (e.g.
`fork of Senior Dev`) — only visible on hover to reduce noise.

### 4.2 List B — Agent definitions (seeded)

No change to List B. Seeded definitions (`is_seeded=1`) never have session instances
and always create a fresh user-owned definition. Their existing flow is already "create
new definition + launch."

### 4.3 "Already Open" Indicator

If a session instance is currently open in another pane, its row in List A shows a small
"active" badge (green dot / running indicator). This gives the user visual awareness
before clicking.

---

## 5. Backend Changes

### 5.1 New RPC: `AgentDefinitionForkCommand`

```rust
pub struct AgentDefinitionForkCommand {
    pub source_id: String,
    pub branch_label: Option<String>,  // None → auto-generate
    pub copy_workdir_contents: bool,   // false → empty new workdir
}

pub struct AgentDefinitionForkResponse {
    pub definition_id: String,
    pub branch_label: String,
    pub working_directory: String,
}
```

Handler in `agent_handlers.rs`:
1. Load source definition
2. Compute next branch_label (count non-archived forks with `parent_id = source_id`)
3. Insert new `db_agent_definitions` row (copy + new id + parent_id + branch_label)
4. Auto-allocate working_directory if `copy_workdir_contents=false`
5. If `copy_workdir_contents=true`: `fs::copy_dir(source.working_directory, new_dir)`
6. Broadcast `agents:changed`
7. Return response

### 5.2 "Active Instance" Query

New helper in `storage/agents.rs`:

```rust
pub fn definition_active_instance(
    conn: &Connection,
    definition_id: &str,
) -> Option<AgentInstance>;
```

Returns the most recent instance with `definition_id = ?` and `status IN ("running",
"paused")`. Used by the frontend detection in §3.2 via a new lightweight RPC, or
by the frontend querying WOS directly for open agent blocks.

### 5.3 No Session Zone Changes

The session zone model (§2.3) is unchanged. Forks get new `definition_id`s, so they
naturally get new zones (`agent:<new_def_id>:current`) without any refactoring of
the zone machinery.

---

## 6. Migration / Backwards Compatibility

- No DB migration needed for the fork flow itself — `parent_id` and `branch_label`
  columns already exist on `db_agent_definitions` (added in v6+).
- Existing agents are unaffected; they have `parent_id = NULL` and `branch_label = NULL`.
- `ListRecentSessionsCommand` already returns all user-owned instances; forks appear
  automatically once created.

---

## 7. Out of Scope (Follow-ups)

- **Conversation branching:** forking mid-conversation to explore two different reply
  paths (requires per-instance session zones, larger refactor)
- **Fork tree visualization:** showing the parent → fork → fork tree in the picker UI
- **Auto-fork setting:** per-definition toggle to always fork on open without prompting
- **Merge/sync:** syncing changes between fork and parent (out of scope, complex)
- **Session zone refactor to per-instance:** decouples zones from definitions entirely;
  would enable branching but is a large separate project

---

## 8. Open Questions

| # | Question | Default |
|---|----------|---------|
| 1 | Should fork prompt also appear when opening via template cards? | No — templates always create fresh; no ambiguity |
| 2 | Should the "Copy working directory contents" checkbox default to on or off? | Off — isolated by default, user opts in to copying |
| 3 | Should forked definitions be editable (change model, system prompt) independently of parent? | Yes — they are independent definitions after fork |
| 4 | Where does the "active in another pane" detection live — WOS query in frontend, or a backend RPC? | WOS query in frontend (already has block state) |
| 5 | Should the fork prompt appear on a session-instance row click (List A), or only in the context menu? | On click when active instance detected; context menu for explicit fork regardless |

---

## 9. Key Files

| File | Change |
|------|--------|
| `frontend/app/view/agent/components/MyAgentsList.tsx` | Add context menu, active-instance badge, fork prompt inline UI |
| `frontend/app/view/agent/agent-model.ts:276` | Add active-instance detection in `launchAgentDefinition()` |
| `frontend/app/view/agent/components/AgentPicker.tsx` | Pass active-instance map to MyAgentsList |
| `agentmux-srv/src/server/agent_handlers.rs` | Add `AgentDefinitionForkCommand` handler |
| `agentmux-srv/src/backend/storage/agents.rs` | Add `definition_active_instance()` helper |
| `frontend/types/gotypes.d.ts` | Add `AgentDefinitionForkCommand` / `AgentDefinitionForkResponse` types |
| `frontend/app/view/agent/rpc-api.ts` | Add `AgentDefinitionForkCommand` RPC wrapper |
