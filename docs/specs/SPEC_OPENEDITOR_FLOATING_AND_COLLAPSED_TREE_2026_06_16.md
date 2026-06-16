# SPEC: OpenEditor — collapsed file-tree + floating-pane support

**Date:** 2026-06-16
**Status:** In progress
**Author:** Naki
**Repo HEAD at analysis:** `bb255fda`
**Related:** [[SPEC_FLOATING_PANE_TEAROFF_2026_05_11]], [[SPEC_MACOS_FLOATING_PANE_TEAROFF_2026_05_29]],
[[SPEC_FLOATING_PANE_REDOCK_2026-05-27]], `docs/retro/saga-coordinator-location-analysis-2026-04-30.md`

---

## 1. Goal

Extend the `OpenEditor` MCP tool (and the underlying `pane.open` RPC / `POST /api/v1/pane/open`)
so an agent can open a file:

1. **with the file-tree sidebar collapsed** (just the file, no explorer), and
2. **in a floating pane** (a chromeless window over the source instance), instead of a docked split.

Both must **reuse the existing systems** — the editor's `editor:tree_expanded` meta and the
existing floating-pane tear-off pipeline — not invent parallel machinery.

---

## 2. How the relevant systems work (verified)

### 2.1 Editor file-tree state is meta-driven
`EditorViewModel` restores tree state from `block.meta` on init
(`frontend/app/view/editor/editor-model.ts:39,266-268`):

```ts
const META_TREE_EXPANDED = "editor:tree_expanded"; // default true
if (meta?.[META_TREE_EXPANDED] === false) this._treeExpanded[1](false);
```

So opening collapsed = write `block.meta["editor:tree_expanded"] = false` at create time. **No
frontend change needed.**

### 2.2 The reducer is a pure state machine; it does not open OS windows
`reducer.rs::update(&mut State, Command) -> Vec<Event>` (no I/O). `CreateWindow{window_id,
workspace_id}` only registers a window↔workspace mapping and emits `SrvWindowOpened`
(`reducer/window.rs:16-48`). Events fan out to SQLite (`persist_subscriber`) + the event bus
(`publish_events`).

### 2.3 Floating panes are frontend-orchestrated tear-offs
A floating pane is a **chromeless `WS_POPUP` window owned by the source main window**, sharing the
same sidecar / data dir / reducer state (`agentmux-cef/src/commands/floating_pane.rs:4-24`). The
tear-off flow (drag path, `service.rs:1873` + `CrossWindowDragMonitor.*`):

1. Frontend → srv RPC `workspace/TearOffBlock` → `sagas::tear_off_block::run`
   (`CreateWorkspace` → `CreateTab` → `MoveBlock`) → `setup_torn_off_block_layout` → returns
   `new_workspace_id`.
2. Frontend → **host** command `open_floating_pane_window{pane_id, workspace_id, x, y, width,
   height}` (`floating_pane.rs:81`) — opens the OS window; its docstring already anticipates
   *"the frontend **or an agent**"* invoking it.
3. The floater boots with `?floatingPaneId=<id>&workspaceId=<ws>`, renders
   `<FloatingPaneWorkspace>` (`app.tsx:377`), and calls `WindowService.CreateWindow(null, wsId)`.

**srv cannot open an OS window** — only the host can, and only the frontend invokes host commands.
So a programmatic float needs a srv→frontend directive.

### 2.4 srv→frontend directive channel (scoped pub/sub)
`frontend/app/store/global.ts:259` `initGlobalEventSubs` subscribes via `waveEventSubscribe(
{eventType, handler, scope?})` (`store/wps.ts`). `scope` becomes a `SubscriptionRequest.scopes`
entry; srv broadcasts `WSEventType{eventtype, oref, data}` (`backend/eventbus.rs:43,158`) and the
bus routes by `eventtype` + `oref`-vs-`scope`. The existing `userinput` sub is **scoped to
`windowId`** — the exact pattern we reuse to target one window.

---

## 3. Design

### 3.1 Collapsed tree (DONE)
- `OpenEditor` tool: new `collapse_tree: boolean` → `pane.open` body `tree_expanded: false`.
- `CommandPaneOpenData.tree_expanded: Option<bool>` (`backend/rpc_types.rs`).
- `build_pane_meta` (editor arm): writes `editor:tree_expanded` when present (`app_api.rs`).
- Frontend already honors it. Compiles (srv + mcp `cargo check` green).

### 3.2 Floating pane (server-owns, flashless)

`OpenEditor(floating: true)` → `pane.open` body `floating: true`.

`CommandPaneOpenData` gains `floating: Option<bool>`. In `open_pane`, when `floating == Some(true)`,
it delegates to `open_pane_floating`:

1. Resolve the source workspace from the reducer's `state.tabs[tab].workspace_id`.
2. **Create the block via the reducer** `Command::CreateBlock{tab_id, meta}` (NOT `wcore::create_block`)
   — the `tear_off_block` saga's pre-condition checks the reducer-canonical `state.blocks` map, and
   only the reducer command populates it. The emitted `BlockCreated` event also carries the meta,
   which `persist_subscriber::apply_block_created` writes into the wstore Block (so the editor renders
   with its file + tree state). We apply the events to wstore + `publish_events`. **No layout
   placement is enqueued**, so the block never renders docked (no flash).
