# Trust Center — Global Brain

**Date:** 2026-06-19  
**Status:** Draft  
**Relates to:** `SPEC_MEMORY_IDENTITY_ARCH_2026_06_19.md`, `SPEC_AGENT_PANE_MEMORY_IDENTITY_MODALS_2026_06_19.md`

---

## 1. What This Is

Every AgentMux agent inherits a shared workspace context at launch — rules, coding standards, project policies, tool configs. This is the **global brain**: the things every agent knows before it starts.

Currently this exists as a flat list of Memory bundles with `is_global=true` in `db_memory_bundles`. They get concatenated into every agent's `CLAUDE.md` at spawn. The problem: the Trust Center Memory tab shows global and per-agent bundles in the same unsorted list with no visual hierarchy. There is no surface that clearly says "here is what every agent in this workspace knows."

This spec defines a **Brain tab** in the Trust Center that presents the global brain as a first-class, unified concept — editable sections that compose into the workspace-wide `CLAUDE.md` all agents inherit.

---

## 2. Mental Model: Two Brains

```
┌─────────────────────────────────────┐
│  TRUST CENTER — Brain tab           │
│  "What every agent knows"           │
│                                     │
│  Workspace-wide shared context:     │
│  rules, policies, standards, tools  │
│  → injected into CLAUDE.md at every │
│    agent launch                     │
└─────────────────────────────────────┘
             ↓ inherited by all agents

┌─────────────────────────────────────┐
│  AGENT PANE — Brain icon            │
│  "What this agent has learned"      │
│                                     │
│  Per-agent autonomous memory:       │
│  facts Claude discovered, codebase  │
│  patterns, session insights         │
│  → ~/.claude/projects/<id>/memory/  │
└─────────────────────────────────────┘
             ↑ written by Claude during sessions
```

The same brain icon is used in both surfaces intentionally — they are the same concept at different scopes. Trust Center = workspace scope. Agent pane = agent scope.

---

## 3. Trust Center Brain Tab

### 3.1 Tab placement

In `BundleManagerModal`, alongside the existing Accounts tab. The current "Memory" tab is replaced by two tabs:

| Tab | Icon | Content |
|---|---|---|
| Accounts | `key` | Credential library (unchanged) |
| Brain | `brain` | Global workspace brain (this spec) |
| Bundles | `layer-group` | Per-agent config presets (renamed from Memory, non-global bundles only) |

The Bundles tab is a follow-on cleanup. The Brain tab is the new surface this spec defines.

### 3.2 Layout

```
┌──────────────────────────────────────────────────────────────────────────┐
│  Trust Center                                                    [✕]    │
├──────────────┬───────────────────────────────────────────────────────────┤
│  Accounts    │  Brain                                                    │
│  Brain     ● │  ─────────────────────────────────────────────────────── │
│  Bundles     │  Every agent inherits these sections at launch.           │
│              │  They compose into the agent's CLAUDE.md in order.       │
│              │                                                           │
│              │  ┌─────────────────────────────────────────────────────┐ │
│              │  │ ▶ Coding Standards          [Edit] [↑] [↓] [✕]     │ │
│              │  └─────────────────────────────────────────────────────┘ │
│              │  ┌─────────────────────────────────────────────────────┐ │
│              │  │ ▶ Security Rules            [Edit] [↑] [↓] [✕]     │ │
│              │  └─────────────────────────────────────────────────────┘ │
│              │  ┌─────────────────────────────────────────────────────┐ │
│              │  │ ▶ AgentMux Project Context  [Edit] [↑] [↓] [✕]     │ │
│              │  └─────────────────────────────────────────────────────┘ │
│              │                                                           │
│              │  [+ Add section]                                          │
│              │                                                           │
│              │  ── Preview ─────────────────────────────────────────── │
│              │  Combined CLAUDE.md that all agents receive:             │
│              │  [View combined ▾]                                        │
└──────────────┴───────────────────────────────────────────────────────────┘
```

### 3.3 Sections

Each section = one `is_global=true` Memory bundle. The global brain is the ordered list of all such sections, concatenated at agent launch.

