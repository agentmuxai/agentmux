# Spec: Pane Drag-to-Tab (Cross-Tab Pane Relocation via Drag & Drop)

**Date:** 2026-07-10
**Status:** Draft
**Repo state:** `main` @ `42b95715`
**Scope:** In-window pane drag onto the tab bar (dropping a tile-layout pane onto a *different tab in the same window*). Cross-window drag (already handled by `DragOverlay`/`CrossWindowDragMonitor`) and touch/pen input are out of scope.

---

## 1. Problem

Today there is no way to drag a pane out of its tab's split layout and drop it into a different tab in the same window. Two related gaps make this worse than "missing feature":

1. **No drop target exists for it.** `tabbar.tsx`'s `monitorForElements` only accepts `source.data.type === tabItemType` (tab reorder). A pane's `draggable()` payload is `{ kind: "tile", ... }` (`tileItemType`) — the tab bar never registers a target for it.
2. **Releasing over the tab bar today mis-fires as a tear-off.** `TileLayout`'s own `onDrop` resets `isDragging` but does not clear `currentDragPayload` (its comment says clearing happens in a `dropTargetForElements.onDrop` — but no such target exists over the tab bar). The subsequent native `dragend` is then picked up by `CrossWindowDragMonitor`, which finds no other AgentMux window under the cursor and treats the release as a tear-off, **spawning an unwanted floating pane window**. This is a real, reproducible bug this feature must close as a side effect (see §7).

Desired UX (from the request):
1. User starts dragging a pane (existing).
2. Hovering over a tab in the bar makes that tab **flash** to signal "valid drop target."
3. The UI **stitches a live preview** of the target tab's layout into view near that tab.
4. While still holding the mouse, a **ghost** inside that preview shows exactly where the pane will land, tracking pointer movement.
5. On release, the pane actually moves into the target tab's tree at the indicated slot.

---

## 2. Current Architecture (What Exists — Reuse, Don't Reinvent)

### 2.1 Layout tree = per-tab reducer, already live for hidden tabs

Each tab owns one `LayoutModel` (`frontend/layout/lib/layoutModel.ts`), a classic dispatch reducer (`treeReducer`, `types.ts:65-82` `LayoutTreeActionType` enum) over a mutable `LayoutTreeState`:

```ts
// frontend/layout/lib/types.ts:193-253
interface LayoutNode {
    id: string;
    data?: TabLayoutData;   // { blockId } on a leaf
    children?: LayoutNode[];
    flexDirection: FlexDirection;  // Row | Column
    size: number;                  // flex-share, NOT a percentage
    // minimize-feature bookkeeping fields omitted
}
type LayoutTreeState = {
    rootNode: LayoutNode;
    focusedNodeId?: string;
    magnifiedNodeId?: string;
    leafOrder?: LeafOrderEntry[];
    pendingBackendActions: LayoutActionData[];
};
```

The two actions this feature needs already exist and already carve out of the target's own size rather than diluting siblings (`sizeFraction`, `types.ts:145-167`):

```ts
interface LayoutTreeSplitHorizontalAction extends LayoutTreeAction {
    type: LayoutTreeActionType.SplitHorizontal;
    targetNodeId: string;
    newNode: LayoutNode;
    position: "before" | "after";
    sizeFraction?: number;  // new node's share of targetNodeId's CURRENT size
}
// LayoutTreeSplitVerticalAction is identical, orthogonal axis
```

**Critically, every tab's `LayoutModel` exists in memory simultaneously, not just the active tab's** (`layoutModelHooks.ts:13,41,47,52` — `layoutModelMap: Map<string, LayoutModel>`, `getLayoutModelForTabById(tabId)`). The backend-action subscription is explicitly wired for all tabs, not just the active one (tear-off windows create a `LayoutModel` before `activeTabId` syncs). This is possible because `workspace.tsx:20-40` keeps **every tab mounted** (`<For each={allTabIds()}>`), hiding inactive ones via `display:none` rather than unmounting.

