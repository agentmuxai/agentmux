# Spec: Named agent continuation — launch modal dropdown of existing agents

**Status:** Spec (no implementation yet)
**Owner:** AgentA
**Date:** 2026-05-12
**Driving requirement:** "When I launch an agent, the launch modal should let me select an agent I've already created and named, and continue working on it — same folder, same identity, same memory, same conversation history."

---

## 1. TL;DR

Every agent launch today already creates a persistent record (a `db_agent_instances` row, a working directory, env injection). The state is there. What's missing is the **UI affordance to find it again and pick up where it left off.**

This spec adds:

1. A **"Continue agent"** dropdown at the top of the launch modal listing every named agent the user has previously launched (most recent first).
2. A `ListNamedAgentsCommand` RPC that returns the dropdown's content.
3. A **resume path** in the spawn flow: when the user picks a past agent, the spawn reuses its working directory, identity binding, memory binding, and (where the CLI supports it) the prior conversation session.
4. A **persistence shim** that records the human-readable instance name on the `db_agent_instances` row so the dropdown has something to display besides UUIDs.

Phase 1 ships the dropdown + reuse of working dir + bundles. Phase 2 adds CLI session continuation (Claude `--continue`, etc.) once Phase 1 is steady.

---

## 2. Today's state

### Already exists

- **`db_agent_instances`** (v7 schema): `id`, `definition_id`, `parent_instance_id`, `block_id`, `session_id`, `status`, `github_context`, `started_at`, `ended_at`, `created_at`. Plus `identity_id` FK (per v7). One row per launch, kept forever.
- **Launch modal** (`frontend/app/view/agent/components/AgentLaunchModal.tsx`): user enters `instanceName`, picks runtime, picks Identity bundle, picks Memory bundle. Submitted as `LaunchOverrides`.
- **Spawn flow** (`server/app_api.rs::launch_forge_agent` ≈ line 1659): generates an instance id, calls `allocate_agent_workdir(instanceName)` to claim a working directory, writes agent config files (`CLAUDE.md`, `.mcp.json`, `.claude/settings.json`), spawns the CLI subprocess with `AGENTMUX_AGENT_ID`, `AGENTMUX_BLOCKID`, identity env, etc.
- **Identity/Memory** are first-class via PR #746/#749/#750/#751 — already pickable in the launch modal.

### Gap

- **No human-readable name stored on `db_agent_instances`.** The user-chosen `instanceName` becomes `AGENTMUX_AGENT_ID` in the spawned env and shapes the working-dir path, but isn't a queryable column. To list past agents by name we'd have to scrape env vars or parse working-dir paths.
- **No RPC to list past agents** ready for the launch modal.
- **No resume path** — every launch is a brand-new instance with a brand-new working dir (with `-2`, `-3` suffixes when name collides per `allocate_agent_workdir`).

---

## 3. Goals

1. **Find it.** Open the launch modal → see a "Continue agent" dropdown above the bundle pickers, listing all past named agents, sorted by recency. Each row shows: instance name, agent definition (Claude / Codex / Gemini / …), identity bundle name, memory bundle name, last-active timestamp, status badge (running / idle / done).
2. **Resume it.** Pick a row → modal switches to a "Continue" mode: instance name field is pre-filled and read-only, identity/memory pickers are pre-filled and read-only, working dir reuses the prior path, Launch button changes to "Continue". Submit → new pane spawns the CLI rooted in that working dir with the prior bindings.
3. **Tell them apart.** Running and stopped past agents are visibly distinct. Picking a running instance shows a warning ("This agent is currently running in another pane — continuing will open a new pane sharing the same working dir").
4. **Forget it.** A row-level "Forget" affordance (right-click → Forget agent) marks the instance hidden from the dropdown without deleting the working directory or DB row (audit trail).

## 4. Non-goals

