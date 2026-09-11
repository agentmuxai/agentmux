# SPEC: Agent Pane Lifecycle Control — Close / Maximize / Minimize / Split / Float

**Status:** proposed — design only, nothing implemented yet
**Date:** 2026-09-10
**Author:** Agent5
**Related:** `agentmux-mcp/src/main.rs`, `agentmux-mcp/src/tool_schemas.rs`,
`agentmux-srv/src/backend/rpc/engine.rs`, `agentmux-srv/src/backend/rpc/schema.rs`,
`agentmux-srv/src/server/mod.rs`, `agentmux-srv/src/server/app_api/fleet.rs`,
`agentmux-srv/src/server/ui_handlers.rs` (the `verified_block_id` signed-identity
mechanism §5.0 adopts), `agentmux-srv/src/sagas/delete_block.rs`,
`agentmux-srv/src/sagas/tear_off_block.rs`, `agentmux-srv/src/sagas/redock_floating_pane.rs`,
`frontend/layout/lib/layoutMagnify.ts`, `frontend/layout/lib/layoutMinimize.ts`,
`agentmux-cef/src/commands/floating_pane.rs`
**Tracked prerequisite:** [agentmuxai/agentmux#3192](https://github.com/agentmuxai/agentmux/issues/3192)
(§1.2/§9 — confirm `delete_block` preserves conversation history)
**Predecessors (extend, don't duplicate):**
[`SPEC_AGENT_APP_API_MCP_BINDINGS_2026_06_28.md`](./SPEC_AGENT_APP_API_MCP_BINDINGS_2026_06_28.md)
(established MCP → REST `/api/v1/agent/*` → server-side-identity-stamped handler
as the canonical agent entry point),
[`SPEC_RPC_BINDINGS_CODEGEN_2026_09_07.md`](./SPEC_RPC_BINDINGS_CODEGEN_2026_09_07.md)
(the `register_typed`/`RpcSchema` registry — §4.2 checked whether this spec
could extend it to MCP and found it can't, see that section),
[`SPEC_MULTI_AGENT_FLEET_CONTROL_2026_08_20.md`](./SPEC_MULTI_AGENT_FLEET_CONTROL_2026_08_20.md)
(the "fleet" authorization tier this spec's §5 borrows and contrasts with),
[`SPEC_MCP_SETNAME_TARGET_ID_2026_06_19.md`](./SPEC_MCP_SETNAME_TARGET_ID_2026_06_19.md)
(the `target_id` cross-element addressing pattern, and its unguarded precedent —
see §5.1),
[`SPEC_AGENT_UI_AUTOMATION_CLICK_SCREENSHOT_2026_08_18.md`](./SPEC_AGENT_UI_AUTOMATION_CLICK_SCREENSHOT_2026_08_18.md)
(the "own pane only" tier this spec's §5 contrasts with fleet).

---

## 0. Origin

While investigating a live bug (a composer `PageUp`/`PageDown` handler letting
the browser's default scroll escape `.agent-view`, reported by the repo owner
as "severely offset, can't even right-click, only the input box shows"), we
went looking for an MCP tool to close or reset the affected pane
(`Loap #2`, block `71e01b40-824d-4f89-849e-c5e22082969b`, confirmed via
direct `objects.db` query to have a perfectly healthy `db_layout` entry — this
is a pure client-side render bug, not data corruption) so an agent could fix
it directly instead of asking a human to click around a pane that doesn't
accept clicks.

**There is no such tool.** `UIClick`/`UIQuery` are explicitly scoped to the
calling agent's own pane and shared app chrome; nothing in the `mcp__agentmux__*`
family can close, maximize, minimize, split, or float *any* pane, own or
otherwise. The repo owner's direction, given directly in this conversation:
pane lifecycle control is "a main feature of agentmux" and should be built
out fully — own pane and other agents' panes, close/maximize/minimize/split/
float — and the design should **combine the entire API + MCP + RPC into a
single coherent system** rather than adding a fourth hand-maintained surface
next to the three that already exist and already drift (see §4.1).

## 1. What exists today

Confirmed by direct code research (not guessed):

| Action | Today's mechanism | Server-visible? |
|---|---|---|
| **Close pane** | Header × → `pane-actions.ts` → `LayoutModel.closeNode` (`layoutMagnify.ts:41-84`) mutates the client tree, then `onNodeDelete` (`tabcontent.tsx:55-71`) calls `services.ObjectService.DeleteBlock(blockId)` → WS `"object"` command → `object.rs:158-179` `"DeleteBlock"` case → `sagas::delete_block::run` (kills the block's PTY/controller, dispatches `Command::DeleteBlock` through the reducer, prunes the layout node). | **Yes** — real server-side mutation, but reached only through the generic untyped `"object"` WS command, not a typed RPC. |
| **Maximize/restore** | `magnifyNodeToggle` (`layoutMagnify.ts:24-34`) — pure client-side `LayoutNode` display flag. | **No.** No server RPC exists at all. |
| **Minimize/restore** | `minimizeNodeToggle`/`rebuildMinimizedSet` (`layoutMinimize.ts`). | **No.** Same as maximize. |
| **Split** | Drag-to-insert path (`LayoutTreeInsertNodeAction` in `layoutModel.ts`/`layoutTree.ts`) — no single named action. | Yes, as an ordinary layout-tree mutation (persisted), but there is no discrete "split this pane" command to call. |
| **Float to a new window** | Drag tear-off → `agentmux-cef` IPC `open_floating_pane_window` (`commands/floating_pane.rs`, backed by `floating_pane.rs`'s `WS_POPUP` HWND + cascade hook), with a pre-warmed pool (`window_pool.rs`/`pane_pool.rs`) for fast promotion. | Yes, but via the srv↔cef named-pipe IPC (`srv_ipc/server.rs`), a third channel distinct from both the WS RPC engine and the MCP REST routes. |

None of these are reachable from an MCP tool today. `DeleteBlock` is the
closest to "already server-side and just needs a door" — maximize/minimize
don't have a server-side concept to expose yet, and float already has to
cross into `agentmux-cef` territory that MCP has never touched.

### 1.1 The two-tier authorization precedent already in the codebase

Two existing capabilities that act across pane/agent boundaries chose
opposite answers to "does the caller need to own the target":

- **`UIClick`/`UIQuery`/`UIScreenshot`** (own-pane tier): hard-scoped to the
  caller's own `block_id`, stamped server-side from `AGENTMUX_BLOCKID` — "own
  pane only," no exceptions (`server/mod.rs:574-580`).
- **`FleetBulkStop`/`FleetBroadcast`** (fleet tier): explicitly **no**
  per-agent ownership check beyond the instance-wide `X-AuthKey` — the route
  comment states plainly that "fleet actions target OTHER agents' panes by
  design, so 'own pane only' doesn't apply here" (`server/mod.rs:581-591`).
  What it has instead: per-target success/failure reporting (never a single
  bool), an optional `staged{batch_size, max_fail_percentage}` blast-radius
  cap, and mandatory audit logging (`fleet.rs`, `log_fleet_action_audit`).

This spec has to put every new pane-lifecycle verb into one of these two
tiers (or define why it needs a third) — see §5.

### 1.2 `FleetBulkStop` "stop" is not "close" — a distinction this spec must not blur

`FleetBulkStop` halts the target's running agent *process* (`stop_one_agent_block`,
`agent_io.rs`) — the pane stays in the layout, now idle/stopped, restartable
from its header. `DeleteBlock` (§1, "Close pane") removes the block from the
layout entirely and tears down its controller. These read as similar but are
not interchangeable, and a new `ClosePane` tool must not silently become a
second name for `FleetBulkStop`, nor vice versa. Before implementation: confirm
whether `sagas::delete_block::run` preserves the agent's conversation history
(`db_agent_history`/`db_agent_instances`) independent of the block it was
displayed in — closing a stuck pane must not be able to destroy the
underlying conversation, only the UI element showing it. If it turns out
`delete_block` does NOT already guarantee that, fixing it is a prerequisite
for this spec, not an implementation detail to discover later.

## 2. Goals

1. Agents can close, maximize, minimize, split, and float **their own** pane.
2. Agents can close, maximize, and minimize **another agent's** pane —
   the exact capability this investigation needed and didn't have.
3. Every new verb is reachable from MCP, from the frontend's own UI (already
   true for close/maximize/minimize/split; float already works), and from the
   REST App API — through **one registration**, not three hand-written copies.
4. The design closes the gap `SPEC_RPC_BINDINGS_CODEGEN_2026_09_07.md` left
   explicitly out of scope (it stops at frontend TS stubs + a diff gate) by
   extending the same registry one step further: to MCP tool schemas too.

## 3. Non-goals

- Exact-pixel resize / arbitrary geometry — out of scope; maximize/minimize/
  restore only (matches what a human already gets from the pane header).
- Multi-monitor placement logic for floated panes — reuses whatever
  `open_floating_pane_window` already does for a human-initiated float.
- Simultaneous fleet-wide layout actions ("maximize every pane in Fab") —
  that composition already exists via `FleetBroadcast`/`FleetBulkStop`'s
  target-list model; this spec adds the single-target primitives they'd call,
  not a new fleet verb.
- Cross-machine (LAN/WAN) pane control — `FleetList`/`DiscoverAgents` already
  distinguish host/cross-channel block_ids from LAN/WAN agent names; a LAN/WAN
  peer has no block_id this instance can address. Confined to host +
  cross-channel for v1, same ceiling `FleetBulkStop` already has today.

## 4. Design — what "combine API + MCP + RPC" turns out to actually mean

### 4.1 The problem this has to solve, stated precisely

Today, adding one new capability that needs to be callable from the frontend
UI, the WS RPC engine, and an MCP tool means hand-writing it **three times**:
a `register_handler`/`register_typed` call in srv, a hand-written TS stub in
`frontend/app/store/rpc-api/`, and a hand-written JSON schema + `match` arm in
`agentmux-mcp/src/main.rs`/`tool_schemas.rs`. `SPEC_RPC_BINDINGS_CODEGEN_2026_09_07.md`
is already fixing the first drift (srv ↔ frontend) via `register_typed` +
codegen + a diff gate. It says nothing about MCP, because MCP is a separate
process with no crate dependency on srv (confirmed: `agentmux-mcp` talks to
srv purely over HTTP with its own hand-written `reqwest` calls).

### 4.2 §4.2/4.3 v1 were wrong — there is no process-wide registry to dispatch through

**Codex P1 on PR #3195, correctly, and this needed real verification, not a
patch:** the original design here assumed a `register_typed`/`RpcSchema`
registry an HTTP handler could look commands up in. That registry does not
exist in a form either transport could share. Checked directly against the
code:

- `agentmux-srv/src/server/websocket.rs:125` — `WshRpcEngine::new()` is
  constructed **fresh inside every WebSocket connection handler**, not once
  process-wide. Each connection's engine, and everything registered on it, is
  private to that connection.
- The HTTP router's `AppState` holds no engine at all — there is nothing for
  a `POST /api/v1/agent/rpc/:command` handler to look `:command` up in, typed
  or untyped, even if `register_typed` recorded a schema.
- Some WS registrations close over that connection's own `conn_id`, so even a
  hypothetical process-wide engine couldn't safely serve an HTTP request with
  no connection of its own.

So a shared dispatch table is not "a small addition to what already exists
internally" — it would require inventing a genuinely new process-wide command
registry, decoupled from per-connection state, that neither transport has
today. That is a real, separate infrastructure project, not part of this
spec's scope.

### 4.3 What actually ships instead: one Rust function, two hand-written call sites

Drop the shared-registry idea entirely. Use the pattern that is **already
shipped and proven** in this exact codebase — `FleetBulkStop` and `ClosePane`
(§7 step 1-2, landed in PR #3196) both work this way today:

1. One `*_impl` function holds the real logic (e.g. `close_pane_impl` /
   `handle_close_pane`), taking `&AppState` plus plain arguments — no
   transport-specific types.
2. A hand-written Axum route in `server/mod.rs` (`POST /api/v1/agent/pane/close`)
   deserializes the REST request and calls it — this is MCP's transport.
3. If the command also needs a WS/frontend caller, a hand-written
   `engine.register_handler` closure deserializes the WS request and calls
   the *same* function.

This is "one implementation, N thin transport adapters," not "one
registration, zero adapters" — the earlier draft overclaimed the latter.
`SPEC_RPC_BINDINGS_CODEGEN_2026_09_07.md`'s `register_typed` work remains
worth doing on its own merits (it stops frontend TS stubs drifting from
srv's Rust types), but it doesn't extend to MCP: the REST route for a new
pane-lifecycle command is a genuinely separate, small, hand-written addition
each time — exactly as `ClosePane`'s route was. That's the accurate scope,
and it's still cheap: one route line, one match arm in
`agentmux-mcp/src/main.rs`, one schema constant in `tool_schemas.rs` — the
same three-line pattern `ClosePane` already added.

| Command | MCP tool | Own-pane semantics | Cross-pane semantics |
|---|---|---|---|
| `pane.close` | `ClosePane` | close caller's own pane (§5 identity) | close pane at `block_id` (fleet tier) |
| `pane.maximize` | `MaximizePane` | maximize caller's own pane | maximize pane at `block_id` (fleet tier) |
| `pane.minimize` | `MinimizePane` | minimize caller's own pane | minimize pane at `block_id` (fleet tier) |
| `pane.restore` | `RestorePane` | undo maximize/minimize on caller's own pane | restore pane at `block_id` (fleet tier) |
| `pane.split` | `SplitPane` | split caller's own pane; `direction`, optional `view` for the new half | **not exposed cross-agent in v1** — see §7 |
| `pane.float` | `FloatPane` | float caller's own pane to a new OS window; **records its origin** (see `pane.dock` below) | **not exposed cross-agent in v1** — see §7 |
| `pane.dock` | `DockPane` | dock a floated pane back to its recorded origin (see below) — own-pane only | not applicable |

`pane.maximize`/`pane.minimize`/`pane.restore` require the srv-side concept
gap from §1 to be closed first — see §6.

**Codex P1 on PR #3191, correctly:** a no-argument "dock back into its tab"
has no destination to resolve against. `sagas::redock_floating_pane::run`
requires an explicit `source_tab_id`, `source_workspace_id`, `target_tab_id`,
`target_workspace_id`, and optional `dst_index` — `tear_off_block::run` does
not persist where a floated block came from, so today there is nothing a
no-arg `DockPane` could read.

**Round 2 (Codex P1 on PR #3195), on the first fix:** recording the origin
alone still isn't a usable destination. The *drag-initiated* tear-off path
(`server/service/tear_off.rs`'s `handle_tear_off_block`, called with
`auto_close: bool` defaulting to `true`) auto-deletes the source tab once
it's empty — so if `pane.float` used that same wrapper, the recorded
`source_tab_id` would frequently already be gone by the time `pane.dock`
runs, and `redock_floating_pane::run` rejects a missing target tab outright.

Fixed properly, verified against the actual code this time: `pane.float`
must **not** go through `handle_tear_off_block` at all. It reuses
`sagas::tear_off_block::run` directly — the same lower-level call
`app_api::pane::open_pane_floating` already makes for the existing
floating-editor feature (`pane.rs`, `SPEC_OPENEDITOR_FLOATING_AND_COLLAPSED_TREE_2026_06_16.md`)
— which never touches the source tab's auto-close behavior; that logic lives
entirely in the drag-tear-off's own wrapper, a sibling `pane.float` doesn't
call. `pane.float` records `source_tab_id`/`source_workspace_id` (block meta,
alongside the other `agent:*`/`cmd:*` keys already carried on a block, §0's
DB inspection found these directly) knowing the source tab survives. As a
second line of defense against any *other* path that might still prune an
empty tab later, `pane.dock` checks whether `source_tab_id` still exists
before calling `redock_floating_pane::run`; if it doesn't, it creates a
fresh tab in the recorded `source_workspace_id` (if that workspace still
exists) as the redock target, rather than failing outright. Optional explicit
`target_tab_id`/`target_workspace_id`/`dst_index` params remain available for
redocking somewhere else entirely.

## 5. Authorization model — the actual hard part

Per §1.1, two tiers already exist. This spec assigns each verb explicitly
rather than inventing new rules per-command — but §5.0 below corrects a real
hole Codex found in how "own-pane" was originally defined here.

### 5.0 Own-pane identity must be resolved server-side from a *signature*, not an optional field (Codex P1 on PR #3191)

The first draft of this spec described own-pane operations as needing "no
additional guard," citing `UIClick` as precedent. That understated what
`UIClick` actually does. Per `agentmux-srv/src/server/ui_handlers.rs`'s own
module doc comment (written after a real vulnerability, PR #2662): **a bare
client-supplied `block_id` is never trusted for anything, including the
caller's own identity** — `/api/v1/ui/*` shares the same instance-wide
`X-AuthKey` every agent can read from its own environment, so "the MCP schema
never exposes a block_id param" is not a real boundary; a bypassing agent can
call the REST endpoint directly with any block_id it wants. `verified_block_id`
closes this the only way that actually works: the request carries
`UiAutomationAuth` — an HMAC-SHA256 signature over the caller's own
`agent_id`, produced with that agent's own `AGENTMUX_JEKT_KEY` (the same
per-agent key `agentmux-mcp` already holds out-of-band for jekt sender
authentication) — srv verifies the signature against that agent's key on
file, and *only then* looks up that agent's actual current `block_id` itself,
server-side, via the `ReactiveHandler` registry. There is no field in the
request that names the caller's own pane at all.

This spec's own-pane operations (§4.3: own-only `pane.split`/`pane.float`/
`pane.dock`, and the own-pane path of `pane.close`/`pane.maximize`/
`pane.minimize`/`pane.restore`) adopt the identical mechanism: `agentmux-mcp`
signs the caller's `agent_id` with its `AGENTMUX_JEKT_KEY` on every pane-
lifecycle call; the REST/generic-dispatcher handler (§4.3) verifies it via
the same `verified_block_id`-style lookup before resolving "my own pane."
**Own-pane operations never accept a `block_id` request field at all** — not
optional, not defaulted, absent from the schema entirely — so there is
nothing for a bypassing direct-REST-call agent to substitute. A `block_id`
parameter exists ONLY on the commands this spec explicitly designates
fleet-tier (close/maximize/minimize/restore's cross-pane form, §5.1 below),
and its presence is exactly what routes a call to fleet-tier handling instead
of own-pane handling — the two are different request shapes, not the same
shape with an optional field, precisely so a foreign `block_id` can never
leak into what was meant to be a self-only call.

### 5.1 Cross-pane operations: fleet tier, with real caller attribution

- **Close/maximize/minimize/restore, targeting another agent's pane: fleet
  tier.** Same posture as `FleetBulkStop` — no per-agent *ownership* check on
  the target, because (matching the existing route comment's own reasoning)
  that *is* the feature: an agent fixing another agent's stuck pane is
  exactly this spec's motivating case, and it cannot require the broken
  pane's own agent to cooperate (it might not be able to — that's the whole
  problem). Unlike `FleetBulkStop` today, though, the *caller's* identity is
  not optional — see §5.2, Codex's second P1 finding, which this spec's
  fleet-tier commands must not inherit.
- **Split/float, own pane only in v1.** These create new panes/windows rather
  than acting on an existing one; there's no motivating case from this
  investigation for doing this to another agent's layout, and doing so
  correctly (which tab? which agent's workspace?) is a bigger design question
  deferred to §7 rather than rushed into v1 alongside the close/maximize/
  minimize work this spec is actually motivated by.

### 5.2 Audit attribution must be a verified `source_agent`, not `None` (Codex P1 on PR #3191)

This spec originally claimed cross-pane calls get "mandatory audit logging
identical to `fleet.rs`'s existing `log_fleet_action_audit`, so 'who closed my
pane and why' is always answerable." Codex checked that claim against the
actual code and it's false as stated: `log_fleet_action_audit` *accepts* a
`source_agent: Option<&str>` parameter, but `fleet_bulk_stop_impl`
(`agentmux-srv/src/server/app_api/fleet.rs`) currently calls it with
`source_agent: None` on every invocation — because the caller's identity was
never verified in the first place, there's nothing trustworthy to pass. Under
the `X-AuthKey`-only posture, "who" is unanswerable by construction: the key
is shared by every agent on the instance.

Fixed by reusing §5.0's signing mechanism for fleet-tier calls too: the
caller signs its own `agent_id` the same way, srv verifies it the same way
`verified_block_id` does, and the now-*verified* agent_id is what
`log_fleet_action_audit` receives as `source_agent: Some(&verified_agent_id)`
— not the unverified claim, the checked one. Add an optional `reason: String`
request field (absent from every existing fleet command, also missing today)
threaded into the audit record alongside it, since "why" was never captured
either. This is a **prerequisite for this spec's fleet-tier commands**, not
optional polish — without it, §8's acceptance criterion ("an audit log entry")
would ship an entry that still can't say who. It's also a real, standing gap
in `FleetBulkStop` itself; fixing the shared signing/verification helper as
part of this work is the natural point to close that gap too, but
`FleetBulkStop`'s own call sites switching to it is this spec's choice to
make, not a requirement — see open question in §9.

### 5.3 Do not repeat `SetName`'s unguarded `target_id` precedent uncritically

`SPEC_MCP_SETNAME_TARGET_ID_2026_06_19.md` §2 relaxed `SetName`'s guard so
that supplying `target_id` bypasses the caller's own `block_id` requirement
*entirely* — there is no check that the caller has any relationship to the
target at all, for any of window/tab/pane/workspace rename. That is a low-
consequence action (a name is cosmetic and reversible) and was accepted on
that basis. **Close/maximize/minimize are not equally reversible** — closing
tears down a controller; maximizing another agent's pane while it's mid-turn
changes what the operating human sees without their pane-owning agent's
knowledge. Adopting fleet tier (§5) rather than `SetName`'s bare-`target_id`
tier is a deliberate choice to get the audit trail and per-target reporting
`SetName` never had, not an oversight.

### 5.5 Open question for the repo owner

Should a cross-pane close/maximize/minimize call require the *target* agent
to be idle (no turn in flight), the way `FleetBulkStop` has no such guard
today but arguably should for a "close," not just a "stop"? Recommendation:
no hard block — the Loap #2 case this spec is motivated by may well need
closing a pane that *looks* stuck precisely because its turn appears
in-flight but isn't actually progressing — but surface `turn_active`
(already returned by `GetAgentTranscript`/`ListConversations`) in the tool
description so the calling agent can make an informed choice, and always log
it in the audit entry.

## 6. Closing the maximize/minimize server-side gap

Maximize/minimize are pure client `LayoutNode` flags today (§1) — no server
RPC, so nothing for `register_typed` to wrap yet. Two options:

**(a) Make them genuine persisted layout state.** Add `magnified`/`minimized`
to the persisted `LayoutNode` shape in `db_layout`, same as position/size are
today. Bigger change, but consistent with "the frontend's layout tree and the
server's `db_layout` should agree," and survives a reload.

**(b) Ephemeral server-mediated relay, no persistence — REVISED.** The first
draft of this option leaned on two things that turned out not to exist:

- **Codex P1 on PR #3195, correctly:** `FleetBroadcast`'s server-side
  resolution (`fleet_broadcast_impl`, `app_api/fleet.rs`) resolves a
  `block_id` to an **agent**, then `inject_message`s that agent's own
  controller (its CLI process's stdin, effectively) — it never touches a
  frontend WebSocket connection. It is not "the same resolution" this needs
  at all.
- Every WS connection's own tab scoping is also a dead end:
  `websocket.rs:79` sets `let tab_id = String::new();` at connection open and
  never populates it before calling `event_bus.register_ws(&conn_id,
  &tab_id)` — so no tab is ever actually registered against any connection.
  There is no existing tab→connection index to look a target up in.

**What actually works, verified against real shipped code:**
`app_api::pane::open_pane_floating` already solves the adjacent problem —
telling *one specific window's* frontend to do something srv can't do
itself (open the CEF host's floating window). It resolves
`workspace_id → window_id` from `state.srv_state`'s own `s.windows` map,
then calls `state.broker.publish(WaveEvent { event: "openfloatingpane",
scopes: vec![window_id], data: Some(json!({...})), .. })` — a real,
shipped, window-scoped push, not a WS-connection-keyed one.

Maximize/minimize can reuse exactly this path: resolve `block_id → tab_id`
(`s.blocks`) `→ workspace_id` (`s.tabs`) `→ window_id` (`s.windows`, same
lookup `open_pane_floating` performs), then `state.broker.publish` a new
event kind (e.g. `"paneMaximize"`/`"paneMinimize"`, carrying `block_id`) to
that one window. The frontend adds one small listener next to its existing
`"openfloatingpane"` handler, calling the same local `magnifyNodeToggle`/
`minimizeNodeToggle` a human's own click already triggers. This is real,
traceable, existing-state-only plumbing — no new registry, no new
connection-scoping mechanism, unlike the original (b).

**Recommendation: (b), REVISED shape, for v1.** Still explicitly a known
limitation (best-effort while the target window is open; state doesn't
survive that window closing and reopening — same caveat as before, just now
resting on a mechanism that actually exists), and revisit (a) (durable
`db_layout` persistence) only if that limitation bites in practice.
`pane.close`, `pane.split`, and `pane.float` all have real persisted or
process-level server state already (§1) and don't face this question.

## 7. Phased implementation plan

1. **`pane.close`, own pane.** Lowest risk — reuses the existing
   `sagas::delete_block::run` path behind a hand-written REST route + MCP
   tool (§4.3's actual, proven pattern). Confirms/fixes the
   history-preservation question from §1.2 as a blocking prerequisite, not a
   follow-up. **Shipped in PR #3196.**
2. **`pane.close`, cross-agent (fleet tier).** The capability this
   investigation actually needed. Ships with per-target reporting + audit
   logging from day one, per §5. **Shipped alongside step 1 in PR #3196.**
3. **`pane.split`, `pane.float`, `pane.dock` — own pane only.** `split`
   reuses the existing `LayoutTreeInsertNodeAction` path; `float`/`dock` cross
   into the srv↔cef named-pipe IPC (§1) for the first time from MCP — new
   ground, gets its own smoke test against a real floated window before
   merging.
4. **`pane.maximize`/`pane.minimize`/`pane.restore`, own pane.** Blocked on
   §6's design decision landing first.
5. **`pane.maximize`/`pane.minimize`/`pane.restore`, cross-agent (fleet
   tier).** Last — highest-novelty combination (new server concept *and*
   fleet-tier cross-agent targeting at once).
Step 6 ("MCP schema codegen") from the original draft is **removed** — §4.2
found the shared-registry premise it depended on doesn't exist. There is no
codegen follow-up pending from this spec; each command's MCP tool schema
stays hand-written in `tool_schemas.rs`, same as every existing tool.

## 8. Acceptance criteria

- An agent can call `ClosePane` with no arguments and its own pane closes;
  the underlying conversation history is independently confirmed to survive
  (§1.2).
- An agent can call `ClosePane(block_id: "<another agent's pane>")` and that
  pane closes, with a `FleetActionResult`-shaped response and an audit log
  entry whose `source_agent` is the caller's *verified* identity (§5.2), not
  `None`.
- `SplitPane`/`FloatPane`/`DockPane` round-trip on the caller's own pane,
  verified against a real running instance (not just unit-tested), the same
  way the existing floating-pane feature is manually verified today.
- `MaximizePane`/`MinimizePane`/`RestorePane` work on the caller's own pane
  while the window stays open, via a `state.broker.publish`/window-scoped
  `WaveEvent` (§6(b) revised) — not the earlier tab→connection idea, which
  §6 found doesn't correspond to anything real. The best-effort/non-durable
  limitation is documented in the tool description itself, not just this
  spec.

## 9. Open questions summary (for repo-owner sign-off before implementation)

1. §1.2 — does `delete_block` already preserve conversation history
   independent of the block? Must be confirmed (and fixed if not) before
   step 1. **Tracked as [agentmuxai/agentmux#3192](https://github.com/agentmuxai/agentmux/issues/3192)**
   (reagent P1 on PR #3191: this was a blocking prerequisite with no owner or
   tracking reference in the first draft).
2. §5.5 — should a cross-pane close/maximize/minimize surface (but not
   block on) the target's `turn_active` state?
3. §6 — accept option (b) (ephemeral relay) for v1, or is durable
   `db_layout` persistence for maximize/minimize worth the larger change up
   front?
4. **Resolved, no longer open (§4.2):** a shared `register_typed`-backed
   dispatch table for REST+MCP turned out not to correspond to any real
   process-wide registry — verified directly against `websocket.rs`'s
   per-connection `WshRpcEngine` construction. Each new command gets a
   hand-written REST route + MCP tool, same pattern `ClosePane` already
   shipped with (PR #3196).
5. §5.2 — should fixing `FleetBulkStop`'s existing `source_agent: None` gap
   (using the same signing/verification helper this spec adds for its own
   fleet-tier commands) be folded into this work, or filed as its own
   separate follow-up? Either way it should not ship as a *known* new gap
   given this spec now has the fix sitting right next to it.
