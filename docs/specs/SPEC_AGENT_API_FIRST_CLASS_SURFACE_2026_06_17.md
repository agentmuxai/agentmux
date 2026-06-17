# Agent API: First-Class Surface (naming, layout, identity, introspection)

**Date:** 2026-06-17
**Status:** Draft
**Author:** naki
**Related:**
- `docs/analysis/REPORT_AGENT_WINDOW_NAMING_2026_06_17.md` (the window-naming finding that triggered this)
- `docs/specs/SPEC_OPENEDITOR_FLOATING_AND_COLLAPSED_TREE_2026_06_16.md` (#1497 — floating pane / collapse_tree, the most recent API add)
- `docs/specs/SPEC_WINDOW_TITLE_FORMAT_2026-05-13.md` (title resolution)

---

## 1. Premise

Agents running inside AgentMux are **first-class actors**, not guests. They should be able to shape their own workspace the way a human can: name things, open/close/focus panes and tabs, and introspect the layout they live in. Today the surface is ad-hoc (5 MCP tools) on top of a powerful-but-undocumented generic gateway. Since we're already extending the API (window naming was the immediate ask), this spec defines a **coherent first-class periphery** and the conventions future additions follow.

This spec is deliberately scoped to abilities whose **backend already exists** — we are surfacing and hardening, not building new subsystems.

---

## 2. What exists today (verified)

### 2.1 Two transports, one auth

Agents get `AGENTMUX_LOCAL_URL` (sidecar base) + `AGENTMUX_AUTH_KEY` (`X-AuthKey`). Every route except `GET /` is auth-gated (`mod.rs` `auth_middleware`).

- **MCP tools** (`agentmux-mcp/src/main.rs`) — the ergonomic surface agents actually reach for: `Shell`, `ShellStop`, `OpenEditor`, `SendMessage`, `DiscoverAgents`. Each is a thin POST to a sidecar route.
- **`POST /agentmux/service`** (`service.rs::handle_service`) — a **generic gateway** to the same reducer/service layer the frontend uses. Body is `WebCallType { service, method, args }`; it runs `dispatch_service(...)` and **broadcasts resulting `WaveObjUpdate`s to the event bus** so the UI reflects agent-driven changes live.

### 2.2 The gateway already exposes 47 methods

`dispatch_service` (`service.rs`) handles, among others:

| Domain | Methods (service.method) |
|---|---|
| **Object/meta** | `object.UpdateObjectMeta`, `object.UpdateTabName`, `object.UpdateObject`, `object.CreateBlock`, `object.DeleteBlock`, `object.GetObject(s)` |
| **Workspace/tab** | `workspace.CreateTab`, `CloseTab`, `SetActiveTab`, `ReorderTab`, `MoveTabToWorkspace`, `MoveBlockToTab`, `PromoteBlockToTab`, `TearOffBlock`, `TearOffTab`, `RedockFloatingPane`, `RestoreTornOffTab`, `CreateWorkspace`, `DeleteWorkspace`, `UpdateWorkspace`, `ListWorkspaces`, `GetWorkspace` |
| **Window** | `window.CreateWindow`, `CloseWindow`, `GetWindow`, `SetWindowPosAndSize`, `SwitchWorkspace`, `client.FocusWindow` |
| **Agent/other** | `agent.define`, `subagent.*`, `history.*`, `block.GetControllerStatus`, `userinput.SendUserInputResponse` |

**Implication:** the abilities the user wants (set window name, tab name, workspace name, manage tabs/panes) are **already reachable over HTTP today** via `object.UpdateObjectMeta` (e.g. `window:displayname`, `frame:title`, `tab:color`), `object.UpdateTabName`, and `workspace.UpdateWorkspace`.

### 2.3 So what's actually missing

The gap is **not transport** — it's that the gateway is **unergonomic, undiscoverable, unscoped, and unsafe** for agent use:

1. **No self-context.** To rename "my tab" an agent must already know its `tab_id`, `window_id`, `workspace_id`, and the `oref` envelope format. It only has `AGENTMUX_BLOCKID` / `AGENTMUX_AGENT_ID`. There is no "who/where am I" call.
2. **No verbs.** Agents must hand-craft reducer calls (`object.UpdateObjectMeta` with the right meta key) — implementation detail leakage. No MCP tools wrap them, so in practice agents never discover them.
3. **No safety model.** `window.CloseWindow` and `workspace.DeleteWorkspace` are as reachable as a rename. Nothing distinguishes a benign self-scoped rename from a destructive global op.
4. **The launch-time naming case is unsolved.** `task dev` spawns a *separate* instance with its own sidecar/window; no env/flag seeds its name (see the window-naming report §4–5).

---

## 3. Goals / Non-goals

**Goals**
- Give agents a small, **discoverable, ergonomic, self-scoped** set of first-class verbs for naming, layout, and introspection.
- Establish **conventions** (self-context defaulting, capability tiers, MCP-tool-wraps-REST-wraps-gateway) so future API growth is consistent.
- Solve the **launch-time naming** case for `task dev` and any spawned instance.

**Non-goals**
- No new rendering/window subsystems — we wrap existing `dispatch_service` methods.
- No remote/cross-instance control (an agent only acts on **its own** instance; cross-instance stays muxbus messaging).
- Not exposing every one of the 47 methods — only the curated, safe subset below.

---

## 4. Design

### 4.1 Layering convention

```
MCP tool  (ergonomic, agent-facing, self-defaulting)
   └── REST verb  /api/v1/...   (clean, typed, documented)
          └── dispatch_service  (existing reducer/service layer)
```

New abilities are added as a **REST verb** (typed, testable) plus a **thin MCP tool** that defaults the target to the agent's own context. The verb internally calls the existing `dispatch_service` method — no reducer changes.

### 4.2 Self-context (foundation — build first)

**`GET /api/v1/self`** → resolves the calling agent's place in the tree from `AGENTMUX_BLOCKID`:

```json
{
  "agent_id": "naki",
  "block_id": "…",          "block_title": "naki",
  "tab_id": "…",            "tab_name": "Work",
  "window_id": "…",         "window_name": "Starter workspace",
  "workspace_id": "…",      "workspace_name": "Starter workspace"
}
```

MCP tool **`WhoAmI`** (no args) returns the same. Every other verb below accepts an optional explicit target but **defaults to the relevant id from self-context**, so the common case ("name my window") is a one-arg call.

### 4.3 Naming / identity verbs (the immediate ask)

All default their target to self-context; all are non-destructive.

| MCP tool | REST | Underlying call | Notes |
|---|---|---|---|
| `SetWindowName(name)` | `POST /api/v1/window/name` | `object.UpdateObjectMeta(window, {"window:displayname": name})` | drives taskbar title (≤64 chars, see title spec) |
| `SetTabName(name, tab_id?)` | `POST /api/v1/tab/name` | `object.UpdateTabName` | tab label |
| `SetPaneTitle(title, block_id?)` | `POST /api/v1/pane/title` | `object.UpdateObjectMeta(block, {"frame:title": title})` | own pane title; complements `OpenEditor(title:)` |
| `SetWorkspaceName(name)` | `POST /api/v1/workspace/name` | `workspace.UpdateWorkspace` | |

### 4.4 Launch-time naming (solves the `task dev` case)

Add startup env vars read by the **new instance** about **itself** (no cross-instance RPC):

- `AGENTMUX_WINDOW_NAME` → seeds `window:displayname` on the instance's window at init.
- `AGENTMUX_WORKSPACE_NAME` → seeds the starter workspace name (overrides the hardcoded `"Starter workspace"`, `wcore/mod.rs:85`) when a fresh DB is initialized.

Usage becomes:
```bash
AGENTMUX_WINDOW_NAME="repro #1503" task dev
```
Optionally add `task dev -- --name "X"` sugar that just exports the env for the child. Wiring point: backend window init seeds the meta before `installWindowTitleEffect` first runs (frontend title effect already reacts to it).

### 4.5 Layout / navigation verbs (high value, low cost)

Self-defaulting, reversible:

| MCP tool | REST | Underlying call |
|---|---|---|
| `FocusPane(block_id?)` / `FocusTab(tab_id)` / `FocusWindow(window_id?)` | `POST /api/v1/{pane,tab,window}/focus` | `client.FocusWindow`, `workspace.SetActiveTab` |
| `NewTab(name?)` | `POST /api/v1/tab/new` | `workspace.CreateTab` (+ `UpdateTabName`) |
| `SetActiveTab(tab_id)` | `POST /api/v1/tab/activate` | `workspace.SetActiveTab` |

`OpenEditor` (incl. `floating`, `collapse_tree`) and `Shell` already cover pane creation.

### 4.6 Introspection verbs (read-only, always safe)

| MCP tool | REST | Underlying call |
|---|---|---|
| `ListTabs()` / `ListWindows()` / `ListWorkspaces()` | `GET /api/v1/{tabs,windows,workspaces}` | `client.GetTab`, `window.GetWindow`, `workspace.ListWorkspaces` |
| `GetLayout()` | `GET /api/v1/layout` | composed read: windows → tabs → blocks tree |

(`DiscoverAgents` already covers the agent inventory.)

### 4.7 User-signal verb (candidate — confirm an event sink exists)

`Notify(message, level?)` → surface a transient toast/log line to the human (not another agent — that's `SendMessage`). Wire to the existing event bus (`wps/publish` already exists). Include only if a frontend toast consumer exists or is cheap to add; otherwise defer.

---

## 5. Safety / capability model

Self-scoped, reversible ops (naming, focus, new tab, introspection) are **allowed by default**. Destructive or global ops are **gated**:

- **Tier 0 (default, ungated):** all of §4.2, §4.3, §4.5 (create/focus/rename), §4.6 (reads).
- **Tier 1 (gated):** `CloseTab` of a tab the agent doesn't own, `CloseWindow`, `DeleteWorkspace`, moving *other* agents' panes. Gate behind an explicit capability flag (e.g. `agent:caps` block meta or a sidecar setting) and **default off**.
- **Never:** cross-instance control; killing processes the agent didn't spawn (already enforced by I3 / kill-by-PID rules).

The raw `/agentmux/service` gateway stays available (it's how the verbs are implemented) but is treated as **internal/advanced** — the curated verbs are the supported, documented surface. Consider gating the destructive `dispatch_service` methods behind the same Tier-1 capability so the gateway can't be used to bypass the verb-level policy.

Every verb's resulting `WaveObjUpdate` already broadcasts to the event bus (existing `handle_service` behavior), so the UI and other windows stay consistent with agent-driven changes.

---

## 6. Implementation plan

**Phase 1 — Foundation + the ask**
1. `GET /api/v1/self` + `WhoAmI` MCP tool (self-context resolver: block_id → tab → window → workspace).
2. `SetWindowName` / `SetTabName` / `SetPaneTitle` / `SetWorkspaceName` (REST verbs + MCP tools, self-defaulting).
3. `AGENTMUX_WINDOW_NAME` / `AGENTMUX_WORKSPACE_NAME` launch-time seeding.

**Phase 2 — Layout & introspection**
4. `FocusPane/Tab/Window`, `NewTab`, `SetActiveTab`.
5. `ListTabs/Windows/Workspaces`, `GetLayout`.

**Phase 3 — Safety & polish**
6. Capability tiers; gate Tier-1 ops; document the gateway as advanced.
7. `Notify` (if event sink confirmed).

Each phase ships independently; Phase 1 alone delivers the requested window naming as a first-class, discoverable ability.

---

## 7. Testing

- **Unit (sidecar):** each REST verb maps args → correct `dispatch_service` call and rejects bad targets; self-context resolves block_id → tab/window/workspace; Tier-1 gate denies when capability off.
- **MCP:** `tools/list` includes the new tools; each tool defaults target from self-context when omitted.
- **Launch-time:** `AGENTMUX_WINDOW_NAME=X task dev` → taskbar shows `X - … - AgentMux` (manual CEF smoke, like floating-pane).
- **Regression:** existing `OpenEditor` (incl. `floating`/`collapse_tree`) and `Shell`/`SendMessage` unaffected.

---

## 8. Open questions

1. **Capability storage** — block meta (`agent:caps`) vs. a sidecar setting vs. per-agent-definition field? Per-definition is most aligned with "agents are first-class entities."
2. **`window:displayname` for a multi-window instance** — `SetWindowName` defaults to the agent's own window; confirm self-context resolves the right window when the agent pane was torn off.
3. **Does a toast/notification consumer exist** in the frontend today, or is `Notify` net-new UI (then defer to its own spec)?
4. **Naming collisions / validation** — clamp lengths (window ≤64), strip control chars; reject empty (mirror the `SendMessage`/`OpenEditor` guards).

---

## 9. Appendix — floating-pane editor API status (checked 2026-06-17)

Requested alongside this spec. **Verdict: working as of `main`.**

- Shipped in **#1497** (`73ce2ec3`, 2026-06-16) — "feat(pane.open): OpenEditor collapse_tree + floating-pane support".
- Full path verified end-to-end: `OpenEditor{floating:true}` → `POST /api/v1/pane/open` → `app_api.rs::open_pane()` floating branch (~L911) → `open_pane_floating()` (~L1011-1186) reuses the `tear_off_block` saga → publishes scoped `openfloatingpane` WPS event → `frontend/app/store/global.ts:307-344` → host `open_floating_pane_window` (`agentmux-cef/src/commands/floating_pane.rs:81-231`) → chromeless `FloatingPaneWorkspace`.
- **Known limitations (by design, not bugs):** Windows = owned floating panes only (Phase 1, no cross-window redock); macOS/Linux = Phase A frameless windows. If the source window isn't in `state.windows`, the floater silently no-ops (logged) — safe.
- **Gap:** no automated test — the cross-process CEF path requires a manual smoke (the spec for #1497 calls this out). Recommend adding a smoke step to the verification checklist; full automation needs a CEF build harness.
