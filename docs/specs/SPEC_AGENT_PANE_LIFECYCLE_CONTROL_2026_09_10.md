# SPEC: Agent Pane Lifecycle Control — Close / Maximize / Minimize / Split / Float, and Unifying RPC + App API + MCP Behind One Registration

**Status:** proposed — design only, nothing implemented yet
**Date:** 2026-09-10
**Author:** Agent5
**Related:** `agentmux-mcp/src/main.rs`, `agentmux-mcp/src/tool_schemas.rs`,
`agentmux-srv/src/backend/rpc/engine.rs`, `agentmux-srv/src/backend/rpc/schema.rs`,
`agentmux-srv/src/server/mod.rs`, `agentmux-srv/src/server/app_api/fleet.rs`,
`agentmux-srv/src/sagas/delete_block.rs`, `frontend/layout/lib/layoutMagnify.ts`,
`frontend/layout/lib/layoutMinimize.ts`, `agentmux-cef/src/commands/floating_pane.rs`
**Predecessors (extend, don't duplicate):**
[`SPEC_AGENT_APP_API_MCP_BINDINGS_2026_06_28.md`](./SPEC_AGENT_APP_API_MCP_BINDINGS_2026_06_28.md)
(established MCP → REST `/api/v1/agent/*` → server-side-identity-stamped handler
as the canonical agent entry point),
[`SPEC_RPC_BINDINGS_CODEGEN_2026_09_07.md`](./SPEC_RPC_BINDINGS_CODEGEN_2026_09_07.md)
(the `register_typed`/`RpcSchema` registry this spec's §4 extends),
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

## 4. Design — one registration, three consumers

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

### 4.2 Extend `RpcSchema` to be the MCP source of truth too

Every pane-lifecycle command in this spec registers **once**, via
`register_typed` (the mechanism `SPEC_RPC_BINDINGS_CODEGEN` is already adding),
with one addition: an `mcp_tool` annotation carrying the pieces
`tool_schemas.rs` currently hand-writes — display name, parameter descriptions,
and which fields are agent-supplied vs. server-stamped (the same
`block_id`-from-`AGENTMUX_BLOCKID` pattern `SPEC_AGENT_APP_API_MCP_BINDINGS`
established).

```rust
engine.register_typed::<ClosePaneReq, ClosePaneResp, _, _>(
    "pane.close",
    RpcMeta {
        mcp_tool: Some(McpToolMeta {
            name: "ClosePane",
            description: "Close a pane (yours by default, or any pane by block_id).",
            stamped_fields: &["block_id"], // present in Req, but ONLY stamped
                                            // server-side when the agent omits it
        }),
        ..Default::default()
    },
    close_pane_impl,
)
```

`--dump-rpc-schema` (already proposed by `SPEC_RPC_BINDINGS_CODEGEN` §3.2) gains
a second output alongside `rpc.gen.d.ts`: `agentmux-mcp/src/tool_schemas.gen.rs`,
a generated `const` array of MCP tool JSON schemas plus a generated dispatch
table (command name → REST path + required/optional fields), replacing the
hand-written entries in `tool_schemas.rs` for every migrated command.
`agentmux-mcp/src/main.rs`'s dispatch loop calls the generated table instead
of a hand-written `match` arm per tool. `scripts/check-rpc-bindings.sh`
(already proposed) is extended to diff this file too — one gate covers all
three consumers going forward, not just frontend↔srv.

This is the concrete mechanism for "combine API + MCP + RPC into one coherent
system": **one `register_typed` call is the only place a new command's shape
is ever written down.** The WS path (frontend), the REST path
(`/api/v1/agent/pane/*`, MCP's transport), and the MCP tool schema all derive
from it. This does not touch the wire formats themselves (`RpcClient.rpcCall`,
the REST `X-AuthKey` middleware) — same non-goal `SPEC_RPC_BINDINGS_CODEGEN`
§4 already states, extended to cover the MCP transport too.

### 4.3 REST routes (MCP's transport, per the established pattern)

Following `SPEC_AGENT_APP_API_MCP_BINDINGS_2026_06_28.md` §5 exactly — the
REST handler resolves identity server-side from `block_id`/`AGENTMUX_BLOCKID`,
never from agent-supplied JSON, for the *caller's own identity*. The
*target* pane's `block_id`, when acting on another agent's pane, is supplied
explicitly by the agent (see §5 for why this is safe and matches
`SetName`'s `target_id` precedent) and resolved through `fleet.rs`'s existing
target-validation path, not trusted blindly:

| Command | REST route | MCP tool | Own-pane semantics | Cross-pane semantics |
|---|---|---|---|---|
| `pane.close` | `POST /api/v1/agent/pane/close` | `ClosePane` | close caller's own pane | close pane at `block_id` (fleet tier) |
| `pane.maximize` | `POST /api/v1/agent/pane/maximize` | `MaximizePane` | maximize caller's own pane | maximize pane at `block_id` (fleet tier) |
| `pane.minimize` | `POST /api/v1/agent/pane/minimize` | `MinimizePane` | minimize caller's own pane | minimize pane at `block_id` (fleet tier) |
| `pane.restore` | `POST /api/v1/agent/pane/restore` | `RestorePane` | undo maximize/minimize on caller's own pane | restore pane at `block_id` (fleet tier) |
| `pane.split` | `POST /api/v1/agent/pane/split` | `SplitPane` | split caller's own pane; `direction`, optional `view` for the new half | **not exposed cross-agent in v1** — see §7 |
| `pane.float` | `POST /api/v1/agent/pane/float` | `FloatPane` | float caller's own pane to a new OS window | **not exposed cross-agent in v1** — see §7 |
| `pane.dock` | `POST /api/v1/agent/pane/dock` | `DockPane` | dock a floated pane back into its tab | own-pane only |

`pane.maximize`/`pane.minimize`/`pane.restore` require the srv-side concept
gap from §1 to be closed first — see §6.

## 5. Authorization model — the actual hard part

Per §1.1, two tiers already exist. This spec assigns each verb explicitly
rather than inventing new rules per-command:

- **Close/maximize/minimize/restore, targeting another agent's pane: fleet
  tier.** Same posture as `FleetBulkStop` — gated by the instance `X-AuthKey`
  only, no per-agent ownership check, because (matching the existing route
  comment's own reasoning) that *is* the feature: an agent fixing another
  agent's stuck pane is exactly this spec's motivating case, and it cannot
  require the broken pane's own agent to cooperate (it might not be able to —
  that's the whole problem). Every cross-pane call gets per-target
  success/failure reporting and mandatory audit logging identical to
  `fleet.rs`'s existing `log_fleet_action_audit`, so "who closed my pane and
  why" is always answerable after the fact.
- **Split/float, own pane only in v1.** These create new panes/windows rather
  than acting on an existing one; there's no motivating case from this
  investigation for doing this to another agent's layout, and doing so
  correctly (which tab? which agent's workspace?) is a bigger design question
  deferred to §7 rather than rushed into v1 alongside the close/maximize/
  minimize work this spec is actually motivated by.
- **Own-pane operations: self-only, no additional guard** — same posture as
  every other self-scoped MCP tool (`MemoryWrite`, `UIClick`).

### 5.1 Do not repeat `SetName`'s unguarded `target_id` precedent uncritically

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

### 5.2 Open question for the repo owner

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

**(b) Ephemeral server-mediated relay, no persistence.** srv accepts
`pane.maximize`, resolves which live WS connection owns the target tab
(same resolution `FleetBroadcast` already does to find a target agent's
delivery channel), and pushes a `{eventtype: "layout-command", ...}` event
down that connection for the frontend to apply locally — same event-push
shape used elsewhere on the WS channel (`SPEC_AGENT_APP_API_MCP_BINDINGS` §3
already documents the `{eventtype: "rpc", data: ...}` wrapper as precedent
for push-style messages on this transport). Cheaper, ships faster, but the
state doesn't survive the owning window closing and reopening.

**Recommendation: (b) for v1**, explicitly flagged as a known limitation
(maximize/minimize via MCP is best-effort while the target window is open;
it is not durable), revisit (a) only if that limitation actually bites in
practice. `pane.close`, `pane.split`, and `pane.float` all have real
persisted or process-level server state already (§1) and don't face this
question.

## 7. Phased implementation plan

1. **`pane.close`, own pane.** Lowest risk — reuses the existing
   `sagas::delete_block::run` path, just gives it a typed `register_typed`
   entry and an MCP door. Confirms/fixes the history-preservation question
   from §1.2 as a blocking prerequisite, not a follow-up.
2. **`pane.close`, cross-agent (fleet tier).** The capability this
   investigation actually needed. Ships with per-target reporting + audit
   logging from day one, per §5.
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
6. **MCP schema codegen (§4.2).** Can land in parallel with steps 1-2 once
   the first `register_typed` + `mcp_tool` annotation exists to generate
   from; migrating the *existing* hand-written MCP tools
   (`OpenAgent`/`FleetBulkStop`/etc.) to generated schemas is a separate,
   optional follow-up cleanup, not required for this spec's own tools to
   ship generated from day one.

## 8. Acceptance criteria

- An agent can call `ClosePane` with no arguments and its own pane closes;
  the underlying conversation history is independently confirmed to survive
  (§1.2).
- An agent can call `ClosePane(block_id: "<another agent's pane>")` and that
  pane closes, with a `FleetActionResult`-shaped response and an audit log
  entry, matching `FleetBulkStop`'s existing response/audit shape.
- `SplitPane`/`FloatPane`/`DockPane` round-trip on the caller's own pane,
  verified against a real running instance (not just unit-tested), the same
  way the existing floating-pane feature is manually verified today.
- `MaximizePane`/`MinimizePane`/`RestorePane` work on the caller's own pane
  while the window stays open; the best-effort/non-durable limitation from
  §6(b) is documented in the tool description itself, not just this spec.
- `scripts/check-rpc-bindings.sh` (extended per §4.2) fails CI if
  `tool_schemas.gen.rs` drifts from the `register_typed` registry, the same
  guarantee `SPEC_RPC_BINDINGS_CODEGEN` already established for the frontend
  TS stubs.

## 9. Open questions summary (for repo-owner sign-off before implementation)

1. §1.2 — does `delete_block` already preserve conversation history
   independent of the block? Must be confirmed (and fixed if not) before
   step 1.
2. §5.2 — should a cross-pane close/maximize/minimize surface (but not
   block on) the target's `turn_active` state?
3. §6 — accept option (b) (ephemeral relay) for v1, or is durable
   `db_layout` persistence for maximize/minimize worth the larger change up
   front?
4. §7 step 6 — is migrating the *existing* hand-written MCP tools to
   generated schemas wanted as a near-term follow-up, or left indefinitely
   as hand-written legacy alongside the newly-generated ones?