**Section row (collapsed):**
- Section name (the bundle's `name`)
- `[Edit]` — expands inline editor
- `[↑]` `[↓]` — reorder (order controls injection order into CLAUDE.md)
- `[✕]` — remove from global brain (sets `is_global=false`, does NOT delete the bundle)

**Section row (expanded / editing):**
```
┌─────────────────────────────────────────────────────────────────────────┐
│ ▼ Coding Standards                              [Collapse] [✕ Remove]  │
│ ─────────────────────────────────────────────────────────────────────── │
│ Name:  [Coding Standards                                              ] │
│                                                                         │
│ Content:                                                                │
│ ┌─────────────────────────────────────────────────────────────────────┐ │
│ │ # Coding Standards                                                  │ │
│ │                                                                     │ │
│ │ - Use TypeScript strict mode                                        │ │
│ │ - No any types                                                      │ │
│ │ - ...                                                               │ │
│ └─────────────────────────────────────────────────────────────────────┘ │
│                                          [Cancel]  [Save]               │
└─────────────────────────────────────────────────────────────────────────┘
```

The content field maps to `Memory.instructions`. Context files, MCP servers, and skills from the bundle are also injected but are not editable in this view yet — they round-trip through the Bundles tab.

### 3.4 Add section

`[+ Add section]` opens a two-option popover:

- **New section** — creates a blank `is_global=true` bundle, opens inline editor immediately
- **Promote existing bundle** — shows a list of non-global bundles; selecting one sets `is_global=true` and adds it to the bottom of the list

### 3.5 Preview: combined CLAUDE.md

`[View combined ▾]` expands a read-only panel showing exactly what will be written into an agent's `CLAUDE.md` — sections concatenated in order, with a `## <section name>` heading separating each one.

This lets users verify the injection output before restarting agents.

---

## 4. Data Model

No schema changes needed. The global brain is entirely represented by existing `db_memory_bundles` rows with `is_global=true`.

**One missing piece: section order.**

Currently `bundle_memory_list_global()` returns global bundles ordered by `name ASC`. That's arbitrary. We need explicit ordering. Two options:

**Option A (simplest): add `sort_order` column to `db_memory_bundles`**
- New integer column, default 0
- Brain tab writes `sort_order` when user reorders
- `bundle_memory_list_global()` uses `ORDER BY sort_order ASC, name ASC`
- Migration: schema v8, set `sort_order = rowid` for existing global bundles

**Option B: store order as a JSON array on a workspace settings row**
- `db_workspace_settings` key `global_brain_order` = `["bundle-id-1", "bundle-id-2", ...]`
- No schema change to bundles table
- More fragile (IDs can go stale)

Recommendation: **Option A**. Clean, co-located with the data, trivial migration.

---

## 5. Injection Behavior (unchanged)

At agent spawn, `agent_config.rs` calls `bundle_memory_list_global()` and concatenates all `is_global=true` bundles into `CLAUDE.md` in the agent's working directory. This already works. The Brain tab is purely a UI change — it surfaces the same data more clearly.

One addition: global bundle sections should be separated by a heading in CLAUDE.md so Claude Code can distinguish them:

```markdown
# [Workspace] Coding Standards

<instructions from bundle>

---

# [Workspace] Security Rules

<instructions from bundle>
```

The `[Workspace]` prefix and `---` separator make it clear these are injected workspace rules, not the agent's own config. This is a backend change to `agent_config.rs`'s concatenation logic.

---

## 6. Effective Time

Changes to the global brain take effect **on the next agent restart**. The Brain tab shows a banner when unsaved changes exist relative to what running agents currently have:

> "Changes to the global brain take effect when agents restart. Running agents are using the version from their last launch."

This banner is advisory only — no forced restart.

---

## 7. Relationship to Agent Pane Brain

| | Trust Center Brain | Agent Pane Brain |
|---|---|---|
| Scope | All agents in the workspace | This agent only |
| Written by | Human (via Brain tab) | Claude Code (autonomously) |
| Content | Rules, standards, policies, tool configs | Discovered facts, codebase patterns, session insights |
| Storage | `db_memory_bundles` (is_global=true) | `~/.claude/projects/<sanitized>/memory/*.md` |
| Injected as | `CLAUDE.md` in working dir | `MEMORY.md` auto-loaded by Claude Code |
| Edit via | Trust Center → Brain tab | Agent pane → Brain modal (Phase 3) |
| Takes effect | Next agent restart | Next Claude session start |

**The invariant:** workspace brain = things humans intend for every agent. Agent brain = things Claude learned about this specific agent's context. Content should not drift between layers — a fact Claude discovers should not be promoted to the workspace brain unless the team explicitly decides it's a universal standard.

---

## 8. Files to Change

| File | Change |
|---|---|
| `frontend/app/modals/bundle-manager-modal.tsx` | Add Brain tab alongside Accounts; wire to new `GlobalBrainManager` component |
| `frontend/app/view/brain/global-brain-manager.tsx` | **NEW** — Brain tab UI: section list, inline editor, preview |
| `frontend/app/view/brain/global-brain-model.ts` | **NEW** — `GlobalBrainViewModel`: loads global bundles, handles reorder/add/remove/edit |
| `frontend/app/view/brain/global-brain.scss` | **NEW** — styles |
| `agentmux-srv/src/backend/storage/migrations.rs` | Schema v8: add `sort_order INTEGER NOT NULL DEFAULT 0` to `db_memory_bundles` |
| `agentmux-srv/src/backend/storage/memory_bundles.rs` | Update `bundle_memory_list_global()` to order by `sort_order ASC, name ASC`; add `bundle_memory_reorder()` |
| `agentmux-srv/src/server/app_api.rs` | Register `ReorderGlobalBrainCommand` |
| `agentmux-srv/src/backend/agent_config.rs` | Add `[Workspace]` heading + `---` separator between injected sections |

---

## 9. Out of Scope

- Per-agent bundle presets (non-global bundles) — handled separately in the Bundles tab
- MCP server / context file / skills editing within the Brain tab — those stay in the Bundles tab for now; only `instructions` is editable inline here
- Syncing the global brain to a git repo or external source — future work
- A "workspace-wide MEMORY.md" shared across all agents — each agent's native memory is intentionally isolated by working directory; sharing would require a separate mechanism and is not proposed here