- **Multi-user / multi-machine sync** of named agents — local DB only, this spec.
- **Branching / forking** a past agent into two (clone its config + working dir into a new instance). Possible follow-up, not in v1.
- **Editing a past agent's bundles** from the dropdown. If you want to change identity, launch new.
- **CLI session resume for non-Claude providers.** Codex and Gemini parity for `--continue` is a Phase 2 follow-up; Phase 1 ships Claude only.
- **Garbage collection of working directories.** Spec for retention/GC is separate.

---

## 5. Data model — small additive change

`db_agent_instances` gains three columns:

```sql
ALTER TABLE db_agent_instances
  ADD COLUMN instance_name TEXT NOT NULL DEFAULT '';
ALTER TABLE db_agent_instances
  ADD COLUMN working_directory TEXT NOT NULL DEFAULT '';
ALTER TABLE db_agent_instances
  ADD COLUMN display_hidden INTEGER NOT NULL DEFAULT 0;
```

Migration is a v8 schema bump. Backfill: leave `instance_name=""` and `working_directory=""` for historical rows; they don't appear in the dropdown. Future launches fill both from the LaunchOverrides + the resolved path returned by `allocate_agent_workdir`. Storing `working_directory` explicitly (rather than re-deriving from the slug at continue-time) is robust against future slug-rule changes and to user-side renames.

`identity_id` and `memory_id` already live on `db_agent_instances` from v7 — no need to re-add.

### Indexes

```sql
CREATE INDEX idx_agent_instances_name_recent
  ON db_agent_instances(instance_name, started_at DESC)
  WHERE display_hidden = 0 AND instance_name != '';
```

Supports the dropdown query in one b-tree scan.

---

## 6. RPC API

### `ListNamedAgentsCommand(filters?) → NamedAgentRow[]`

Returns all `db_agent_instances` rows with `instance_name != ''` AND `display_hidden = 0`, joined with `db_forge_agents` for the definition name. Sorted by `started_at DESC`. Capped at 200 by default (paginate later if needed).

Shape:

```ts
interface NamedAgentRow {
    instanceId: string;          // db_agent_instances.id
    instanceName: string;        // user-chosen name (AGENTMUX_AGENT_ID)
    definitionId: string;        // FK to db_forge_agents
    definitionName: string;      // resolved: "Claude Code", "Codex", "Gemini", ...
    provider: string;            // "claude" | "codex" | "gemini" | ...
    workingDir: string;          // resolved from allocate_agent_workdir
    identityId: string;          // "" if blank
    identityName: string;        // resolved bundle name or "(ambient creds)"
    memoryId: string;            // "" if blank
    memoryName: string;          // resolved bundle name or "(vanilla CLI)"
    startedAt: number;           // unix ms
    endedAt: number;             // unix ms (0 if still considered active)
    status: "running" | "idle" | "done";
    blockIdHint: string;         // the most recent block_id the user opened this in
}
```

### `HideNamedAgentCommand(instance_id)` — sets `display_hidden = 1`.

### `ContinueNamedAgentCommand(instance_id, target_pane_hint?) → { block_id, tab_id }`

Spawns a new block in the target tab (or current tab) using the prior instance's bindings. Calls the existing internal `launch_forge_agent` plumbing but with:

- `instanceName = row.instance_name`
- `identityId = row.identity_id`
- `memoryId = row.memory_id`
- `workingDirOverride = row.working_dir` — skip `allocate_agent_workdir`, reuse as-is
- `parent_instance_id = row.id` — chain the lineage so we can show "this is a continuation of <old>" later

Returns the new `block_id` so the frontend can focus it.

---

## 7. UX

### Launch modal — new dropdown at top

