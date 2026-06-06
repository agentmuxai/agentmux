# Analysis: Pane click-to-focus-border paint latency

**Date:** 2026-05-28
**Status:** Investigation (no code change yet)
**Scope:** Time from `mousedown`/`click` on a pane → visible focused-border
color change on `.block-mask`.

## TL;DR

The visible focused-border color change is gated on **one reactive
signal propagation + one CSS color update on a layer that runs a
`backdrop-filter: blur(...)`**. The signal propagation kicks off four
side effects in parallel, three of which are pure cost on the path to
first-paint of the new border:

1. A full `updateTree()` rebalance+layout pass that recomputes every
   leaf's transform even though no topology changed (cost: O(nodes),
   runs synchronously inside the reducer).
2. A whole-tree-state atom replacement that re-fires every block's
   `isFocused` memo regardless of whether that block was involved
   (cost: O(nodes) reactive checks).
3. Two `console.log` calls on the DOM focus capture path
   (`block.tsx:191,193`), one of which serializes the click target via
   `getElemAsStr(event.target)` walking the DOM. Cheap when devtools
   are closed, **measurable** when open.
4. A `main_window_focus` IPC dispatched into the host (`block.tsx:214`)
   that posts a CEF UI-thread task to reclaim Win32 focus + defocus
   every pane. Doesn't block paint but contends with the CEF UI thread
   in the same frame.

The actual border-color change is a single CSS variable swap. The
visual delay you perceive is dominated by **(1) updateTree + (2)
whole-atom replacement + (the GPU compositing pass triggered by
backdrop-filter)** — not by the network/IPC roundtrip (persistence is
debounced 100ms and lives off the paint path).

Highest-impact, lowest-risk fix: **skip `updateTree()` for
FocusNode-only actions**, since the tree topology and per-leaf
transforms are unchanged. Expected to remove most of the synchronous
work from the click handler. Two-line change, all in
`frontend/layout/lib/layoutModel.ts`.

---

## 1. The end-to-end chain

```
[USER]   mousedown on .block element
   ↓
[DOM]    onClick handler fires on .block-frame
   ↓
[block.tsx:247] handleBlockClick() {
   ↓
   ├─ setBlockClicked(true)                                     // SolidJS signal write (cheap)
   ├─ if (!focusWithin) setFocusTarget()                        // view-specific giveFocus() OR focus a hidden input
   │      └─→ may trigger DOM focus event → handleChildFocus()  // see "DOM focus path" below
   └─ if (!isFocused()) nodeModel.focusNode()
         ↓
[layoutNodeModels.ts:70] focusNode → model.focusNode(nodeid)
         ↓
[layoutModel.ts:730] model.focusNode(nodeId) → focusNodeImpl
         ↓
[layoutFocus.ts:152] focusNode(model, nodeId) {
   ├─ if (model.focusedNodeId === nodeId) return;               // early-return — second click on same pane is free
   ├─ findNode(rootNode, nodeId)                                // O(nodes) tree walk
   └─ model.treeReducer({ type: FocusNode, nodeId })
}
         ↓
[layoutModel.ts:504] treeReducer(action) {
   ├─ case FocusNode:
   │     ├─ focusNode(treeState, action)                        // mutates treeState.focusedNodeId
   │     └─ focusManager.requestNodeFocus()                     // NO-OP — focusManager.ts:30
   └─ if (setState) {
        ├─ this.updateTree()                                    // ★★★ updateTree (hot path)
        ├─ this.localTreeStateAtom._set({ ...this.treeState })  // ★★ whole-state replacement
        └─ this.persistToBackend()                              // debounced 100ms, off paint path
      }
}
```

`updateTree()` calls `updateTreeImpl(model, balanceTree=true)` which:

- `balanceNode(rootNode, callback)` walks the entire tree top-down.
- For every node, `updateTreeHelper` computes `additionalProps` (rect,
  transform, magnify state, gap-corrected sizes) and pushes leafs into
  `newLeafs`.
- Reads several signals: `pendingTreeAction`, `resizeHandleSizePx`,
  `magnifiedNodeSizeAtom`. Each `getter()` is a SolidJS signal read.
- Final pass sets atoms: `additionalProps`, `leafOrder`, etc.

