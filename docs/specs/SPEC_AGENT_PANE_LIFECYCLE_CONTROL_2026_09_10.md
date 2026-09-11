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
3. Every new verb is reachable from MCP and (already true today) the
   frontend's own UI, through **one shared Rust implementation per command**
   plus a thin, hand-written transport adapter for each caller (REST for
   MCP, WS `register_handler` for the frontend where one is needed) — not
   three independently-diverging copies of the actual logic. §4.2 found
   that a single shared *dispatch registry* spanning both transports isn't
   buildable against the real `WshRpcEngine`/`AppState` architecture; §4.3
   is the corrected, smaller goal this spec actually delivers, and it's
   what `ClosePane` (PR #3196) already ships.
4. ~~The design closes the gap `SPEC_RPC_BINDINGS_CODEGEN_2026_09_07.md` left
   explicitly out of scope by extending the same registry one step further,
   to MCP tool schemas too.~~ **Dropped (§4.2).** That registry doesn't
   exist in a cross-transport form to extend. MCP tool schemas stay
   hand-written in `tool_schemas.rs`, same as every tool before this spec.

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
shipped and proven** in this exact codebase — `FleetBulkStop`, on `main`
today, works this way. `ClosePane` (§7 step 1-2) is built the same way in
[PR #3196](https://github.com/agentmuxai/agentmux/pull/3196), not yet merged
as of this revision — a second, independent instance of the same pattern,
not yet itself a "shipped" precedent to point to until it lands:

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

`pane.maximize`/`pane.minimize`/`pane.restore` need one small, precedented
reducer-command addition first (not a "gap" — see §6, revised after
verification: the persisted state already exists, only a targeted
dispatchable command to set it is missing).

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

**Round 3 (Codex P1 on PR #3195), on the second fix:** reusing the raw saga
call correctly avoids the auto-close problem, but `open_pane_floating` and
`pane.float` are not actually solving the same problem — `open_pane_floating`
starts from a block that was created fresh and **never inserted into the
source tab's layout tree at all** (§0's own comment on that function: "no
layout node," "the block never renders docked before the saga moves it").
`pane.float` starts from an *existing, already-laid-out* pane, so calling
just `tear_off_block::run` moves the block's ownership but leaves the OLD
layout node behind in the source tab (now a dangling reference) and does
nothing to materialize the new tab's layout or open the actual floating
window. `pane.float`'s implementation must do everything
`open_pane_floating` does *after* its own `tear_off_block::run` call too,
not just the saga call in isolation: `setup_torn_off_block_layout` for the
destination, `event_bus.broadcast_wave_obj_updates` for the new
workspace/tab/block, and the `state.broker.publish(WaveEvent{event:
"openfloatingpane", ...})` call to actually open the OS window —
`open_pane_floating`'s full body (`pane.rs` lines ~30-190) is the real
reference implementation to adapt, not just its saga call.

**Round 4 (Codex P1 on PR #3195), the specific miss in Round 3's list:**
naming a "`LayoutDeleteNodeByBlock`-style prune" undersold what's actually
required. `server::service::layout_helpers::queue_source_layout_delete`
(`pub(crate)`, already used by both `tab_move.rs` and the drag-tear-off's
`tear_off.rs`) is the specific function needed — it does the reducer-level
prune **and** queues a `pendingbackendactions` entry so an already-loaded
frontend converges instead of resurrecting the dangling source leaf on its
next unrelated edit (the exact bug
`INVESTIGATION_LAYOUT_DEAD_SPACE_STALE_TREE_RESURRECTION_2026_07_08.md`
documents, and the reason `sagas::delete_block::run`'s own Step 2b exists —
§1 already cited that saga as `pane.close`'s foundation without connecting
that its Step 2b is solving the identical problem `pane.float` now also
has). Call `queue_source_layout_delete` by name, not an approximation of
it.

**Same finding applies to `pane.dock` — DockPane needs the redock
wrapper's layout work too, not just the raw saga.** Exactly the same shape
of gap as `pane.float`'s: `sagas::redock_floating_pane::run` (§4.3's table)
only moves tab membership; it does not touch either tab's layout tree. The
real, shipped RPC handler, `server::service::tear_off::handle_redock_floating_pane`
(`pub(crate)`, sibling to `handle_tear_off_block` in the same file), is what
actually does `queue_target_layout_insert`/`queue_target_layout_split` on
the destination, `queue_source_layout_delete` on the (now-empty) floating
source, and broadcasts both resulting layouts — without which the block
moves but never visually appears anywhere. `pane.dock`'s implementation
should call `handle_redock_floating_pane` directly (filling in its
`source_tab_id`/`source_workspace_id`/`target_tab_id`/`target_workspace_id`
arguments from the recorded-origin defaults resolved above, or from
explicit override params) rather than calling the raw saga and
re-deriving its wrapper's layout work by hand — including when the
resolved destination is a freshly-created fallback tab, not just the
originally-recorded one.

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

## 6. Maximize/minimize: not a gap at all — REVISED after verification

§1 and this section's first two drafts both assumed maximize/minimize have
**no server-side concept to build on** — that was wrong, and building an
entire relay mechanism on that wrong premise is exactly the kind of
compounding error a spec review exists to catch before code does.

**Codex P2 on PR #3195, correctly:** maximize state is **already persisted**
server-side. `LayoutState.magnifiednodeid` (`agentmux-srv/src/backend/obj.rs:441`)
is a real, persisted column, round-tripped through `persist_subscriber.rs`
and mirrored on `TabRecord`. Minimize is real too, if less directly: a
minimized leaf is `node.minimized = true` — a **top-level** field on
`LayoutNode` itself (`frontend/layout/lib/layoutMinimize.ts:111`), not
nested under its `.data`. **Codex P1 on PR #3195, a follow-up correction to
this same paragraph:** `LayoutNode` and `LayoutNodeData` each carry their
*own*, separate `extra` catch-all (`agentmux-common/src/layout_types.rs:90`
vs `:55`) — writing `minimized` into `LayoutNodeData.extra` (the `.data`
sub-object's catch-all) would serialize it as `data.minimized`, which
nothing reads; it belongs in `LayoutNode.extra` (the node's own top-level
catch-all, line 90), the one that actually round-trips to what
`node.minimized` reads. Both are genuine `serde_json::Map` catch-alls that
round-trip to `db_layout` — so a human clicking minimize already persists
it correctly today, via the ordinary whole-tree layout push
(`model.persistToBackend()`, called by both `magnifyNodeToggle` and
`minimizeNodeToggle`); this paragraph's job is only to make sure a NEW
targeted command writes to the same place.

**What's genuinely missing, verified directly (this spec's own follow-up,
not yet reviewer-found):** a *targeted*, single-field way to set it.
`Command::SetMagnifiedNode { tab_id, node_id }` already exists as a reducer
command (`agentmux-srv/src/reducer.rs:83`, `reducer/layout.rs`) — but its
**only** call site (`server/service/object.rs:267`) fires it as an internal
side-effect of a different action, never as its own dispatchable command a
client can invoke directly. Minimize has no equivalent targeted command at
all; the only existing path is the coarse whole-tree push, which needs the
caller to already hold a full, current copy of the layout tree — awkward
for a stateless MCP call and a real way to lose a concurrent edit.

**Design: expose `SetMagnifiedNode` as its own dispatchable action, and add
its minimize counterpart the same way.** Both follow the exact
`("object", "DeleteBlock")` pattern `ClosePane` already reuses (§4.3) — a
small, additive, precedented change, not a new mechanism:

- `pane.maximize`/`pane.restore` → dispatch the *existing*
  `Command::SetMagnifiedNode { tab_id, node_id: block's node (or "" to
  restore) }` directly, exposed via a new `("object", "SetMagnifiedNode")`
  case in `object.rs` alongside `DeleteBlock`'s.
- `pane.minimize`/`pane.restore` → add `Command::SetMinimizedNode { tab_id,
  node_id, minimized: bool }`, mirroring `SetMagnifiedNode`'s shape, writing
  the same top-level `LayoutNode.extra.minimized` field the frontend's own
  toggle already writes (not `LayoutNodeData.extra` — see the correction
  above) — new reducer command, small and precedented, not a new *category*
  of state.
- Both are genuinely idempotent by construction (set a field to an explicit
  value), which also resolves the separate toggle-vs-idempotent finding
  from the previous draft — there is no toggle involved at all once this is
  a direct field set, so `MaximizePane` called twice trivially stays
  maximized.
- REST route + MCP tool for each, exactly `ClosePane`'s shape (§4.3): no
  new transport, no window-scoped push, no relay. Real, durable,
  `db_layout`-agreeing state — the win the discarded option (a) was reaching
  for, obtained for free once the actual existing mechanism was found.

The `state.broker.publish`/window-scoped-`WaveEvent` mechanism this section
previously proposed (verified real in the process of finding this — see
`open_pane_floating`'s `"openfloatingpane"` push) remains valid, useful
precedent for a *future* command that has no persisted-state answer
available — just not this one. The *architectural* open question §9 item 3
tracked (ephemeral vs. durable) is resolved — durable, via the existing
fields. Remaining refinements to the exact reducer-command semantics are
listed in §6.1, not re-opening this decision.

### 6.1 Implementation-time refinements this spec's own review surfaced

This PR (#3195) is a design spec for work that has not started — per the
repo owner's own direction, Phase 3+ (split/float/dock/maximize/minimize)
is explicitly on hold pending their review of §9, not proceeding to code
yet. Seven rounds of review on this document have each found real
architectural gaps (§4.2's false registry, §5's identity/audit holes, §6's
false "no persisted state" premise) worth fixing at the design level before
any of that is worth writing. The review has since moved into a further,
narrower layer — exact field paths, frontend reconciliation timing, edge
cases in existing toggle guards — that are genuinely correct concerns, but
belong to the *implementation* of each command, not this document's
architecture. Listed here rather than resolved individually, so the actual
implementer verifies each against the code *at build time* (which will
have moved on from today's snapshot regardless of how precisely this spec
tries to describe it now) instead of trusting this list as a final word:

- **Cross-channel forwarding (Codex P1):** a target block on another local
  channel is invisible to this instance's `AppState`. `FleetBulkStop`
  handles this via `forward_stop_to_shared_channel`; pane-lifecycle
  commands need the same, plus a real answer for how the remote instance
  verifies `verified_block_id`'s per-instance HMAC across the channel
  boundary — or v1 narrows explicitly to same-channel only, which needs
  to be stated as a real MCP-tool-description-level limitation, not left
  implicit.
- **Stacked panes (Codex P1, round 6+7) — affects shipped code, not just
  future work:** a layout leaf can host a `blockStack` of multiple
  blockIds (`frontend/layout/lib/layoutStack.ts`), and
  `getNodeByBlockId` matches ANY member — so anything that resolves a
  leaf this way and deletes it (`delete_block::run`'s
  `LayoutDeleteNodeByBlock` step, and therefore `pane.float`'s planned
  source-delete too) removes the WHOLE leaf even when the target is one
  member of a multi-block stack, orphaning its siblings (still live in
  `blockids`, no layout node, invisible). **This is not hypothetical for
  `pane.close`** — PR #3196 already ships `delete_block::run` this way.
  Tracked as [agentmuxai/agentmux#3202](https://github.com/agentmuxai/agentmux/issues/3202)
  rather than left as a spec-only note, since it affects code already
  under review, not just Phase 3+. Needs a server-side stack-aware close
  (detach only the targeted member when others remain; fall back to the
  existing whole-leaf delete only for the last/only member) for both
  `pane.close` and, later, `pane.float`.
- **Live reconciliation for an already-open target tab (Codex P1):**
  persisting `SetMagnifiedNode`/`SetMinimizedNode` and broadcasting the
  WaveObj update does not, by itself, update a frontend that already has
  the tab's `LayoutModel` loaded — `layoutPersistence.ts`'s
  `onBackendUpdate` doesn't copy an updated root/`magnifiednodeid` today,
  and `layout_helpers.rs` documents this non-auto-sync explicitly (the
  same class of gap `queue_source_layout_delete` exists to close for
  deletes, §6's own citation above). The command needs the equivalent
  pending-action/reconciliation path, or its target window won't visibly
  update until some unrelated edit happens to refresh it.
- **`SplitPane` direction (Codex P2):** `LayoutTreeInsertNodeAction` (§4.3's
  table) takes no target node or direction — it can't honor
  `direction: left/right/up/down` at all. Needs the real
  `SplitHorizontal`/`SplitVertical` actions, or `open_pane`'s own
  `resolve_placement` + queued split action, not a plain insert.
- **`SplitPane` must create the block before placing it (Codex P2, round
  7):** both the frontend split helpers and `app_api::open_pane` dispatch
  `CreateBlock` (deriving/validating its meta) *before* enqueueing
  placement — the table's `view` param alone has no creation/default/
  inheritance step behind it, so there's no block ID yet for the
  placement actions above to place. Reuse `open_pane`'s create-then-place
  flow rather than assuming a block already exists.
- **Minimize's last-expanded-leaf guard (Codex P2):** the existing human
  toggle refuses to minimize a tab's last expanded leaf
  (`countExpandedLeaves(...) <= 1`) — an unconditional `SetMinimizedNode`
  needs the identical guard, or an agent could leave a tab with zero
  visible content, a state the UI is deliberately designed to prevent.
- **`RestorePane` must be target-specific (Codex P2):** clearing
  `magnifiednodeid` unconditionally restores whichever pane happens to be
  maximized — if that's a different pane than the one `RestorePane`
  targeted, it wrongly un-maximizes an unrelated pane. Clear it only when
  its current value equals the *targeted* pane's own resolved node id.
- **`DockPane`'s `dst_index` is currently a no-op (Codex P2, round 7):**
  `handle_redock_floating_pane` (§4.3's citation for `pane.dock`) takes
  its own optional args as `target_block_id`/`direction` and always calls
  `redock_floating_pane::run` with `dst_index: None` — it has no path to
  honor an explicit ordering today. Either extend that wrapper to accept
  and forward a real `dst_index`, or `pane.dock` performs the wrapper's
  layout work itself around a direct saga call instead of calling the
  wrapper as-is. Don't advertise `dst_index` as functional until one of
  those is true.

## 7. Phased implementation plan

1. **`pane.close`, own pane.** Lowest risk — reuses the existing
   `sagas::delete_block::run` path behind a hand-written REST route + MCP
   tool (§4.3's actual, proven pattern). Confirms/fixes the
   history-preservation question from §1.2 as a blocking prerequisite, not a
   follow-up. **Implemented in [PR #3196](https://github.com/agentmuxai/agentmux/pull/3196)
   — not yet merged at the time of this revision; check that PR for current
   status rather than treating this spec as confirmation it's on `main`.**
2. **`pane.close`, cross-agent (fleet tier).** The capability this
   investigation actually needed. Ships with per-target reporting + audit
   logging from day one, per §5. **Implemented alongside step 1 in PR #3196
   (same merge-status caveat).**
3. **`pane.split`, `pane.float`, `pane.dock` — own pane only.** `split`
   reuses the existing `LayoutTreeInsertNodeAction` path; `float`/`dock` cross
   into the srv↔cef named-pipe IPC (§1) for the first time from MCP — new
   ground, gets its own smoke test against a real floated window before
   merging.
4. **`pane.maximize`/`pane.minimize`/`pane.restore`, own pane.** No longer
   blocked on an open design question — §6 found both already have real
   persisted server-side state, just no targeted set-command yet. Expose
   `Command::SetMagnifiedNode` as its own dispatchable `("object", ...)`
   action; add the small, precedented `Command::SetMinimizedNode`
   counterpart; wrap each in a REST route + MCP tool exactly like
   `ClosePane`.
5. **`pane.maximize`/`pane.minimize`/`pane.restore`, cross-agent (fleet
   tier).** Same fleet-tier pattern §5.1 already establishes for `pane.close`
   — no longer "new server concept and cross-agent at once," since step 4
   resolved the server-concept half.
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
- `MaximizePane`/`MinimizePane`/`RestorePane` set real, `db_layout`-persisted
  state (`LayoutState.magnifiednodeid` / the leaf's `extra.minimized`) via
  the new targeted `SetMagnifiedNode`/`SetMinimizedNode` actions (§6) —
  durable across a reload, not best-effort, since §6 found the earlier
  "no persisted state exists" premise was wrong.

## 9. Open questions summary (for repo-owner sign-off before implementation)

1. §1.2 — does `delete_block` already preserve conversation history
   independent of the block? Must be confirmed (and fixed if not) before
   step 1. **Tracked as [agentmuxai/agentmux#3192](https://github.com/agentmuxai/agentmux/issues/3192)**
   (reagent P1 on PR #3191: this was a blocking prerequisite with no owner or
   tracking reference in the first draft).
2. §5.5 — should a cross-pane close/maximize/minimize surface (but not
   block on) the target's `turn_active` state?
3. **Resolved at the architecture level (§6):** maximize/minimize turned
   out to already have real persisted server-side state
   (`LayoutState.magnifiednodeid`, `LayoutNode.extra.minimized`) — the
   "which design, ephemeral or durable" question was built on a false
   premise. Expose the existing `SetMagnifiedNode` reducer command as its
   own dispatchable action, add its `SetMinimizedNode` counterpart. Several
   real command-level refinements (last-expanded-leaf guard,
   target-specific restore, live reconciliation for an already-open tab)
   remain and are tracked in §6.1, not re-opening this architectural
   decision.
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