**Consequence for this feature:** the target (inactive) tab's tree is already reachable and dispatchable via `getLayoutModelForTabById(targetTabId)` — we do not need to construct or load anything. The catch: its DOM is `display:none`, so `getBoundingClientRect()`-derived geometry is 0×0 while hidden. **We cannot hit-test against the real DOM for the hover preview** (§4.2 covers the workaround).

### 2.2 Existing tab bar drag (reorder-only) — `frontend/app/tab/*`

Library: [`@atlaskit/pragmatic-drag-and-drop`](https://github.com/atlassian/pragmatic-drag-and-drop) (`draggable()` / `dropTargetForElements()` / `monitorForElements()`), used throughout — not react-dnd, not custom pointer handling.

- `droppable-tab.tsx` registers `draggable()` per tab with `getInitialData: () => ({ tabId, workspaceId, tabIndex, type: tabItemType })`.
- **The tab bar is a single `monitorForElements`** (`tabbar.tsx:188`), not per-tab drop targets. Reorder position comes from `computeInsertionPoint(clientX)` (`tabbar-dnd.ts:43-81`) against a `Map<tabId, HTMLDivElement>` registry of tab wrapper refs.
- Visual feedback today is **gap-widening only** (`insertionPoint` signal drives `padding-left`/`padding-right` on the two flanking tabs) — **there is no highlight/flash on the hovered tab itself anywhere in the codebase.** The "flash" beat in this spec is net-new UI, not an extension of an existing affordance.
- `onDrop` on the tab-bar monitor hit-tests the strip's own rect (`dropInsideBar`) before committing.

### 2.3 Pane drag payload — `frontend/layout/lib/TileLayout.*.tsx`

```ts
// same shape in CrossWindowDragMonitor.win32/.darwin/.linux.tsx:35-37
export type DragItemPayload =
    | { kind: "tile"; node: LayoutNode }
    | { kind: "tab"; tabId: string; workspaceId: string };
```

`setCurrentDragPayload`/`getCurrentDragPayload` is module-level "what's being dragged right now" state, written by both tab drag (`kind: "tab"`) and pane drag (`kind: "tile"`), read by `CrossWindowDragMonitor`. `TileLayout`'s `draggable()` is registered on the pane's header (`[data-role="block-header"]`, not the tile body — see the WebView2 comment at `TileLayout.win32.tsx:405-420`), with a native drag-image PNG (`onGenerateDragPreview` → `toPng`).

### 2.4 The closest prior art: `RedockFloatingPane`

Cross-window pane drop (an already-solved, structurally identical problem — move a block into a *different tab's* tree, from a live drag, at a user-chosen slot) uses a two-part pattern worth copying exactly:

**Backend (`agentmux-srv/src/sagas/redock_floating_pane.rs`):** the saga is intentionally ownership-only —

```rust
// run_inner, redock_floating_pane.rs:149-163
ctx.dispatch(Command::MoveBlock {
    block_id, src_tab_id: source_tab_id, dst_tab_id: target_tab_id.clone(), dst_index,
}).await
```

Its own doc comment states: *"the saga itself does not touch LayoutState... backend just stores."* The RPC handler that **wraps** the saga (`workspace.rs:1151-1230`) does the layout-tree work, separately, after the saga succeeds:

```rust
// workspace.rs:1188  — the saga (ownership move only)
let saga_result = crate::sagas::redock_floating_pane::run(...).await;
// workspace.rs:1212 — target tab: split the tree at the exact previewed slot
queue_target_layout_split(state, &target_tab_id, &block_id, tbid, dir).await
// workspace.rs:1226 — source tab: remove the now-relocated leaf
queue_source_layout_delete(state, &source_tab_id, &block_id).await
```

`queue_target_layout_split` (`layout_helpers.rs:123-159`) maps an 8-direction `DropDirection` to `SplitHorizontal`/`SplitVertical` + `before`/`after`, with `sizeFraction` fixed at `0.5` for inner directions (Top/Right/Bottom/Left) and `0.2` for outer band directions (OuterTop/.../OuterLeft), falling back to a plain `InsertNode` for `Center`/unknown. Both queue functions append to the target/source tab's own `LayoutState.pendingbackendactions` array — **not** an RPC response — which each tab's already-live `LayoutModel` picks up via `layoutPersistence.ts`'s `processPendingBackendActions → handleBackendAction → model.treeReducer(action, /*setState=*/false)`, working correctly even while that tab is `display:none` (§2.1). This is exactly the delivery mechanism this feature needs, with zero changes.

**Contrast — `MoveBlockToTab` is NOT this pattern (a trap, not a template):** `WorkspaceService.MoveBlockToTab(wsId, blockId, sourceTabId, destTabId, autoClose)` (`services.ts:180`, called only from `DragOverlay.tsx:97` on cross-window pane drop into another window's *active* tab) dispatches `Command::MoveBlock` and handles auto-closing an emptied source tab (`workspace.rs:535-620`) — **and nothing else**. It never calls `queue_target_layout_split`/`queue_source_layout_delete`. Grepping confirms zero references to either from this handler. **This is an existing gap in the cross-window-drop path** (dropping a pane into another window's active tab today updates `Tab.blockids` membership but not that tab's visual tree) — out of scope to fix here, but do not copy this handler's shape for the new feature; copy `RedockFloatingPane`'s instead.

### 2.5 Ghost/preview precedent for a *live, direction-aware* drop indicator

`app-init.ts:113-273` (`installFloatingRedockHoverListener`) is the closest existing "ghost stitched into a target's layout, updating live while a real drag is in progress": on `floating-redock:hover-state` host events (cursor position during a floating-window OS-level move), it does `elementFromPoint` → nearest `[data-blockid]` leaf → `determineDropDirection` → `rectForDirection` (hardcoded Top/Right/Bottom/Left = half-leaf, Outer* = 1/5 band, Center = full-leaf) → positions a raw `.floating-redock-drop-placeholder` div, gated by a 180ms dwell timer on Windows (`REDOCK_DWELL_MS`) to prevent flicker.

**This only works because its target is a visible, active tab's real DOM** (the dragged window is a separate OS process, the target tab is necessarily the one currently on screen in that other window). **Our target tab is usually inactive and `display:none`**, so `elementFromPoint`/`getBoundingClientRect` are not usable directly — see §4.2.

---

## 3. Goals

1. Drag a pane from its current tab and drop it into a different tab in the same window, landing at a user-chosen slot within that tab's layout.
2. While hovering a candidate tab (mouse still down), the tab visually flashes to confirm it is a valid drop target.
3. While continuing to hover the same tab, show a live schematic preview of that tab's current layout, with a ghost rect tracking the pointer to indicate exact landing position (which leaf, which side).
4. Fix the existing mis-fire where releasing a dragged pane over the tab bar today spawns a floating tear-off window instead of doing nothing / being handled.
5. Reuse the existing `RedockFloatingPane`-style backend split described in §2.4 rather than inventing a new persistence path.

## Non-Goals

- Cross-window pane→inactive-tab drop (still limited to the active tab of another window via `DragOverlay`, per today's behavior) — a natural phase 2, deferred because it requires cross-process schematic-preview delivery, not just in-renderer state.
- Fixing `MoveBlockToTab`'s existing layout-tree gap (§2.4) — noted, not fixed, to keep this spec's blast radius to the new code path only.
- Keyboard-only / accessibility-parity pane relocation — deferred, tracked separately.
- Reordering panes within the *same* tab via the tab bar (already out of scope; that's `TileLayout`'s existing in-tab drop targets).

---

## 4. Design

### 4.1 New drop target on the tab bar for `tileItemType`

`tabbar.tsx`'s existing `monitorForElements` only monitors `tabItemType`. Add a second `canMonitor` branch (or a parallel monitor instance) that accepts `source.data.type === tileItemType`, reading the payload as `DragItemPayload & { kind: "tile" }`. On `onDragEnter`/`onDrag` (pragmatic-dnd's per-move callback), hit-test cursor X/Y against each tab wrapper's rect (the same `tabWrapperRefs` registry `tabbar-dnd.ts` already maintains) to determine which tab, if any, is currently hovered.

```ts
// New signal, colocated with insertionPoint/bouncingTabId in tabbar.tsx
const [hoveredDropTabId, setHoveredDropTabId] = createSignal<string | null>(null);
```

Set only when the dragged payload is `kind: "tile"` and the tile's *own* tab differs from the hovered tab (dragging over your own current tab is a no-op, matching `MoveBlockToTab`'s existing same-tab short-circuit at `workspace.rs:562-564`).

### 4.2 Tab flash (Goal 2)

A CSS class (e.g. `.tab--drop-flash`) toggled by `hoveredDropTabId`, applied as a restartable keyframe pulse (not a static highlight) so re-entering the same tab after briefly leaving re-triggers the flash — mirrors the existing `bouncingTabId` one-shot-animation pattern (`tabbar.tsx`) rather than inventing a new animation-retrigger mechanism. Dwell-gate this at ~120–180ms (matching `REDOCK_DWELL_MS`'s precedent, §2.5) before the preview (§4.3) mounts, so a fast pass-through over several tabs doesn't spawn/despawn previews per-frame.

### 4.3 Schematic tab-layout preview (Goal 3) — the genuinely new piece

No thumbnail/mini-layout preview of any kind exists anywhere in the codebase today (confirmed by grep — zero hits for thumbnail/mini-layout/schematic/preview in this context). Because the target tab is `display:none` (§2.1), we cannot render or hit-test its real DOM. Build a **schematic renderer**: a small popover anchored below/near the hovered tab (e.g. 240×160px, non-interactive except for hosting the ghost) that:

1. Reads `getLayoutModelForTabById(hoveredDropTabId).treeState.rootNode` directly (already live, per §2.1 — no fetch/load step).
2. Recursively lays out the schematic using the **same pure geometry function** the real layout uses — `layoutGeometry.ts`'s `updateTreeHelper` takes an arbitrary `boundingRect`, not necessarily the real DOM's, so pass the popover's own box instead of a real element's rect. This is a direct reuse, not a reimplementation of split-tree math.
3. Renders each leaf as a plain `<div>` rect (no live pane content — a schematic, not a live thumbnail) scaled to the popover.

### 4.4 Ghost tracking within the preview (Goal 3, continued)

While `hoveredDropTabId` is set and the popover is mounted, track pointer position relative to the popover, hit-test against the schematic leaf rects, and reuse the existing `determineDropDirection` (quadrant-based: which half/outer-band of the hovered leaf the pointer is in) — the same function `TileLayout`'s in-tab `OverlayNode` drop targets already use, and the same 9-way `DropDirection` vocabulary (`types.ts:35-45`) that `queue_target_layout_split` (§2.4) already consumes. Render a highlighted sub-rect within the popover (via the existing `rectForDirection` half/1-5-band/full-leaf convention from `app-init.ts:150-174`) as the ghost. No new direction math — only a new rect source (schematic, not real DOM).

### 4.5 Drop commit (Goal 1, 5)

On release with `hoveredDropTabId` set and a resolved `(targetLeafId, direction)`:

1. Call a **new** RPC modeled directly on `RedockFloatingPane`'s two-call pattern (§2.4), not on `MoveBlockToTab`. Simplest option: add a same-window variant, e.g. `WorkspaceService.MovePaneToTab(wsId, blockId, sourceTabId, destTabId, targetBlockId, direction)`, backed by a new saga (or a thin extension of the existing `RedockFloatingPane` saga/RPC handler if source and target workspace happen to be the same — the saga's own guard at `redock_floating_pane.rs:106-111` only rejects *same-tab* moves, not same-workspace-different-tab ones, so this may be reusable near-verbatim rather than net-new; confirm during implementation whether `RedockFloatingPane`'s pre-condition checks (`source_workspace_id`/`target_workspace_id` params) can simply both be set to the current window's workspace id).
2. Handler: `MoveBlock` (ownership only) → on success, `queue_target_layout_split(target_tab_id, block_id, targetLeafId, direction)` → `queue_source_layout_delete(source_tab_id, block_id)`. Both land via each tab's `pendingbackendactions`, consumed by the already-subscribed (even while hidden) `LayoutModel` (§2.1) — **no special-casing needed for the target tab being inactive.**
3. Clear `hoveredDropTabId`, unmount the popover, clear `currentDragPayload`.

### 4.6 Direction default when no fine-grained slot is chosen

If the user drops without dwelling long enough for the preview to mount (fast flick), fall back to `queue_target_layout_insert` (append) exactly as `queue_target_layout_split`'s own `Center`/unknown-direction branch already does (`layout_helpers.rs:141-142`) — no new fallback logic needed, just don't block the drop on the preview having mounted.

---

## 5. Lifecycle / State Machine

Mapped to the five UX beats from §1:

| Step | Trigger | State change | Existing vs. new |
|---|---|---|---|
| 1. Drag start | `TileLayout` `draggable().onDragStart` | `setCurrentDragPayload({kind:"tile", node})` | Existing |
| 2. Hover a tab | pointer move over tab bar, `kind:"tile"` payload | `hoveredDropTabId` set (after dwell gate) | **New** (§4.1) |
| 2. Flash | `hoveredDropTabId` changes | `.tab--drop-flash` keyframe restarts | **New** (§4.2) |
| 3. Stitch preview | `hoveredDropTabId` stable past dwell | schematic popover mounts, reads live (hidden) `LayoutModel` tree | **New** (§4.3) |
| 4. Ghost tracks pointer | pointer move within popover | `determineDropDirection` recomputed against schematic rects; ghost rect updates | Reused math (§2.5), new rect source (§4.4) |
| 4. Leave tab / drag elsewhere | pointer leaves tab+popover region | `hoveredDropTabId` → null, popover unmounts | **New** |
| 5. Drop | `mouseup` / pragmatic-dnd `onDrop` while `hoveredDropTabId` set | new RPC call → `MoveBlock` + `queue_target_layout_split` + `queue_source_layout_delete` | Backend pattern reused from `RedockFloatingPane` (§2.4), new RPC entry point |
| Cleanup (always) | drag ends, any path | `currentDragPayload` cleared, `hoveredDropTabId` cleared, popover unmounted | Fixes §7 bug |

---

## 6. Files Touched

| File | Change |
|---|---|
| `frontend/app/tab/tabbar.tsx` | Add `hoveredDropTabId`/`bouncingTabId`-style signal; extend (or add a second) `monitorForElements` to accept `tileItemType`; wire flash class + popover mount/unmount |
| `frontend/app/tab/tabbar-dnd.ts` | Extend tab-wrapper-rect hit-testing helper to be reusable for tile-over-tab hover, not just tab-reorder insertion point |
| `frontend/app/tab/tab.tsx` | Apply `.tab--drop-flash` class when `hoveredDropTabId === thisTabId` |
| `frontend/app/tab/tab.scss` | New `.tab--drop-flash` keyframe pulse |
| **New:** `frontend/app/tab/tab-drop-preview.tsx` | Schematic popover component: reads target `LayoutModel`, renders via `updateTreeHelper`, hosts ghost rect |
| `frontend/layout/lib/TileLayout.win32/.darwin/.linux.tsx` | On `onDrop`, if `hoveredDropTabId` was set at release, call the new cross-tab move RPC instead of falling through to in-tab drop handling; ensure `currentDragPayload` is cleared here so `CrossWindowDragMonitor` cannot treat the release as an unhandled tear-off (fixes §7) |
| `frontend/app/store/services.ts` | New `WorkspaceService.MovePaneToTab(...)` method (or confirm `RedockFloatingPane`'s existing RPC can be called with same-workspace ids — see §4.5.1) |
| `agentmux-srv/src/server/service/workspace.rs` | New RPC arm (or same-workspace call path into the existing `"RedockFloatingPane"` arm, `workspace.rs:1151-1230`) |
| `agentmux-srv/src/sagas/redock_floating_pane.rs` | Confirm/relax the "source and target are the same tab" guard doesn't also incorrectly reject same-workspace-different-tab (it shouldn't — re-verify during implementation) |

**Backend blast radius:** no schema/persistence changes — this reuses `LayoutActionData`/`pendingbackendactions`/`Command::MoveBlock` verbatim. Only a new RPC arm (or a relaxed guard on an existing one) and its saga wiring.

---

## 7. Bug Fix Bundled With This Feature

Today: drag a pane, release it over a sibling tab → `TileLayout`'s element-level `onDrop` fires (native dragend always fires on the drag source, regardless of drop target), clears `isDragging` but not `currentDragPayload` → `CrossWindowDragMonitor` sees an unhandled `dragend` with no AgentMux window under the cursor → misinterprets it as a tear-off → **spawns a new floating pane window.**

This spec's new tab-bar drop target (§4.1) must call `event.preventDefault()`-equivalent / consume the drop and explicitly clear `currentDragPayload` in its own `onDrop`, before `TileLayout`'s fallback `onDrop` or `CrossWindowDragMonitor`'s dragend listener runs. Ordering here matters and should be covered by an integration test (§8) — this is the one place a race could silently reintroduce the tear-off mis-fire.

---

## 8. Implementation Order

1. Fix §7's clear-payload-on-consumed-drop ordering in isolation first (smallest, highest-value, verifiable independently of the rest of the feature — turns "drag pane over tab bar, release" from "spawns a floating window" into "silently does nothing").
2. Add `hoveredDropTabId` + tab flash (§4.1, §4.2) — visually verifiable with no backend changes.
3. Add the schematic preview popover (§4.3) using the reused `updateTreeHelper` — verifiable in isolation against a live tab's tree.
4. Add ghost tracking + `determineDropDirection` reuse (§4.4) — completes the frontend-only preview.
5. Add the new RPC + saga wiring (§4.5), modeled on `RedockFloatingPane` — confirm whether the existing saga's guard can be reused with same-workspace ids before writing a new one.
6. Wire drop commit (§4.5) end-to-end; remove any temporary console-only drop handling from step 2–4.

---

## 9. Testing Guidance

- Unit: `determineDropDirection` against schematic (non-1:1-scaled) rects returns the same direction as against real rects for equivalent relative cursor positions (regression guard for §4.4 reusing the function outside its original DOM-rect context).
- Unit: `queue_target_layout_split`/`queue_source_layout_delete` are unchanged by this feature (no new tests needed if the new RPC calls them as-is — confirms no signature drift).
- Integration: drag a pane from tab A onto tab B (inactive) → switch to tab B → the pane appears in the previewed slot, tab A no longer shows it, and no floating window was spawned in the process (regression test for §7).
- Integration: drag a pane, hover tab B then tab C then release over tab C → only tab C receives the pane (hover state transitions cleanly, no stale `hoveredDropTabId`).
- Integration: drag a pane over its own current tab → no flash, no preview, drop is a no-op (mirrors `MoveBlockToTab`'s existing same-tab short-circuit).
- Manual: verify the flash re-triggers on re-entering the same tab after a brief exit (not just a static highlight that plays once).

---

## 10. References

- `docs/analysis/ANALYSIS_FLOATING_PANE_GHOST_LANDING_DISCONNECT_2026_07_04.md` — the `sizeFraction` fix this feature depends on; read before touching `queue_target_layout_split`.
- `docs/specs/SPEC_FLOATING_PANE_REDOCK_2026-05-27.md` — original design doc for `RedockFloatingPane`, the pattern this spec copies.
- `frontend/layout/lib/layoutModelHooks.ts`, `layoutTree.ts`, `layoutGeometry.ts`, `layoutPersistence.ts` — reducer, tree ops, geometry, and backend-action delivery respectively.
- `frontend/app/tab/tabbar.tsx`, `tabbar-dnd.ts`, `droppable-tab.tsx` — existing tab-reorder drag to extend.
- `frontend/app-init.ts:113-273` (`installFloatingRedockHoverListener`) — direction/ghost-rect vocabulary to reuse in §4.4.
- `agentmux-srv/src/sagas/redock_floating_pane.rs`, `agentmux-srv/src/server/service/layout_helpers.rs`, `workspace.rs:1151-1230` — backend pattern this spec's new RPC should mirror.
