# Spec: Host vs Container Agent Differentiation
**Date:** 2026-06-18  
**Status:** Ready for implementation  
**Author:** parko-0617i

---

## 1. Problem Statement

AgentMux has full backend support for both host and container agents (`spawn_turn` vs
`spawn_container_turn`) but treats them as interchangeable in the UI. Users cannot tell at
a glance whether a running pane is sandboxed or has direct host access, and the launch modal
defaults to host mode — the opposite of what almost every user should want.

This spec covers:
- Making container the **default** runtime when creating any agent
- **Warning** when host mode is selected (it grants full system access)
- **Visual differentiation** (badge + color) in every surface that shows an agent
- No backend changes required — spawn logic, branch routing, and ContainerManager are
  all production-ready

---

## 2. Background: Current Implementation State

### 2.1 What is already fully implemented

**Backend (Rust):**
- `spawn_turn()` — runs Claude Code CLI as a host subprocess (no isolation)
- `spawn_container_turn()` — runs via Docker socket API; env vars passed through
  `CreateExecOptions.env` (not argv), preventing CWE-214 credential leakage
- `ContainerManager::ensure_running()` — idempotent create/start/pull lifecycle
  (cross-platform: Windows named pipe, macOS via `docker context inspect`, Linux socket)
- Branch logic in `agent_handlers.rs:3205-3208`: reads `block.meta["agentMode"]`
  and routes to the correct spawn path

**Frontend data model:**
- `LaunchForm.runtime: "host" | "container"` — the form field exists
- `AgentLaunchModal` renders a host/container radio toggle (lines 748–768)
- `launchAgentDefinition()` writes `agentMode: overrides?.agentType ?? agent.agent_type ?? "host"`
  into block meta (`agent-model.ts:523`)
- `agent_type: "host" | "container"` persisted in the `agent_definitions` DB row
- `AgentDefCard` (forge view) already renders `HOST` / `CONTAINER` text badges via
  `forge-agent-type-badge forge-agent-type-{host|container}` classes

**Security:**
- Container: credentials injected via Docker socket API; host env never reaches the agent process
- Host: full host process environment — same access as the OS user running AgentMux

### 2.2 What is missing

| Gap | Location | Notes |
|-----|----------|-------|
| Runtime defaults to `"host"` | `types.ts:51` — `initialForm()` | Should be `"container"` |
| New user-defined agent defaults to `"host"` | `app_api.rs:2528` | Should be `"container"` |
| No warning on host selection | `AgentLaunchModal.tsx:750` | Need an amber callout |
| No runtime badge on the running pane | `agent-view.tsx` | Use a `PaneRow` pin (see §5.2) |
| No runtime badge on My Agents cards | `AgentCard.tsx` | `agent_type` not currently displayed |
| Seeded templates are all `"host"` | `scripts/gen-seed.js` | 6 of 7 should become `"container"` |

---

## 3. Design: Runtime Semantics

### 3.1 Container agent (default)

- Runs inside `ghcr.io/agentmuxai/agent-claude:<version>` (or a custom image)
- Claude Code process reaches only `/workspace` (bind-mounted by AgentMux at launch)
- Credentials live in a named Docker volume (`agentmux-claude-<slug>`) — isolated per agent
- Host env vars never reach the agent process
- Container persists between turns (`tini + sleep infinity`); turns run via `docker exec`
- **Use for:** all regular coding work, untrusted repos, customer-facing tasks, anything
  that touches production systems

### 3.2 Host agent

- Runs as a direct subprocess of the AgentMux server process
- Inherits the full host process environment (all env vars, filesystem paths, credentials)
- No filesystem isolation — working dir is whatever `cmd:cwd` is set to
- **Use for:** sysadmin/DevOps tasks (e.g. managing Docker itself), tasks that need host
  package managers, tasks requiring GUI/desktop access, AgentMux maintenance agents
- **Requires explicit user opt-in with a clear warning**

### 3.3 Security model

```
Container ─── isolated ──── /workspace only + Docker socket API
Host      ─── full access ─ identical to the OS user running AgentMux
```

---

## 4. Design: Color Scheme

Adapted from a5af/dev-tools `AGENT_COLORS.md` conventions, mapped to AgentMux design tokens.