All of this work is correct and necessary when topology, sizes, or
magnify state change. **None of it is needed for a FocusNode-only
action** — focus is a property on `treeState.focusedNodeId`, doesn't
change layout geometry.

```
[layoutModel.ts:589] this.localTreeStateAtom._set({ ...this.treeState })
   ↓
[SolidJS] propagates to every subscriber of localTreeStateAtom
   ↓
[layoutNodeModels.ts:47] every block's isFocused memo re-evaluates:
   isFocused = createMemo(() => {
       const treeState = model.localTreeStateAtom();           // subscribe to whole state
       return treeState.focusedNodeId === nodeid;              // narrow comparison
   })
```

For a tab with N blocks, this is N memo evaluations per focus change.
For most blocks `treeState.focusedNodeId === nodeid` is the same value
as before (still false), and SolidJS's memo dependency tracking will
NOT re-render those blocks. So the cost is N comparisons, not N
renders — bounded, but still a wide signal fan-out for a narrow
change.

```
[block.tsx:164] isFocused = nodeModel.isFocused;
   ↓
[blockframe.tsx:755] class={clsx({
      "block-focused": isFocused() || props.preview,
      ...
   })}
   ↓
[DOM] .block class toggle on the previously-focused and newly-focused blocks
   ↓
[CSS] .block-focused .block-mask { border-color: var(--accent-color); z-index: 50 }
   ↓
[GPU] composite pass — .block-mask has backdrop-filter inside .block-content
                       (see block.scss:476-485). Triggers a re-rasterize
                       of the affected layer.
   ↓
[VISIBLE] new border color paints
```

### DOM focus path (parallel side effect)

If `setFocusTarget` triggered an actual DOM focus change:

```
[DOM]    focusin event bubbles up
   ↓
[block.tsx:190] handleChildFocus(event) {
   ├─ console.log("setFocusedChild", blockId, getElemAsStr(event.target))    // ★ getElemAsStr walks DOM
   ├─ if (!isFocused()) {
   │     console.log("focusedChild focus", blockId)                          // ★ second console.log
   │     nodeModel.focusNode()                                               // re-enters the focusNode path above
   │  }
   └─ invokeCommand("main_window_focus", { window_label })                   // fire-and-forget IPC
}
```

The two `console.log` calls fire on every focus event. Visible
overhead is small with devtools closed; with devtools open they're
each a few hundred microseconds — and `getElemAsStr` walks the DOM to
build a tag.classname#id breadcrumb. The IPC is fire-and-forget but
the backend handler (`ipc.rs:475 main_window_focus`) posts a
`MainFocusReclaimTask` to the CEF UI thread that walks browser_panes
to defocus_all + calls `host.set_focus` on the target browser —
contention on the same UI thread that paints.

---

## 2. Where the perceived latency comes from

Ranked by impact on first-paint:

| # | Site | Cost | Avoidable? |
|---|---|---|---|
| **1** | `updateTree()` inside FocusNode reducer (`layoutModel.ts:588`) | O(nodes) tree walk + per-leaf transform recompute + N signal writes. Synchronous on the click path. | **Yes** — topology and sizes unchanged. Pure waste for FocusNode actions. |
| **2** | `.block-mask` GPU compositing on `block-focused` class toggle | `backdrop-filter` forces a re-rasterize of the focused layer on each class change. On software-compositing Linux (GPU/WebGL blocklisted in this dev env), the rasterize is CPU-bound and visible. | **Partially** — see §3.B. The blur is only meaningfully visible during the magnify gesture; the focus border doesn't need it. |
| **3** | Whole-state atom replacement (`localTreeStateAtom._set({...treeState})`) → N `isFocused` memo evaluations | O(blocks) reactive checks. Each is cheap, but the spread copy + atom swap propagates through every subscriber of the layout state, including non-focus consumers (additionalProps, leafOrder, magnified, ...). | **Partially** — a dedicated `focusedNodeIdAtom` would narrow the fan-out. Bigger refactor. |
| **4** | `findNode(rootNode, nodeId)` inside `layoutFocus.ts::focusNode` | O(nodes) tree walk to locate the leaf. Runs even though the click handler already has the nodeId. | **Yes** — the existence check could be skipped on the hot path or replaced with a flat-map lookup. Small fish. |
| **5** | `console.log("setFocusedChild", ..., getElemAsStr(event.target))` (`block.tsx:191`) | Two console.log + DOM walk. Cheap with devtools closed. Visible with devtools open. | **Yes** — drop or gate behind a debug flag. |
| **6** | `main_window_focus` IPC + `MainFocusReclaimTask` on CEF UI thread | Doesn't block paint, but contends with the same UI thread CEF uses for rendering this frame. | **Partially** — could be skipped when the click target is in the same window already-focused. |
| 7 | `viewModel.giveFocus()` (`setFocusTarget` in block.tsx:217) | View-specific. Terminal: xterm focus call. Agent: input focus. Each can force layout. | Already gated by `focusWithin` check; minor. |
| 8 | `persistToBackend()` (`layoutPersistence.ts:261`) | Debounced 100ms then writes the WaveObject → IPC. | Already off-path. |

