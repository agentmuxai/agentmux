# Swarm View Tree Redesign

**Date:** 2026-06-19  
**Status:** Draft  
**Author:** Claude (clamk-0612a)

---

## 1. Goal

Replace the current Swarm view (flat subagent list with Active/Retired tabs) with a **live two-level tree** where:

- **Level 1 (roots):** every active agent pane in the current window
- **Level 2 (children):** subagents spawned by each root agent during the current session

No tabs. No separate retired list. Status is communicated inline via row indicators. Completed subagents stay visible under their parent — they just look done.

---

## 2. Current State

### Problems with the current design

| Problem | Impact |
|---------|--------|
| Shows subagents only — no root agents | User can't see which Claude Code session each subagent belongs to at a glance |
| Active/Retired tab split | Hides completed subagents; forces tab switching to see history |
| 3D flip card to detail view | Slow; the key info (status, model, open button) fits inline |
| Flat list | Loses the parent→child relationship that gives context |
| No agent-level summary | No way to see "agent X has 3 active subagents" without counting |

### What exists

- **`subagent_watcher.rs`**: watches JSONL files under `~/.config/claude-{agent_id}/projects/{workspace}/subagents/`; emits `subagent:spawned`, `subagent:activity`, `subagent:completed` WPS events; each event carries `parentBlockId` linking the subagent to its agent pane
- **`AgentInstance` records**: one per agent pane (`block_id`, `status`, `started_at`); has `parent_instance_id` for nested spawning (not used today)
- **`AgentTrackedBlocksCommand`**: RPC returning `block_id[]` for panes with active process trackers
- **`term:activity` block meta**: written by `useBlockActivity` (PR #1577) — the Claude Code session topic label (e.g. "auth refactor")
- **`ActiveSubagent` / `SubagentInfo`**: existing data shape, linked to parent via `parentBlockId`

---

## 3. New Design

### 3.1 Information Architecture

```
Swarm
├── 🤖 Agent pane: "auth refactor"          [running ●] [Open]
│   ├── toasty-zooming-grove                [running ●] [Open]
│   └── fizzy-lunar-bridge                  [done   ✓] [Open]
│
├── 🤖 Agent pane: "fix CI pipeline"        [running ●] [Open]
│   └── calm-river-delta                    [running ●] [Open]
│
└── 🤖 Agent pane                           [idle   –] [Open]
    (no subagents yet)
```

**Root row (agent pane):**
- Robot icon (🤖 or inline SVG bot icon, matches agent widget icon)
- Label: `term:activity` value if set (e.g. "auth refactor"), else `"Agent pane"` fallback
- Status chip right-aligned: `running ●` / `idle –` / `done ✓`
- Open button (on hover or always visible at right edge)

**Child row (subagent):**
- Indented 20px
- Subagent slug (human-readable name, e.g. "toasty-zooming-grove")
- Status chip right-aligned: `running ●` / `done ✓`
- Open button (on hover)

### 3.2 Status Indicators

| State | Chip text | Dot color | Dot animation | Opacity |
|-------|-----------|-----------|---------------|---------|
| Agent running | `running` | `--warning-color` (amber) | pulse 1.5s | 100% |
| Agent idle (process exited, pane open) | `idle` | `--secondary-text-color` (gray) | none | 80% |
| Subagent active | `running` | `--warning-color` | pulse 1.5s | 100% |
| Subagent completed | `done` | `--success-color` (green) | none | 65% |

No separate tab for completed subagents — they stay inline under their parent, dimmed. This preserves session history without hiding it.

### 3.3 Row Height and Density

- Root row: **36px** (slightly taller, acts as section header)
- Child row: **30px** (compact)
- No connector lines (VS Code Explorer style — cleaner at sidebar width)
- 20px left indent for children
- Rows are not collapsible in v1 (all agents visible by default; revisit if users have >10 agents)

### 3.4 Expand/Collapse (v1: always expanded)

In v1, all agent rows are always expanded. The tree is short (typically 1–5 agents, 0–10 subagents each) so collapsing is unnecessary complexity. A future v2 can add chevrons if the tree grows.

### 3.5 Actions

- **Clicking an agent row** → focuses that agent pane (calls existing `focusBlock` / `setActiveTab`)
- **Clicking a subagent row** → calls `openSubagentPane(...)` (existing logic in `subagent-pane-manager.ts`)
- **"Open" button** on right edge of each row → same as click, but always visible for discoverability

### 3.6 Empty State

If there are no agent panes at all: show `"No active agent panes"` centered text.

If an agent has no subagents yet: show a soft `"No subagents yet"` child row at 65% opacity — helps users understand the tree structure before any subagents appear.

---

## 4. Data Model

### 4.1 Tree Node Types

```typescript
interface AgentTreeNode {
    blockId: string;
    label: string;             // term:activity or "Agent pane"
    status: "running" | "idle" | "done";
    subagents: SubagentTreeNode[];
}

interface SubagentTreeNode {
    agentId: string;
    slug: string;
    parentBlockId: string;
    status: "active" | "completed";
    model: string | null;
    lastEventAt: number;
}
```

### 4.2 Tree Construction

```
agentTreeAtom: AgentTreeNode[] =
    for each block_id in AgentTrackedBlocksCommand result:
        label   = block meta["term:activity"] ?? "Agent pane"
        status  = shellprocstatus === "running" ? "running" : "idle"
        subagents = subagentsAtom
                        .filter(s => s.parentBlockId === block_id)
                        .sort((a, b) => b.lastEventAt - a.lastEventAt)
```

**Note:** Agent panes without a process tracker (stopped but pane still open) are not included in v1. They can be added later via `AgentInstances` RPC filtered by `block_id` presence.

### 4.3 Live Update Strategy

| Event | Action |
|-------|--------|
| `subagent:spawned` | Add child node under matching parent; re-sort that parent's children |
| `subagent:completed` | Flip child status to `"completed"` |
| `subagent:activity` | Update `lastEventAt` on child |
| `controllerstatus` (shellprocstatus=running) | Mark agent root as `"running"` |
| `controllerstatus` (shellprocstatus=done) | Mark agent root as `"idle"` |
| `block:activity` (term:activity WPS) | Update agent root label |
| agent pane opened/closed | Reload `AgentTrackedBlocks` list |

No full list re-render on events — update only the affected node. SolidJS signals make this cheap with granular `createSignal` per node.

---

## 5. Component Structure

```
SwarmView (swarm-view.tsx)
├── SwarmTreeRoot
│   └── For each AgentTreeNode:
│       ├── AgentRow          ← root-level row
│       │   ├── BotIcon
│       │   ├── AgentLabel    ← term:activity or fallback
│       │   ├── StatusChip
│       │   └── OpenButton
│       └── For each SubagentTreeNode:
│           └── SubagentRow   ← indented child row
│               ├── SubagentSlug
│               ├── StatusChip
│               └── OpenButton
│
└── EmptyState (if agentTree is empty)
```

### 5.1 Reuse / What to Delete

| Current file | v2 action |
|-------------|-----------|
| `swarm-view.tsx` | Rewrite — remove 3D flip, Active/Retired tabs, flat list |
| `swarm-model.ts` | Extend: add `agentTreeAtom`, subscribe to `controllerstatus` events per pane |
| `swarm-view.scss` | Rewrite — new compact tree row styles; remove flip animation |
| `subagent-pane-manager.ts` | Keep as-is — `openSubagentPane` still called from subagent rows |

---

## 6. Visual Design

### 6.1 Row Anatomy (agent root)

```
┌────────────────────────────────────────────────────┐
│ 🤖  auth refactor              running ●  [Open]  │  36px
└────────────────────────────────────────────────────┘
```

### 6.2 Row Anatomy (subagent child)

```
┌────────────────────────────────────────────────────┐
│     toasty-zooming-grove         running ●  [Open] │  30px  (20px indent)
└────────────────────────────────────────────────────┘
```

### 6.3 CSS Sketch

```scss
.swarm-tree-root {
    display: flex;
    flex-direction: column;
    gap: 0;
    overflow-y: auto;
    padding: 8px 0;
}

.swarm-agent-row {
    display: flex;
    align-items: center;
    height: 36px;
    padding: 0 8px 0 10px;
    gap: 6px;
    cursor: pointer;
    border-radius: 4px;

    &:hover {
        background: var(--hover-bg);
        .swarm-open-btn { opacity: 1; }
    }

    .swarm-agent-label {
        flex: 1;
        font-size: 13px;
        font-weight: 500;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
}

.swarm-subagent-row {
    display: flex;
    align-items: center;
    height: 30px;
    padding: 0 8px 0 30px; /* 10 + 20 indent */
    gap: 6px;
    cursor: pointer;
    border-radius: 4px;
    opacity: 1;

    &.completed { opacity: 0.65; }

    &:hover {
        background: var(--hover-bg);
        .swarm-open-btn { opacity: 1; }
    }

    .swarm-subagent-slug {
        flex: 1;
        font-size: 12px;
        font-family: var(--monospace-font);
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }
}

.swarm-status-chip {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 11px;
    color: var(--secondary-text-color);
    flex-shrink: 0;

    .swarm-status-dot {
        width: 6px;
        height: 6px;
        border-radius: 50%;
        background: currentColor;

        &.running {
            background: var(--warning-color);
            animation: swarm-pulse 1.5s ease-in-out infinite;
        }
        &.done    { background: var(--success-color); }
        &.idle    { background: var(--secondary-text-color); }
    }
}

.swarm-open-btn {
    opacity: 0;
    transition: opacity 100ms;
    font-size: 11px;
    padding: 2px 6px;
    border-radius: 3px;
    border: 1px solid var(--border-color);
    color: var(--secondary-text-color);
    background: transparent;
    cursor: pointer;
    flex-shrink: 0;

    &:hover { background: var(--hover-bg); color: var(--primary-text-color); }
}

@keyframes swarm-pulse {
    0%, 100% { opacity: 1; }
    50%       { opacity: 0.4; }
}

.swarm-no-subagents {
    padding: 0 8px 0 30px;
    height: 26px;
    display: flex;
    align-items: center;
    font-size: 11px;
    color: var(--secondary-text-color);
    opacity: 0.65;
    font-style: italic;
}
```

---

## 7. SwarmModel Changes

```typescript
// New signal
agentTreeAtom: Accessor<AgentTreeNode[]>

// New subscriptions (in constructor, inside onMount):
// 1. controllerstatus per each tracked block
// 2. block:activity per each tracked block (for label updates)
// 3. subagent:spawned / subagent:completed / subagent:activity (already exist)

// New methods:
buildTree(): AgentTreeNode[]
    // Merge agentBlockIds + subagentsAtom into tree

loadAgentBlocks(): Promise<void>
    // Calls AgentTrackedBlocksCommand, fetches block meta for each
    // (to get term:activity)
```

---

## 8. Implementation Plan

### Phase 1 — Model (backend neutral, frontend only)
1. Extend `SwarmModel` with `agentTreeAtom` built from `AgentTrackedBlocksCommand` + existing `subagentsAtom`
2. Subscribe to `controllerstatus` WPS events for each tracked block to update running/idle
3. Subscribe to `block:activity` (`block:activity` WPS, from PR #1577) to update labels reactively

### Phase 2 — View
1. Replace `swarm-view.tsx` with new tree layout (`AgentRow` + `SubagentRow`)
2. Remove 3D flip card and Active/Retired tabs
3. Write new SCSS; remove old card + flip styles

### Phase 3 — Polish
1. Empty state row ("No subagents yet") per agent
2. "Open" button hover affordance
3. Visual QA at 240px, 300px, 400px swarm pane widths

---

## 9. Out of Scope (v1)

- **Collapse/expand**: all nodes always visible in v1
- **Multi-level nesting**: subagents spawning their own subagents (not observed in practice today)
- **Search/filter**: too small a list to need it yet
- **Drag to reorder**: agents are listed in spawn order (newest first)
- **Retired-only filter**: completed subagents are always visible inline, dimmed

---

## 10. Open Questions

1. **Agent pane ordering**: newest-first (matches current behavior) or by `term:activity` alphabetical? → Recommend newest-first (consistent with subagent ordering, predictable).
2. **Agent panes not in `AgentTrackedBlocks`** (idle/stopped but pane still open): include via `AgentInstances` RPC filter? → v2; v1 only shows panes with active trackers.
3. **Subagent row click vs open button**: should clicking the row navigate to the pane? → Yes, consistent with agent row click behavior.
