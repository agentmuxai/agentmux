# Spec: Agent Pane Bottom Action Bar
**Date:** 2026-04-22  
**Status:** Draft  
**Scope:** Frontend (SolidJS) + Backend (Rust RPC)  

---

## 1. Goal

Add a persistent, always-visible action bar pinned to the bottom of the agent pane containing three buttons:

1. **Add Agent** — create a new forge agent
2. **Import Agents** — bulk import agents from a JSON file
3. **Export Agents** — bulk export all forge agents to a JSON file

This gives users a consistent entry point for agent management directly from the agent pane, without navigating to the Forge pane. It also makes the portable seeding system accessible from the UI — the import/export format is compatible with `forge-seed.json`.

---

## 2. Visual Design

### Placement

The bar is appended as the last child of `.agent-view` (the root flex column), below `.agent-composer-region`. It is **always visible** regardless of whether the AgentPicker or a running agent session is showing.

```
┌─────────────────────────────────────────┐
│  .agent-view (flex column)              │
├─────────────────────────────────────────┤
│  .agent-document (flex: 1, scrollable)  │
├─────────────────────────────────────────┤
│  .agent-composer-region                 │
│  └─ AgentFooter (textarea + hint)       │
├─────────────────────────────────────────┤  ← NEW
│  .agent-action-bar                      │
│  [ + Add Agent ][ ↓ Import ][ ↑ Export ]│
└─────────────────────────────────────────┘
```

### Button Row

```
┌──────────────────────────────────────────────────────┐
│  [+ Add Agent]          [↓ Import]       [↑ Export]  │
└──────────────────────────────────────────────────────┘
```

- Three equal-width buttons, `flex: 1`, with a gap between them
- Subtle styling — secondary/ghost buttons, not primary CTA weight
- Icons: `+` (add), `↓` (import), `↑` (export) — use existing icon system or unicode
- Border-top separates bar from composer region
- Bar height: ~32px (compact, does not compete with conversation area)
- Font size: 12px, matching `.agent-input-hint` scale
- No hover tooltip required at MVP; can be added later

### SCSS Pattern (follows existing agent-view.scss conventions)

```scss
.agent-action-bar {
    display: flex;
    flex-direction: row;
    gap: 4px;
    padding: 4px 6px;
    border-top: 1px solid var(--border-color);
    background: color-mix(in srgb, var(--main-text-color) 3%, transparent);
    flex-shrink: 0;

    .agent-action-btn {
        flex: 1;
        display: flex;
        align-items: center;
        justify-content: center;
        gap: 4px;
        height: 24px;
        font-size: 12px;
        color: var(--secondary-text-color);
        background: transparent;
        border: 1px solid var(--border-color);
        border-radius: 4px;
        cursor: pointer;
        white-space: nowrap;
        transition: background 0.1s, color 0.1s;

        &:hover {
            background: color-mix(in srgb, var(--main-text-color) 8%, transparent);
            color: var(--main-text-color);
        }

        &:active {
            background: color-mix(in srgb, var(--main-text-color) 14%, transparent);
        }

        &.agent-action-btn-disabled {
            opacity: 0.4;
            cursor: default;
            pointer-events: none;
        }
    }
}
```

---

## 3. Button Behaviours

### 3.1 Add Agent

**Trigger:** Click `+ Add Agent`

**Behaviour:**
1. Call `createforgeagent` RPC with minimal defaults (name: `"New Agent"`, provider: `"claude"`, icon: `"✦"`)
2. On success: open the AgentFocusedPanel to the Forge tab for the newly created agent (same as clicking ⚙ on an agent card)
3. Focus the agent name field so the user can immediately rename

**RPC:** `createforgeagent` (already exists — `CommandCreateForgeAgentData` in rpc_types.rs)

**Error handling:** Show a transient error toast if RPC fails. No modal.

---

### 3.2 Import Agents

**Trigger:** Click `↓ Import`

**Behaviour:**
1. Open a native file picker (`<input type="file" accept=".json">`) — triggered programmatically, no visible input element in DOM
2. User selects a `.json` file
3. Parse the file client-side; validate it matches the import format (see §4)
4. Show a preview modal:
   - List of agents to be imported (name, provider, icon)
   - Warning count: "N agents already exist and will be skipped" (match by slug)
   - Options: `[Cancel]` `[Import N agents]`
5. On confirm: call `importforgeagents` RPC (new — see §5.1) with parsed payload
6. On success: broadcast `forgeagents:changed` event → picker refreshes automatically
7. Show a brief success banner: "Imported N agents"

**Error handling:**
- Invalid JSON → show inline error: "Invalid file — expected AgentMux export format"
- Version mismatch → show warning: "Format version X — some fields may be ignored"
- Partial failure → show which agents failed to import

---

### 3.3 Export Agents

**Trigger:** Click `↑ Export`