```
┌─ Launch Agent ─────────────────────────────────────────┐
│                                                        │
│  Continue agent:  ⏷ [— New agent —              ]    │  ← new
│                     ┌──────────────────────────────┐  │
│                     │ — New agent —                │  │
│                     │ ────────────────────────────  │  │
│                     │ ● Aria-streaming   2m ago    │  │
│                     │   Claude · personal · main   │  │
│                     │ ○ Bo-perf-test     1h ago    │  │
│                     │   Codex · work · vanilla     │  │
│                     │ ○ Cleo-pr-review   yesterday │  │
│                     │   Claude · personal · vault  │  │
│                     └──────────────────────────────┘  │
│                                                        │
│  Instance name:    [ Aria-streaming             ]     │  ← pre-filled, read-only when continue
│  Runtime:          ( ) Host  ( ) Container             │
│  Identity:         ⏷ [ personal                 ]     │  ← pre-filled, read-only when continue
│  Memory:           ⏷ [ main                     ]     │  ← pre-filled, read-only when continue
│                                                        │
│        [ Cancel ]              [ Continue → ]          │  ← label flips: Launch ↔ Continue
└────────────────────────────────────────────────────────┘
```

- **Default selection:** "— New agent —". Existing flow unchanged when this is selected.
- **Pick a past agent** → name + identity + memory fields auto-fill from the row and become **read-only**. Runtime stays editable (you may want host vs container to change).
- **Status badge** in the dropdown:
  - ● green = running somewhere right now
  - ○ gray = idle / done