| Runtime | Color | Value | Maps to existing token |
|---------|-------|-------|------------------------|
| Container | Green | `#22c55e` | `var(--success-color)` (used in PERMISSION_COLORS for "plan" mode) |
| Host | Amber | `#f59e0b` | `var(--warning-color)` (used in PERMISSION_COLORS for "acceptEdits" mode) |
| Host warning callout border | Red | `#ef4444` | `var(--error-color)` |

Green = sandboxed/safe. Amber = elevated host access.  
No new CSS tokens needed — `--success-color` and `--warning-color` are already used by
`AgentControlBar.tsx:36-42` for permission-mode colors and carry the right semantic meaning.

---

## 5. Design: UI Changes

### 5.1 RuntimeBadge component (new)

**Location:** `frontend/app/view/agent/components/RuntimeBadge.tsx`

A tiny pill rendered identically across all surfaces. Uses FontAwesome icons (the existing
icon system — see `frontend/util/util.ts:100-131` for the `makeIconClass` pattern):

- Container → `fa-solid fa-box` — box/package = sandboxed
- Host → `fa-solid fa-server` — server = native machine

```tsx
interface RuntimeBadgeProps {
    runtime: "host" | "container";
    size?: "sm" | "md";  // sm = 10px, for card rows; md = 12px, for PaneRow
}

// Renders:
//   container: [  box  Container] — green pill
//   host:      [server Host     ] — amber pill
```

CSS: inline `background: var(--success-color)` / `var(--warning-color)`, white text,
4px border-radius, 4px 8px padding. No new SCSS file needed — style can live in
`AgentLaunchModal.scss` or a small co-located `RuntimeBadge.scss`.

### 5.2 Running pane — PaneRow runtime pin

**File:** `frontend/app/view/agent/agent-view.tsx` (around line 1139)

The `AgentControlBar` is **Claude-only** (`if (providerId !== "claude") return null`)
and is hidden inside the collapsible `agent-composer-details` panel — not a valid
surface for persistent metadata.

The correct pattern is a **`PaneRow`** placed at the same level as the failure-recovery
row (already at lines 1139-1156 of `agent-view.tsx`). `PaneRow` is the established
"auxiliary pin" primitive for the agent pane; the runtime indicator is exactly this kind
of persistent, low-prominence contextual row.

```tsx
// In agent-view.tsx, above or below the failure-recovery PaneRow:
<Show when={agentMode() && agentMode() !== "standalone"}>
    <PaneRow
        sigil={agentMode() === "container" ? "□" : "⚙"}
        title={agentMode() === "container" ? "Container" : "Host — full system access"}
        accent={agentMode() === "container" ? "done" : "idle"}
    />
</Show>
```

- `accent="done"` → green left-border for container (safe/isolated)
- `accent="idle"` → amber left-border for host (elevated, but not an error)
- `accent="neutral"` → no color (fallback for unknown/standalone)

Data source: `agentMode` from block meta. Read via
`useBlockMetaKeyAtom(blockId, "agentMode")` — already written at launch in
`agent-model.ts:523`.

### 5.3 My Agents list — AgentCard badge

**File:** `frontend/app/view/agent/components/AgentCard.tsx`

Add `<RuntimeBadge runtime={props.agent.agent_type} size="sm" />` to the card row,
after the agent name, before the timestamp/session status area. Data source:
`agent.agent_type` from `AgentDefinition` (already available in card props).

The forge view (`AgentDefCard.tsx`) already has a text-only `HOST`/`CONTAINER` label
pattern — `RuntimeBadge` brings the same information to the agent pane's My Agents list
in a visually consistent, color-coded form.

### 5.4 AgentLaunchModal — default + warning

**Files:**
- `frontend/app/store/launch-flow-state/types.ts`
- `frontend/app/view/agent/components/AgentLaunchModal.tsx`

#### 5.4.1 Default to container (`types.ts:51`)

```ts
// Before
runtime: "host",

// After
runtime: "container",
```

The `Opened` command carries `initial?.runtime` from the stored `agent_type`, so
re-launching an existing host agent still pre-selects host. Only the "new agent"
creation path is affected by the default change.

#### 5.4.2 New user-defined agent default (`app_api.rs:2528`)