**Behaviour:**
1. Call `exportforgeagents` RPC (new — see §5.2) — returns full agent list with content and skills
2. Construct a JSON blob matching the import format (see §4)
3. Trigger browser download:
   - Filename: `agentmux-agents-{YYYY-MM-DD}.json`
   - MIME type: `application/json`
   - Encoding: UTF-8 (same pattern as existing session export via `atob`)
4. Show a brief success banner: "Exported N agents"

**Loading state:** Button shows spinner/disabled state while RPC is in flight.

**Error handling:** Toast on RPC failure.

---

## 4. Import/Export JSON Format

The format is intentionally **compatible with `forge-seed.json`** so exports can be dropped in as seeds and vice versa. Version field allows future evolution.

```json
{
  "version": 4,
  "exported_at": "2026-04-22T10:00:00Z",
  "source": "agentmux-export",
  "agents": [
    {
      "id": "agentx",
      "name": "AgentX",
      "icon": "🔴",
      "description": "Primary coding agent",
      "provider": "claude",
      "shell": "pwsh",
      "working_directory": "",
      "agent_bus_id": "agentx",
      "agent_type": "host",
      "environment": "windows",
      "restart_on_crash": false,
      "content": {
        "soul": "...",
        "agentmd": "...",
        "startup": "...",
        "env": "AGENT_NAME=agentx\nAGENTMUX_AGENT_ID=AgentX",
        "mcp": "",
        "hooks": ""
      },
      "skills": [
        {
          "name": "Startup Verification",
          "trigger": "startup",
          "skill_type": "prompt",
          "description": "Re-run tool verification checks",
          "content": "..."
        }
      ]
    }
  ]
}
```

**Fields omitted from export** (runtime/identity state, not portable):
- `id` (UUID — regenerated on import; `slug`/`agent_bus_id` used for dedup)
- `created_at`
- `parent_id`, `branch_label`
- `is_seeded`
- `accounts` (identity — contains auth refs, not exported for security)

**Dedup on import:** match by `id` field (slug). If an agent with the same `id` already exists → skip and report. No merge/overwrite at MVP.

---

## 5. New Backend RPC Commands

### 5.1 `importforgeagents`

**Handler:** `forge_handlers.rs` — new function `handle_import_forge_agents()`

**Request:**
```rust
pub struct CommandImportForgeAgentsData {
    pub agents: Vec<ForgeAgentImport>,  // parsed from JSON
}

pub struct ForgeAgentImport {
    pub id: String,          // used as slug for dedup check
    pub name: String,
    pub icon: String,
    pub description: String,
    pub provider: String,
    pub shell: String,
    pub working_directory: String,
    pub agent_bus_id: String,
    pub agent_type: String,
    pub environment: String,
    pub restart_on_crash: bool,
    pub content: HashMap<String, String>,   // content_type → content
    pub skills: Vec<ForgeSkillImport>,
}

pub struct ForgeSkillImport {
    pub name: String,
    pub trigger: String,
    pub skill_type: String,
    pub description: String,
    pub content: String,
}
```

**Response:**
```rust
pub struct ImportForgeAgentsResult {
    pub imported: Vec<String>,   // names of agents successfully imported
    pub skipped: Vec<String>,    // names of agents skipped (already exist)
    pub failed: Vec<String>,     // names of agents that failed
}
```

**Logic:**
1. For each agent in payload:
   - Check if `db_forge_agents` has a row with `slug = agent.id` → skip if found
   - Otherwise: insert ForgeAgent row, then insert ForgeContent rows, then insert ForgeSkill rows
2. Fire `forgeagents:changed` event after all inserts
3. Return result summary

---

### 5.2 `exportforgeagents`

**Handler:** `forge_handlers.rs` — new function `handle_export_forge_agents()`

**Request:** empty (`{}`)

**Response:**
```rust
pub struct ExportForgeAgentsResult {
    pub version: u32,
    pub exported_at: String,   // ISO 8601
    pub agents: Vec<ForgeAgentExport>,
}

pub struct ForgeAgentExport {
    // all ForgeAgent fields except id, created_at, parent_id, branch_label, is_seeded, accounts
    pub id: String,            // slug used as export id
    pub name: String,
    pub icon: String,
    pub description: String,
    pub provider: String,
    pub shell: String,
    pub working_directory: String,
    pub agent_bus_id: String,
    pub agent_type: String,
    pub environment: String,
    pub restart_on_crash: bool,
    pub content: HashMap<String, String>,
    pub skills: Vec<ForgeSkillExport>,
}
```

**Logic:**
1. Call `wstore.forge_list()` to get all agents
2. For each agent: call `wstore.forge_content_get_all(agent_id)` and `wstore.forge_skills_list(agent_id)`
3. Assemble `ForgeAgentExport` — use `slug` as the export `id`, omit identity/runtime fields
4. Return assembled payload

---

## 6. Component Structure

### New component: `AgentActionBar.tsx`