The persistence call is correctly debounced and async — it lives off
the paint path entirely. The user-perceived delay is items 1–3.

---

## 3. Optimizations, ranked by leverage

### A. Skip `updateTree()` for FocusNode actions ★★★ (2 lines, low risk)

**File:** `frontend/layout/lib/layoutModel.ts:587-591`

Current:
```ts
if (setState) {
    this.updateTree();
    this.localTreeStateAtom._set({ ...this.treeState });
    this.persistToBackend();
}
```

Proposed:
```ts
if (setState) {
    // FocusNode changes only treeState.focusedNodeId — no topology
    // or geometry change. Skip the per-leaf transform recompute;
    // savings scale with #panes in the tab.
    if (action.type !== LayoutTreeActionType.FocusNode) {
        this.updateTree();
    }
    this.localTreeStateAtom._set({ ...this.treeState });
    this.persistToBackend();
}
```

**Risk audit:** `updateTree` produces `additionalPropsAtom`,
`leafOrderAtom`, etc. For FocusNode, none of those derived values
change. Subscribers that depend on them (placeholder transforms,
resize hints) don't need to re-run. The only consumers that DO need
to update are the `isFocused` memos, which subscribe to
`localTreeStateAtom` directly — and we still set that.

`validateFocusedNode` runs inside `updateTreeImpl` and could
defensively re-check that the focused node still exists in the tree —
but FocusNode just SET that id from a node we just looked up via
`findNode`, so it's tautologically valid.

**Expected win:** removes the dominant synchronous cost from the
click handler. Bigger trees → bigger savings.

### B. Decouple `.block-mask` border from `backdrop-filter` layer ★★

**File:** `frontend/app/block/block.scss`

The focused-border lives on `.block-mask`, which has both:
- `border: 2px solid var(--border-color)` (the visible ring)
- `backdrop-filter` (inherited from the surrounding context, see
  block.scss:483 comment).

Class toggle changes only the `border-color`, but the element is
backed by a backdrop-filter layer — every change forces the GPU (or
CPU on this software-rendering setup) to re-rasterize the filtered
layer.

**Proposal:** move the focused-ring border to a sibling element OR
the `.block-frame` itself (which is already a compositing layer for
other reasons), and keep `.block-mask` static during focus changes.
The terminal's xterm composited-scroll constraint that made
`.block-mask` the chosen carrier (per the `backdrop-filter` comment)
is about z-stacking; a separate `.block-frame-focus-ring` element
sized identically + positioned on top works equally well.

**Risk audit:** the comment at block.scss:483 documents *why*
backdrop-filter is currently on `.block-mask` — to composite above
xterm's overflow-y:scroll layer. Need to verify that the new
focus-ring element also wins the stacking order, OR move the ring to
`.block-frame` and rely on the existing `position: relative` already
established by block.scss (block.scss:15-19 mentions this).

**Expected win:** removes a software-rasterize pass per focus change
on Linux/Wayland-without-GPU. Bigger on heavy-content panes (browser,
editor with LSP overlays).

### C. Drop or gate the two debug `console.log`s ★ (3 lines)

**File:** `frontend/app/block/block.tsx:191,193`