```rust
// Before
agent_type: "host".to_string(),

// After
agent_type: "container".to_string(),
```

This is the Rust create path for user-defined agents (not from templates). The DB
column default is `'standalone'`; this explicit assignment is what sets the effective
type for all user-created definitions.

#### 5.4.3 Host warning callout (`AgentLaunchModal.tsx`)

Show below the radio group when `runtime() === "host"`:

```
┌────────────────────────────────────────────────────────┐
│ ⚙  Host access                                          │
│    This agent runs directly on your machine with full   │
│    access to your files, environment variables, and     │
│    credentials. Use only for admin or system-level      │
│    tasks. Container mode is recommended for all regular │
│    work.                                                │
└────────────────────────────────────────────────────────┘
```

Style: `border-left: 3px solid var(--warning-color)`, amber icon, muted background.
Not a gate — user can still proceed.

#### 5.4.4 Radio group label update

Current labels: `"Host"` / `"Container"`  
New labels (add subtext below each radio):

```
○  Container  (recommended)     ○  Host — full system access
   Isolated Docker sandbox          Runs as your OS user
```

Subtext: 10px, `var(--secondary-text-color)`. No layout change needed — just add
a `<span class="agent-launch-modal-radio-sub">` under each existing radio label.

### 5.5 Template seed defaults (`scripts/gen-seed.js`)

Change `agent_type` on templates based on `containerSupported` in `cli-catalog.ts` —
that field is already the source of truth for which CLIs can run in a container:

| Template | Provider | containerSupported | New agent_type |
|----------|----------|--------------------|----------------|
| Claude Code | claude | true | **container** |
| Codex CLI | codex | true | **container** |
| Gemini CLI | gemini | true | **container** |
| Kimi Code | kimi | true | **container** |
| Pi | pi | true | **container** |
| GitHub Copilot | copilot | true | **container** |
| OpenClaw | openclaw | false (needs host ACP) | host (unchanged) |

6 of 7 flip to container. The 1 that stays host (`openclaw`) is explicitly
`containerSupported: false` in the catalog — it is an ACP orchestrator that requires
host-level access to manage other agents.

The re-seed engine (`agent_seed.rs`) deletes old seeded rows and inserts new ones on
version bump — bump `schemaVersion` in `gen-seed.js` to trigger the reseed.

Existing user-owned agents in the DB are unaffected — the reseed only touches rows
where `is_seeded = 1`.

---

## 6. Implementation Plan

### Phase 1 — Visual differentiation (purely additive, no behavior change)

1. `RuntimeBadge.tsx` — new component (+ co-located SCSS)
2. `agent-view.tsx` — add runtime PaneRow pin (lines ~1137–1156 area)
3. `AgentCard.tsx` — add `<RuntimeBadge>` to My Agents card row

~150 LOC frontend. Zero risk to existing flows.

### Phase 2 — Default to container + host warning

1. `types.ts:51` — `runtime: "container"`
2. `app_api.rs:2528` — `agent_type: "container".to_string()`
3. `AgentLaunchModal.tsx` — host warning callout + radio subtext
4. `scripts/gen-seed.js` — update 6 templates to `"container"`, bump schemaVersion

~80 LOC frontend, ~5 LOC Rust, ~1 LOC seed script.

### Phase 3 — Agent definition editor (deferred)

Add runtime selector to `AgentCardSettingsPanel.tsx` so users can change `agent_type`
on an existing definition without recreating it. Requires a new
`UpdateAgentDefinitionCommand` RPC on the backend.

Not blocking — users can recreate agents to change runtime today.

---

## 7. Files to Change

### Phase 1

| File | Change |
|------|--------|
| `frontend/app/view/agent/components/RuntimeBadge.tsx` | **NEW** — pill badge component |
| `frontend/app/view/agent/components/RuntimeBadge.scss` | **NEW** — badge styles |
| `frontend/app/view/agent/agent-view.tsx` | Add runtime PaneRow pin (~line 1139) |
| `frontend/app/view/agent/components/AgentCard.tsx` | Add `<RuntimeBadge>` to card row |

### Phase 2