- **Picking a running instance** surfaces a yellow inline warning: *"This agent is currently active in another pane. Continuing will open a new pane sharing the same working directory."*
- **Sort:** by `started_at DESC`. Pinning / favorites is a v2 ask, not in v1.
- **Empty state:** if there are no named agents yet, the dropdown is hidden entirely (don't waste space on the empty case).

### Right-click affordance — "Forget agent"

On a row inside the dropdown (or via the existing Agent settings panel): right-click → "Forget agent". Calls `HideNamedAgentCommand`. Row disappears from the dropdown. Working dir + DB row are preserved (audit + recovery).

A future Identity pane section could list hidden agents with "Unhide" — out of scope for this spec.

---

## 8. Spawn flow

### New agent (existing)

Unchanged. `launch_forge_agent` calls `allocate_agent_workdir(instanceName)` → claims free `~/agentmux-work/<slug>/`, may suffix `-2`, `-3` on collision. Writes new `db_agent_instances` row.

### Continue agent (new)

`ContinueNamedAgentCommand(instance_id)`:

1. Load `db_agent_instances` row by id.
2. Validate row: `instance_name != ''`, working dir still exists on disk, identity_id + memory_id still resolve (or fall back to "blank" with a logged warning).
3. Choose target tab/pane.
4. Insert a new `db_agent_instances` row with `parent_instance_id = old.id`, `instance_name = old.instance_name`, `identity_id`, `memory_id`, `definition_id` copied, fresh `id`/`block_id`/`session_id`, `status = "running"`, `started_at = now`.
5. Write agent config files **into the existing working dir** (idempotent — CLAUDE.md, .claude/settings.json, .mcp.json overwrite is fine; agent config is regenerated from bundles each spawn already).
6. Spawn the CLI subprocess with `cmd:cwd = old.working_dir`, env injection same as new-agent path, plus `AGENTMUX_RESUME_FROM = old.id` so the wrapper / CLI plumbing can hook session continuation later (Phase 2).
7. Return `{ block_id, tab_id }` to the frontend.

**Phase 2 (CLI continuation):** when `AGENTMUX_RESUME_FROM` is set AND the provider is Claude, append `--continue` to the spawn args. Other providers handled per their own resume flags.

---

## 9. Edge cases

| Case | Handling |
|---|---|
| Working dir deleted on disk | Surface error in modal: "Working directory `~/agentmux-work/X/` is gone — launch as new agent?" Offer fallback to new-agent flow with the same name. |
| Identity bundle deleted | Fall back to `blank` with logged warning; row still listed with "(missing identity)" tag. |
| Memory bundle deleted | Same as above. |
| Two running instances of same name | Allowed — `parent_instance_id` chains them; the second pane is a "second seat" at the same working dir. Users do this rarely but it's a real workflow. Yellow inline warning suffices. |
| Name collision with a *new* launch | If user types an existing instance name into the launch field while "— New agent —" is selected, modal nudges: "An agent named `X` exists — did you mean to continue it?" Click to switch the dropdown to that row. |
| Definition deleted (`db_forge_agents` cascade) | `db_agent_instances` rows cascade-delete per existing FK. Won't appear in dropdown. |
| Migration on a fresh install | Empty `db_agent_instances` → empty dropdown → modal looks identical to today. Zero regression risk. |
| Concurrent continue while old instance still running | Allowed (per row above). Resource isolation is the existing per-block model — no new contention. |

---

## 10. Phasing

| Phase | Scope | LoC est. |
|---|---|---|
| **0** | v8 schema migration: add 3 columns + index. Backfill `instance_name` from existing block meta where derivable. | ~80 |
| **1** | RPC + spawn changes: `ListNamedAgentsCommand`, `HideNamedAgentCommand`, `ContinueNamedAgentCommand`. Working-dir reuse path. Identity/memory bundle resolution still goes through resolver. | ~250 |
| **2** | Launch modal UI: dropdown component, status badges, pre-fill / read-only modes, "Continue" button label flip, inline warnings. | ~300 |
| **3** | Right-click "Forget" affordance in the dropdown + Identity-pane hidden-agents list. | ~100 |
| **4** *(separate PR)* | CLI session continuation: Claude `--continue` wired when `AGENTMUX_RESUME_FROM` is set. Codex / Gemini parity per their flags. | ~150 |

Phase 0+1+2 land together (the migration is useless without the UX, the UX is useless without the data). Phase 3 follows. Phase 4 is the big quality win — actual conversation continuity — but ships when we've validated the data flow.

---

## 11. Open questions

1. **Working directory location.** Today `allocate_agent_workdir` lives under `~/agentmux-work/<slug>/`. Continue flow reuses that path. If the user has moved/renamed it externally, we error. Should we offer to relocate? Probably v2.
2. **What counts as "active"?** `status = "running"` covers a live pane. But `block_id` could be stale if the host crashed without cleanup. Pragmatic: rely on the `pidregistry` running-process check at list time, downgrade stale `running` rows to `idle`. Audit log if the downgrade fires.
3. **Per-user vs per-machine.** Today `db_agent_instances` is the local DB — no cross-machine sync. That's fine. If we ever add identity vault sync (PR-G, task #33), named agents could ride alongside but it's out of scope here.
4. **Soft-delete vs filter-out.** `display_hidden = 1` is a soft delete. Do we need a hard "Delete agent + working dir" option? Yes eventually, but it's a destructive action — separate confirm flow, separate PR.
5. **Sub-agents.** `parent_instance_id` already exists. Should sub-agents appear in the dropdown? Probably no — they're scoped to their parent's session. Filter `parent_instance_id = ''` in the dropdown query.
6. **Conversation history surfacing.** Phase 4 enables Claude `--continue`. But what if the user wants to *see* the past conversation BEFORE deciding to continue? Could surface a peek-pane on hover (read `db_blockfiles` for the prior block's stream-json). Worth its own spec.

---

## 12. Cross-references

- v7 Identity schema: [`SPEC_IDENTITY_FORGE_INTEGRATION_AND_VAULT_2026_05_08.md`](identity-forge-integration-and-vault-2026-05-08.md) (link path may need adjusting)
- Launch modal rearchitecture: `docs/specs/launch-modal-rearchitecture-2026-05-01.md`
- `db_agent_instances` schema: `agentmux-srv/src/backend/storage/migrations.rs:410-424`
- Launch entry: `agentmux-srv/src/server/app_api.rs::launch_forge_agent`
- `allocate_agent_workdir`: same file
- Identity resolver: `agentmux-srv/src/identity/resolver.rs`
- Launch modal component: `frontend/app/view/agent/components/AgentLaunchModal.tsx`