```
frontend/app/view/agent/components/AgentActionBar.tsx   ← NEW
frontend/app/view/agent/agent-view.scss                 ← ADD .agent-action-bar styles
frontend/app/view/agent/agent-view.tsx                  ← ADD <AgentActionBar> at bottom
agentmux-srv/src/server/forge_handlers.rs               ← ADD handle_import/export_forge_agents
agentmux-srv/src/backend/rpc_types.rs                   ← ADD command structs
frontend/app/store/rpc-api.ts                           ← ADD ImportForgeAgentsCommand, ExportForgeAgentsCommand
```

### AgentActionBar.tsx skeleton

```tsx
import { createSignal } from "solid-js";
import { RpcApi } from "@/store/rpc-api";
import { TabRpcClient } from "@/store/global";

export function AgentActionBar() {
    const [importing, setImporting] = createSignal(false);
    const [exporting, setExporting] = createSignal(false);

    async function handleAddAgent() {
        await RpcApi.CreateForgeAgentCommand(TabRpcClient, { /* defaults */ });
        // open forge panel for new agent
    }

    async function handleImport() {
        // trigger file picker → parse → preview modal → RpcApi.ImportForgeAgentsCommand
    }

    async function handleExport() {
        setExporting(true);
        try {
            const result = await RpcApi.ExportForgeAgentsCommand(TabRpcClient, {});
            // trigger download
        } finally {
            setExporting(false);
        }
    }

    return (
        <div class="agent-action-bar">
            <button class="agent-action-btn" onClick={handleAddAgent}>
                + Add Agent
            </button>
            <button class="agent-action-btn" onClick={handleImport} disabled={importing()}>
                ↓ Import
            </button>
            <button class="agent-action-btn" onClick={handleExport} disabled={exporting()}>
                ↑ Export
            </button>
        </div>
    );
}
```

---

## 7. Import Preview Modal

A lightweight modal (not a full page) shown before confirming import:

```
┌────────────────────────────────────────────┐
│  Import Agents                             │
├────────────────────────────────────────────┤
│  Found 7 agents in file:                   │
│                                            │
│  ✅ 🔴 AgentX — Primary coding agent       │
│  ✅ 🟡 AgentY — Secondary coding agent     │
│  ⏭️  🔵 AgentZ — Already exists, skip      │
│  ✅ 🌙 AgentK — Kimi Code CLI agent        │
│  ✅ 🟢 Agent1 — Sandboxed coding agent     │
│  ✅ 🟠 Agent2 — Sandboxed coding agent     │
│  ✅ 🟣 Agent3 — Sandboxed coding agent     │
│                                            │
│  6 will be imported · 1 will be skipped    │
├────────────────────────────────────────────┤
│  [Cancel]              [Import 6 Agents]   │
└────────────────────────────────────────────┘
```

Use existing overlay/modal patterns in the codebase (check `AgentFocusedPanel` or any existing modal component for the pattern to reuse).

---

## 8. Out of Scope (MVP)

- Merge/overwrite existing agents on import (dedup is skip-only at MVP)
- Selective export (export subset of agents)
- Import from URL
- Import format version migration
- Agent identity/account data in export (security risk — always omitted)
- Drag-and-drop import
- Undo/undo-import

---

## 9. File Change Summary

| File | Change |
|------|--------|
| `frontend/app/view/agent/components/AgentActionBar.tsx` | **CREATE** — new component |
| `frontend/app/view/agent/agent-view.tsx` | **EDIT** — add `<AgentActionBar>` as last child of `.agent-view` |
| `frontend/app/view/agent/agent-view.scss` | **EDIT** — add `.agent-action-bar` + `.agent-action-btn` styles |
| `frontend/app/view/agent/components/ImportPreviewModal.tsx` | **CREATE** — import confirmation modal |
| `frontend/app/store/rpc-api.ts` | **EDIT** — add `ImportForgeAgentsCommand`, `ExportForgeAgentsCommand` |
| `agentmux-srv/src/backend/rpc_types.rs` | **EDIT** — add `CommandImportForgeAgentsData`, `ImportForgeAgentsResult`, `ExportForgeAgentsResult`, `ForgeAgentExport`, `ForgeAgentImport` |
| `agentmux-srv/src/server/forge_handlers.rs` | **EDIT** — add `handle_import_forge_agents()`, `handle_export_forge_agents()`, register in `register_v6_handlers()` |

---

## 10. Relationship to Portable Seeding System

The export format is intentionally a superset of `forge-seed.json`:

- An **export** from the UI can be dropped directly into `agentmux-srv/forge-seed.json` (after removing `exported_at`/`source` fields) and recompiled
- A **forge-seed.json** can be imported via the UI without modification
- This closes the loop: the visual agent management system and the binary seed manifest share a common format

Future: expose a "Set as default seed" option that writes the export to a `forge-seed.override.json` file in the data directory, allowing runtime seed customization without recompilation.