```ts
const handleChildFocus = (event: FocusEvent) => {
    console.log("setFocusedChild", nodeModel.blockId, getElemAsStr(event.target));  // ← remove
    if (!isFocused()) {
        console.log("focusedChild focus", nodeModel.blockId);                       // ← remove
        nodeModel.focusNode();
    }
    // ...
};
```

These were diagnostic logs that survived their investigation. Gate
behind a `Logger.debug("focus:diag", ...)` (which is suppressed in
production) or delete. Cheap with devtools closed; visible with
devtools open or with the host's CONSOLE relay enabled (which is
on in this dev build — every `console.log` round-trips into the host
log).

**Expected win:** measurable when devtools is open or the host's
console-relay sink is hot.

### D. Skip `main_window_focus` IPC when target is already in focus window ★

**File:** `frontend/app/block/block.tsx:212-214`

Add a guard: if `document.hasFocus()` and the click was within the
same window, don't send the IPC. (Today every DOM focus event sends
it, even for intra-window focus moves.)

**Expected win:** removes a CEF UI thread task per focus change.
Doesn't affect first paint but reduces concurrent contention on the
rendering thread.

### E. Narrow the layout state atom to `focusedNodeId` ★★ (refactor)

**File:** `frontend/layout/lib/layoutModel.ts`, `layoutNodeModels.ts`

Split `localTreeStateAtom` into two atoms: one for
`focusedNodeIdAtom` (narrow, only fires when focus changes) and one
for the rest of treeState. `isFocused` memos subscribe to the narrow
one.

**Risk audit:** wider refactor. Other reactive paths that subscribe
to localTreeStateAtom would still need to fire for the full set of
changes; only `isFocused` benefits. Probably overkill if (A) and (B)
land.

### F. Eager paint via View Transitions API (browser-supported)

`document.startViewTransition()` lets the browser snapshot before and
animate to after. For a border-color flip we don't need animation,
but the API hints to the browser "this is a discrete visual change,
prepare the layer". Marginal — and unsupported on Wayland WebKit
versions we may target. Not recommended yet.

---

## 4. Measurement plan

Before/after for any of A–E:

1. With devtools open, Performance recorder armed.
2. Click an unfocused pane.
3. Stop after the new border appears.
4. Measure: `mousedown → first paint with new .block-mask border-color`.
5. Compare across runs (5×) before vs. after the change.

A `[focus:perf]` console mark + measure pair would automate this:

```ts
performance.mark("focus-click");
nodeModel.focusNode();
queueMicrotask(() => {
    performance.mark("focus-microtask");
    performance.measure("focus → microtask", "focus-click", "focus-microtask");
});
```

Combine with `requestAnimationFrame` for the paint-side mark.

Target: **sub-16ms (one frame at 60Hz)** for the focus-click → paint
round-trip on this hardware. Today's perceived "delay" suggests
we're somewhere in the 50–150ms range; A+B should comfortably land in
budget.

---

## 5. Recommendation

Land A and C in one small PR — they're each a few lines and address
the dominant cost (A) and the noise (C). Measure with the markers
above, then decide whether B is worth the extra refactor risk based
on the measured remainder.

E (atom narrowing) and D (IPC guard) are good follow-ups once A is
in; their wins are smaller and they touch more code.

## 6. References

- `frontend/app/block/block.tsx` — `handleBlockClick`,
  `handleChildFocus`, `setFocusTarget`.
- `frontend/layout/lib/layoutModel.ts:504-591` — `treeReducer` with
  FocusNode case and the post-action `if (setState)` block.
- `frontend/layout/lib/layoutFocus.ts:152` — `focusNode` exported.
- `frontend/layout/lib/layoutGeometry.ts::updateTree` — the tree
  walker that's hot today.
- `frontend/layout/lib/layoutNodeModels.ts:47` — `isFocused` memo
  binding.
- `frontend/app/block/block.scss:476-501` — focused-border CSS,
  `backdrop-filter` comment.
- `agentmux-cef/src/ipc.rs:475` — `main_window_focus` IPC handler
  and the UI-thread task it posts.
- `frontend/app/store/focusManager.ts` — `requestNodeFocus` is a
  no-op, the call sites in `treeReducer` are leftover from a previous
  iteration.