3. Run `sagas::tear_off_block::run(state, block_id, source_tab_id, source_ws_id)` →
   `CreateWorkspace`→`CreateTab`→`MoveBlock` → `{new_workspace_id, new_tab_id}`, then
   `service::setup_torn_off_block_layout` (now `pub(crate)`) makes the block the new tab's root.
4. Broadcast `waveobj:update` for the new workspace/tab/layout/block on the event bus (mirrors the
   docked path + the tear-off DnD handler) so other frontends' WaveObj caches sync.
5. Resolve the **source window**: `state.windows` entry whose `workspace_id == source_ws_id` →
   `window_id`. Publish a scoped directive via the WPS broker:
   `broker.publish(WaveEvent{ event: "openfloatingpane", scopes: [window_id],
   data: {block_id, workspace_id: new_workspace_id} })`. (Scoped delivery goes through the broker, not
   `event_bus.broadcast_event` which is unfiltered.) Transient directive — no new reducer `Event`.
6. Return `PaneOpenResult{ block_id, tab_id: new_tab_id, view, created: true }`.

Frontend (`initGlobalEventSubs`, scoped to `windowId`):

```ts
{ eventType: "openfloatingpane", scope: initOpts.windowId, handler: (event) => {
    const { block_id, workspace_id } = event.data;
    // geometry computed frontend-side (srv has no cursor): centered default size
    invokeCommand("open_floating_pane_window", {
        pane_id: block_id, workspace_id, x, y, width, height,
    });
}}
```

The new floater window boots → `?floatingPaneId&workspaceId` → `FloatingPaneWorkspace` renders the
editor. **No new window machinery** — reuses `tear_off_block` (saga), `open_floating_pane_window`
(host), `FloatingPaneWorkspace` (renderer), and the scoped-event channel.

### 3.3 Why server-owns over docked-then-tear-off
Alternative: open docked, then have the frontend call `TearOffBlock` + `open_floating_pane_window`
(literal drag-path replay). Rejected: it flashes the pane docked first and splits the orchestration
across two round-trips. Server-owns keeps the reducer-state move atomic in one `open_pane` call
(the architecture's contract: srv owns reducer state via sagas; frontend owns the OS-window spawn).

---

## 4. File-by-file change list

| File | Change | Status |
|---|---|---|
| `agentmux-mcp/src/main.rs` | `collapse_tree` + `floating` tool params → body fields | ✅ done |
| `agentmux-srv/src/backend/rpc_types.rs` | `CommandPaneOpenData.tree_expanded` + `.floating` | ✅ done |
| `agentmux-srv/src/server/app_api.rs` | `build_pane_meta` tree key; `open_pane` floating branch + `open_pane_floating` (reducer CreateBlock + tear_off_block saga + window resolution + scoped broker directive) | ✅ done |
| `agentmux-srv/src/server/service.rs` | `setup_torn_off_block_layout` → `pub(crate)` | ✅ done |
| `frontend/app/store/global.ts` | `openfloatingpane` subscription (scope=windowId) → host `open_floating_pane_window` | ✅ done |
| `.changesets/` | `feat(pane.open): OpenEditor collapse_tree + floating-pane support` | ✅ done |

No new host command, no new reducer `Event` variant, no new saga. `cargo check -p agentmux-srv
-p agentmux-mcp` green; `tsc --noEmit` clean for `global.ts`. **Floating still needs a CEF-build
smoke test** (the srv→frontend→host cross-process path can't be unit-tested).

---

## 5. Testing

- **Collapsed tree:** `OpenEditor(file, collapse_tree:true)` → editor opens with tree hidden;
  toggling still works (meta round-trips). `cargo check -p agentmux-srv -p agentmux-mcp` (green).
- **Floating:** `OpenEditor(file, floating:true)` → a chromeless floating window appears showing the
  editor; the source window is unchanged; closing the floater behaves like a torn-off pane.
  Requires a full `task package` / `task dev` (CEF) build to smoke — the cross-process path can't be
  unit-tested.
- **Regression:** plain `OpenEditor(file)` unchanged (docked split); `split`/`title` unaffected.

---

## 6. Risks / open items

- **Window resolution** (source workspace → `window_id`) relies on the main window being registered
  in `state.windows` via `CreateWindow` (it is, per `app-init.ts:282`). If a window is missing from
  the map, the directive won't route — fall back is "no floater opens" (safe, logged), not a crash.
- **Geometry**: srv has no cursor, so the frontend picks a default size/position (centered). Could
  later accept `x/y/width/height` hints on the tool.
- **Scope id parity**: frontend `initOpts.windowId` must equal the reducer's `window_id` for the
  source window (same id used by the working `userinput` scoped sub — reused deliberately).
- Floating panes are subordinate (owned) windows on Windows; macOS/Linux Phase A is a frameless CEF
  window (redock is a follow-up per the macOS spec). Behavior matches the existing drag tear-off.
