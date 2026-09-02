# Spec: Forge Widget

## Status: APPROVED — implementing
**Date:** 2026-03-10

---

## Overview

The Forge is a top-bar widget (like Agent, Terminal, Sysinfo) that opens a pane
where users create, edit, and delete agents. Agents created in the Forge appear
in the Agent pane picker. Both widgets are always visible in the top bar.

```
┌─────────────────────────────────────────────────────────────────┐
│  [agent] [terminal] [sysinfo] [forge]          [help][⚙][<>]   │
└─────────────────────────────────────────────────────────────────┘
```

---

## Agent Configuration

Each Forge agent has:

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `id` | string (UUID) | auto | Generated on create |
| `name` | string | yes | Display name in picker |
| `icon` | string | yes | Emoji (default: ✦) |
| `provider` | string | yes | `"claude"` \| `"codex"` \| `"gemini"` |
| `description` | string | no | Subtitle shown in picker card |
| `created_at` | i64 | auto | Unix ms timestamp |

Phase 1 only. System prompt and working directory are Phase 2 extensions.

---

## Forge Pane UI

### Agent list (default state)

```
┌─────────────────────────────────────────────────────────────────┐
│  Forge                                                          │
│  ─────────────────────────────────────────────────────────────  │
│                                                                  │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │ ✨  Claude Coder               Claude Code    [Edit][✕]  │   │
│  └──────────────────────────────────────────────────────────┘   │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │ 🤖  My Researcher              Codex CLI      [Edit][✕]  │   │
│  └──────────────────────────────────────────────────────────┘   │
│                                                                  │
│  [ + New Agent ]                                                 │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### Create / Edit form (inline, replaces list)

```
┌─────────────────────────────────────────────────────────────────┐
│  Forge  /  New Agent                                            │
│  ─────────────────────────────────────────────────────────────  │
│                                                                  │
│  Icon    [ ✨ ]  (emoji picker or free-type)                    │
│                                                                  │
│  Name    [________________________]                              │
│                                                                  │
│  Provider                                                        │
│    ◉  ✨  Claude Code    (claude --output-format stream-json)   │
│    ○  🤖  Codex CLI      (codex --full-auto)                   │
│    ○  💎  Gemini CLI     (gemini --yolo)                       │
│                                                                  │
│  Description (optional)                                          │
│  [________________________]                                      │
│                                                                  │
│  [ Save ]  [ Cancel ]                                           │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### Empty state

```
┌─────────────────────────────────────────────────────────────────┐
│  Forge                                                          │
│  ─────────────────────────────────────────────────────────────  │
│                                                                  │
│              ✦                                                   │
│         No agents yet                                           │
│      Create your first agent                                    │
│                                                                  │
│              [ + New Agent ]                                    │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

---

## Data Flow

```
Forge pane                    Backend (agentmuxsrv-rs)           Agent pane
──────────                    ────────────────────────           ──────────
Create agent ──► CreateForgeAgent ──► INSERT db_forge_agents ──► WPS event
Edit agent   ──► UpdateForgeAgent ──► UPDATE db_forge_agents ──► WPS event
Delete agent ──► DeleteForgeAgent ──► DELETE db_forge_agents ──► WPS event
                                                                      │
                                                         useForgeAgents
                                                         re-renders picker
                                                         with updated list
```

The Agent picker subscribes to the `forgeagents:changed` WPS event and refreshes
its list whenever agents are created, updated, or deleted.

---

## Backend Changes

### New DB table: `db_forge_agents` (in `migrations.rs`)

```sql
CREATE TABLE IF NOT EXISTS db_forge_agents (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    icon TEXT NOT NULL DEFAULT '✦',
    provider TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL DEFAULT 0
);
```

Not a WaveObj — simple CRUD table, no version tracking needed.

### New RPC commands (in `rpc_types.rs` + `websocket.rs`)

| Command | Input | Output |
|---------|-------|--------|
| `listforgeagents` | `{}` | `ForgeAgent[]` |
| `createforgeagent` | `{ name, icon, provider, description }` | `ForgeAgent` (with generated id) |
| `updateforgeagent` | `{ id, name, icon, provider, description }` | `ForgeAgent` |
| `deleteforgeagent` | `{ id }` | `null` |

All mutating commands broadcast `forgeagents:changed` WPS event after DB write.

### ForgeAgent struct (in `waveobj.rs` or new `forgeagent.rs`)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForgeAgent {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub provider: String,
    pub description: String,
    pub created_at: i64,
}
```

---

## Frontend Changes

### New files

```
frontend/app/view/forge/
  ├── forge.tsx          # barrel export (attach viewComponent to avoid circular import)
  ├── forge-model.ts     # ForgeViewModel class
  ├── forge-view.tsx     # ForgeView component (list + form)
  └── forge-view.scss    # styles
```

### Modified files

| File | Change |
|------|--------|
| `frontend/app/block/block.tsx` | Add `BlockRegistry.set("forge", ForgeViewModel)` |
| `agentmuxsrv-rs/src/config/widgets.json` | Add `defwidget@forge` entry |
| `frontend/app/view/agent/agent-view.tsx` | Replace mock `getProviderList()` with `useForgeAgents()` hook |
| `agentmuxsrv-rs/src/backend/storage/migrations.rs` | Add `db_forge_agents` table |
| `agentmuxsrv-rs/src/backend/rpc_types.rs` | Add 4 new command constants + data structs |
| `agentmuxsrv-rs/src/server/websocket.rs` | Register 4 new handlers |

### `useForgeAgents` hook

```typescript
// Queries ListForgeAgentsCommand on mount, subscribes to WPS forgeagents:changed,
// re-queries on change event.
function useForgeAgents(): ForgeAgent[] { ... }
```

### Agent picker wiring

```typescript
// Before (Phase 1 — hardcoded):
const agents = getProviderList();

// After (Phase 2):
const agents = useForgeAgents();
// If agents.length === 0 → empty state + "create in Forge" button
// Launch: model.launchAgent(agent.id) with agent's provider + outputFormat
```

The `launchAgent()` method in `agent-model.ts` needs to accept a full `ForgeAgent`
(or look it up by id from the forge agent list) to get provider + outputFormat.

### Widget entry (`widgets.json`)

```json
"defwidget@forge": {
    "display:order": -6,
    "icon": "hammer",
    "color": "#a78bfa",
    "label": "forge",
    "description": "Create and manage your agents",
    "blockdef": {
        "meta": {
            "view": "forge"
        }
    }
}
```

---

## Implementation Order

1. **Backend first** — migrations, struct, commands, handlers, WPS event
2. **Forge view** — ForgeViewModel + ForgeView (list + form), register in BlockRegistry, add to widgets.json
3. **Wire Agent picker** — replace `getProviderList()` with `useForgeAgents()`, update `launchAgent()` to use ForgeAgent data
4. **Test end-to-end** — open Forge, create agent, switch to Agent pane, see it in picker, launch it