| File | Change |
|------|--------|
| `frontend/app/store/launch-flow-state/types.ts:51` | `runtime: "container"` |
| `agentmux-srv/src/server/app_api.rs:2528` | `agent_type: "container".to_string()` |
| `frontend/app/view/agent/components/AgentLaunchModal.tsx` | Host warning + radio subtext |
| `scripts/gen-seed.js` | Update 6 templates + bump schemaVersion |

### Phase 3

| File | Change |
|------|--------|
| `frontend/app/view/agent/components/AgentCardSettingsPanel.tsx` | Runtime selector UI |
| `agentmux-srv/src/server/agent_handlers.rs` | `UpdateAgentDefinition` handler |

---

## 8. What We Do NOT Change

- Backend spawn logic — both paths are production-ready
- `agentMode` block meta field name — stable
- `agent_type` DB column — stable  
- Container image (`ghcr.io/agentmuxai/agent-claude`) — shipping since v0.46.4
- DB column default (`'standalone'`) — stays; the create path sets the explicit value
- Launch flow reducer — only `initialForm()` default changes in Phase 2

---

## 9. Resolved Design Decisions

**Q1: AgentControlBar vs PaneRow for badge placement?**  
→ **PaneRow in `agent-view.tsx`**. `AgentControlBar` is Claude-only and hidden inside
the collapsible details panel — it's not a valid persistent surface. `PaneRow` is the
established auxiliary-pin primitive (already used for failure-recovery at line 1139)
and is always visible.

**Q2: Which icon set?**  
→ **FontAwesome Solid**, which is the existing icon system. `fa-box` for container
(sandboxed), `fa-server` for host (native machine). Both glyphs exist in the FA free
set. Render via `<i class="fa-solid fa-box" />` pattern used throughout the codebase.

**Q3: How many templates, which ones change?**  
→ 7 seeded templates (claude, codex, gemini, kimi, pi, copilot, openclaw).
6 flip to `"container"` (those with `containerSupported: true` in `cli-catalog.ts`).
1 stays `"host"`: openclaw (needs host-level ACP access to orchestrate other agents).
The `containerSupported` flag in `cli-catalog.ts` is the single source of truth — no
divergence between the catalog and the seed data.

**Q4: agent_type migration for existing rows?**  
→ No migration needed. Existing user agents keep their current `agent_type`; the
`Opened` command in the launch modal carries `initial.runtime = agent.agent_type`,
so they always re-launch in the same mode they were created with. Only new agents
(created after Phase 2 ships) default to `"container"`. Seeded template rows are
reset on every re-seed (version bump) — that's the correct mechanism.

---

## 10. References

- `agentmux-srv/src/backend/blockcontroller/subprocess.rs` — `spawn_turn` (346–851), `spawn_container_turn` (876–1235)
- `agentmux-srv/src/backend/container.rs` — `ContainerManager` lifecycle (670 LOC, cross-platform)
- `agentmux-srv/src/server/agent_handlers.rs:3205-3208` — host/container branch
- `agentmux-srv/src/server/app_api.rs:2528` — user agent create path (current "host" default)
- `agentmux-srv/src/backend/storage/migrations.rs:141` — `agent_type TEXT NOT NULL DEFAULT 'standalone'`
- `frontend/app/store/launch-flow-state/types.ts:49-51` — `initialForm()` + runtime default
- `frontend/app/view/agent/components/AgentLaunchModal.tsx:743-768` — current radio group
- `frontend/app/view/agent/agent-view.tsx:1139-1156` — failure-recovery PaneRow (placement reference)
- `frontend/app/view/agent/components/AgentControlBar.tsx:60-165` — Claude-only, gated on `providerId`
- `frontend/app/view/agent/agent-model.ts:523` — `agentMode` written to block meta at launch
- `frontend/app/view/agent/defaults/cli-catalog.ts` — `containerSupported` per provider (source of truth)
- `frontend/app/view/agent-def/components/AgentDefCard.tsx:38-51` — existing HOST/CONTAINER text badge (forge view)
- `scripts/gen-seed.js:142` — current `agent_type: "host"` in seed
- `docs/specs/SPEC_CONTAINER_PANE_SUPPORT_2026_06_11.md` — container architecture spec
- `docker/Dockerfile.agent-agentmux` — container image definition
- a5af/dev-tools: `docker/AGENT_COLORS.md` — color conventions (green=container, amber/red=host)
