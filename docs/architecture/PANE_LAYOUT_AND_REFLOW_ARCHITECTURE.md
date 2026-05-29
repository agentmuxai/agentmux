# Pane Layout & Reflow Architecture

> Deep reference for AgentMux's tiling pane layout — how the tree is owned, how geometry is
> computed, how panes render (DOM **and** native browser HWNDs), and how (and why not yet)
> panes animate when they open/close/split. Living document.
>
> Companion: `docs/CEF_ARCHITECTURE.md` (host/CEF), `docs/analysis/ANALYSIS_PANE_OPEN_CLOSE_ANIMATION_2026_05_29.md`
> (the animation problem), `docs/specs/SPEC_PANE_REFLOW_ANIMATION_2026_05_29.md` (the animation design).

---

## 0. The one thing to understand first

A pane is **two layers that must stay aligned**:

1. A **DOM wrapper** (`.tile-node`) positioned by a CSS `transform` + explicit `width`/`height`, containing the pane body (`.block-content`). Terminal/agent/editor panes render their content **inside this DOM** (xterm canvas, CodeMirror).
2. For **browser panes only**, a **native child window (HWND)** that composites *above* the DOM at the Win32 level ("airspace"), positioned by the host via `SetWindowPos` to overlay the DOM wrapper's screen rect.

CSS can animate layer 1. It **cannot** touch layer 2 — the host moves the HWND. Any reflow animation therefore has two coordinated halves. This split is the root of every subtlety below.

```
            ┌───────────────────────────── window (CEF) ─────────────────────────────┐
  DOM tree  │  .tile-layout > .display-container > .tile-node[transform,w,h]          │
  (Chromium │     └─ .tile-leaf > .block > .block-content[innerRect w,h] > <View/>    │  ← terminal/agent/editor live HERE
   render)  │                                                                          │
            │  native browser-pane HWND  ───────────────────────────────────┐         │  ← browser panes are a SEPARATE
  Win32     │  (composites ABOVE the DOM; positioned by SetWindowPos to      │         │     OS window on top, tracking
  airspace  │   overlay the .tile-node's screen rect; hole punched in it     │         │     the DOM rect
            │   via SetWindowRgn so DOM overlays/modals show through)         │         │
            └───────────────────────────────────────────────────────────────┴─────────┘
```

---

## 1. Ownership: backend tree is authoritative, frontend is optimistic

| Concern | Backend (`agentmux-srv`) | Frontend |
|---|---|---|
| **Tree structure** (`LayoutNode` graph) | **Authoritative** — `TabRecord.rootnode`, mutated only via reducer arms, persisted to the wave-object store (SQLite). | Read-through projection (`layoutModel.treeState`), mutated **optimistically** before the server confirms. |
| **focused / magnified node id** | Authoritative (`TabRecord.focused_node_id`, `magnified_node_id`). | Projection, updated on sync events. |
| **Flex sizes** (`node.size`, relative units) | Authoritative, persisted in the tree. | Projection; converted to pixels at render time. |
| **Pixel geometry** (rects, transforms) | **Never computed.** | **Computed entirely on the frontend** from container measurements. |
| **Browser-pane window lifecycle** | block lifecycle (srv) + CEF window lifecycle (host reducer). | Calls host RPC to spawn/close/resize the HWND. |

**Key invariant:** the backend stores **relative flex sizes**, never pixel coordinates. Each window computes its own pixels from its own container size — which is how the same tab can render at different sizes in different windows.

### Backend types & ops
- `agentmux-common/src/layout_types.rs` — `LayoutNode { id, flex_direction, size, children, data: Option<{block_id}> }`, `FlexDirection {Row,Column}`, `ResizeOp`, `SplitPosition`. Leaf = `data: Some`, no children; group = `data: None`, has children.
- `agentmux-srv/src/backend/layout/mod.rs` — pure tree mutators: `insert_node`, `insert_node_at_index`, `delete_node` (collapses sole-child parents), `move_node`, `swap_nodes`, `resize_nodes` (atomic, validates all sizes ∈ [0,100] before mutating), `replace_node`, `split_horizontal/vertical`, `ensure_group_node`. **No geometry here.**
- `agentmux-srv/src/reducer/layout.rs` + `reducer.rs` — command arms (`LayoutInsertNode`, `LayoutDeleteNode`, `LayoutMoveNode`, `LayoutResizeNodes`, `SetFocusedNode`, `SetMagnifiedNode`, …) → mutate `TabRecord` → emit versioned `Event::Layout*` (every event carries a monotonic `version`).
- Persistence: `agentmux-srv/src/backend/obj.rs` `LayoutState { rootnode, focusednodeid, magnifiednodeid, … }`, one row per `tab_id` in the wstore.

### Sync path (backend ↔ frontend)
```
user action (drag/close/split) in window A
  └─ frontend layoutTree reducer applies LOCALLY (sub-ms) → UI updates optimistically
  └─ persistence layer (debounced ~100ms) dispatches Command::Layout* (with correlation_id) → srv
        srv reducer validates + applies to TabRecord.rootnode → bump_version → Event::Layout*
        wstore row mutates → WaveObjUpdate broadcast to ALL windows
          window A (originator): correlation_id matches a pending local action → no-op
          window B (other window, same tab): applies the granular mutation to its treeState
```
**Authoritative truth = srv reducer.** Frontends are optimistic projections that reconcile via versioned events.

---

## 2. Frontend layout model & geometry pipeline

Files: `frontend/layout/lib/{layoutModel,layoutTree,layoutGeometry,layoutNodeModels,layoutModelHooks,layoutResize,layoutPersistence,utils}.ts`, `TileLayout.win32.tsx`, `tilelayout.scss`.

### 2.1 Tree → geometry
- `LayoutModel.treeReducer(action)` (`layoutModel.ts`) applies the action to `treeState`, then (if `setState`): `updateTree()` → `localTreeStateAtom._set({...})` → `persistToBackend()` (debounced 100ms).
- `updateTree()` (`layoutGeometry.ts`) walks the (optionally balanced) tree via `updateTreeHelper`, producing per-node **`additionalProps`**: `{ rect, transform, resizeHandles, pixelToSizeRatio, treeKey }`. Container nodes lay children out left-to-right (Row) or top-to-bottom (Column) using `childSize / pixelToSizeRatio`. Results are committed in a `batch()` to three signals: `leafs`, `leafOrder`, `additionalProps`.
- `setTransform(rect)` (`utils.ts`) is the geometry → CSS bridge. It emits an **animatable** style object:
  ```ts
  { position:"absolute", top:0, left:0,
    transform:`translate3d(${left}px,${top}px,0)`, width:`${w}px`, height:`${h}px` }
  ```

### 2.2 Render tree (win32)
```
.tile-layout            ← style: --gap-size-px, --animation-time-s ; class "animate" gates transitions
  .display-container    ← absolute, full-size; authoritative bounding rect
    ResizeHandleWrapper  ← splitter handles (one per gap)
    DisplayNodesWrapper
      <Key each={leafs()} by={node.id}>      ← STABLE keying → tile-node DOM persists across layout changes
        DisplayNode → .tile-node             ← style = additionalProps.transform (translate3d + w/h)
          .tile-leaf                          ← single DOM node per pane (reparented when magnified)
            .block > .block-content           ← width/height from useDebouncedNodeInnerRect → <View/>
```
- `DisplayNode` (`TileLayout.win32.tsx`): `style={addlProps()?.transform}`, `id={node.id}`. Because `<Key by={node.id}>` keys on the stable node id, the **DOM element is reused** across layout changes — only its inline style mutates → a CSS transition *can* fire.
- `useNodeModel`/`getNodeModel` (`layoutNodeModels.ts`) expose per-leaf accessors: `additionalProps()`, `innerRect()` (gap-inset content size, `null` when magnified), `isFocused/isMagnified/isResizing/ready`, `animationTimeS`.
- The pane **body size** comes from `block.tsx` → `useDebouncedNodeInnerRect(nodeModel)` → `.block-content { width/height }`.

### 2.3 Resize (splitter) drag — distinct path
`layoutResize.ts`: pointer-move (throttled 10ms) → `SetPendingAction(ResizeNode)` + `updateTree(false)` (no balance) → rects update **instantly**. `isResizing()` = `isContainerResizing() || isSplitterDragging()` is **true** during a drag, which **removes the `.animate` class** so the drag tracks the cursor with no transition. On release: `CommitPendingAction` → balance + persist.

### 2.4 Reveal / first-paint gate
- `frontend/app/store/tab-reveal.ts`: on tab switch/open, sets a `tab-content-hidden` (`visibility:hidden`) gate, then lifts it after a settle window (≈80ms clean of >50ms long-tasks, hard cap 800ms) so users never see a piecemeal first paint.
- `TileLayout.win32.tsx onMount`: at **150ms** it flips `setAnimate(true)` and `layoutModel.ready._set(true)`. So the `.animate` class (and thus all tile transitions) is **off for the first 150ms** and **on thereafter** (except during resize).

---

## 3. The reflow animation (target behavior, current design, status)

**Goal:** on open/close/split/rebalance, every pane glides to its new geometry over ~150ms instead of snapping.

### 3.1 Why it's hard (recap of §0)
- DOM panes *can* ride CSS transitions on `.tile-node` (transform/size) + `.block-content` (size).
- Browser panes *cannot* — the host repositions the HWND. They must be driven separately, **in lockstep** with the DOM, or they visibly drift from their neighbors.
- Historically `DefaultAnimationTimeS = 0`, so the transition machinery (`tilelayout.scss .tile-layout.animate .tile-node { transition: width,height,transform var(--animation-time-s) }`, present since PR #829) shipped **dormant**.

### 3.2 The chosen design (one clock; native tracks DOM)
`SPEC_PANE_REFLOW_ANIMATION_2026_05_29.md`. Implemented on branch `agentx/pane-open-close-animation`:
- **DOM panes — pure CSS:** `DefaultAnimationTimeS = 0.15`; `.tile-node` transition switched to ease-out; the inner rect is applied **immediately** on open/close (was debounced — see §3.4) and `.block-content` width/height get a matching CSS transition so the body glides with the wrapper. Drag path unchanged (`.animate` off → instant). Reduced-motion zeroes the transitions.
- **Browser panes — per-frame tracking:** a shared signal `frontend/app/platform/pane-anim.ts` (`notifyPaneReflow()` / `paneReflowActive()`). `DisplayNode` pings it whenever a node's geometry changes while animating (and not resizing). `browser-view.tsx` runs a `requestAnimationFrame` loop while the signal is active, re-sampling its placeholder's live (CSS-animating) `getBoundingClientRect()` and forwarding it via the existing `browser_pane_resize` → `SetWindowPos`. The native window therefore tracks exactly what its DOM placeholder is doing — **one clock (frontend rAF), no drift, zero new Rust.**
- **Decision:** rejected a host-side (Rust/tokio) interpolation loop — it runs on a separate clock/easing and would drift against the DOM CSS transition. For browser-pane *resize* cost, the agreed fallback is **smooth position / snap size** if per-frame CEF relayout stutters on heavy pages.

### 3.3 Native pane positioning (host)
- IPC `browser_pane_resize {block_id,x,y,width,height}` (device px) → `ipc.rs` → `BrowserPaneManager::resize` → `SetWindowPos(hwnd,…,SWP_NOACTIVATE)` (`agentmux-cef/src/browser_panes.rs`). Thread-safe from the tokio IPC thread; ~1–2ms.
- The frontend browser pane reports its rect from `browser-view.tsx syncPosition()` (×devicePixelRatio), today driven by a `ResizeObserver` + 200ms poll; the animation adds the rAF driver above.
- **Airspace clip:** `browser_panes_set_overlay_clip` → `SetWindowRgn` punches holes in pane HWNDs so DOM overlays/modals show through (`pane-overlay.ts` rAF-coalesces the dispatch; only browser panes register in `pane-rect-registry.ts`). Independent of pane position.
- **Only browser panes are native.** Terminal (xterm), agent (pty/xterm), editor (CodeMirror), sysinfo, etc. are all DOM.

### 3.4 The `innerRect` debounce — the original "snap"
`useDebouncedNodeInnerRect` (`layoutModelHooks.ts`) historically delayed applying the content's `innerRect` by `animationTimeS` on open/close (instant only during resize/magnify/reduced-motion). With the wrapper transition dormant (`0s`), this meant content snapped to its new size with no animation. Even with the wrapper transition on, the debounce holds the **body** at its old size for the whole animation and then snaps it — so only the empty wrapper would ease. The fix removes the open/close debounce (apply immediately) and lets the `.block-content` CSS transition animate the body in lockstep.

### 3.5 Status — ✅ root cause found: suppressed by `prefers-reduced-motion`
The implementation (§3.2) is **correct**. It appeared to "still snap" because the **OS had *reduce motion* enabled** (`prefers-reduced-motion: reduce`), which AgentMux honors via a **global accessibility reset** in `frontend/app/app.scss`:

```scss
.prefers-reduced-motion {        // class toggled on the root from prefersReducedMotionAtom
    * { transition-duration: none !important; transition-timing-function: none !important; /* … */ }
}
```

A one-shot runtime diagnostic confirmed it (live app, `.tile-node`): `.animate` **was** applied, `--animation-time-s` **was** `0.15s`, `isResizing` false — yet computed `transition-duration: 0s` / `transition-property: none`, with `matchMedia('(prefers-reduced-motion: reduce)').matches === true`. The global `* !important` reset wins over every component transition, so it kills the new tile/content glide **and** the pre-existing drag-placeholder glide alike (which is why both "stopped" together).

**Implications / decisions:**
- With OS animations enabled (`prefers-reduced-motion: no-preference`), the reflow animation works as designed — no code change needed.
- **Product question:** is pane reflow *decorative* motion (respect reduced-motion — current, accessible default) or *essential* spatial motion (animate regardless)? If the latter, the tile/content rules (and the placeholder) must be exempted from the global reset — e.g. drive the wrapper with the **Web Animations API** from the geometry-change effect in `DisplayNode` (WAAPI isn't caught by the CSS `* { transition: none }` reset), or scope the global reset to exclude `.tile-node`/`.block-content`. Either way it should be an explicit, deliberate exemption, not an accident.
- Many developers run with OS animations off; this is the likely default test condition, so any "is the animation working?" check must first confirm `prefers-reduced-motion`.

---

## 4. Magnify, ephemeral, multi-window (brief)
- **Magnify:** `magnifiedNodeId` in tree state. The magnified pane's single `.tile-leaf` is **reparented** (DOM `appendChild`) into a centered `.magnify-pane` overlay; other tiles get `display:none` (`.tile-hidden`) so native browser HWNDs reporting 0×0 are hidden by the host. Block/view/native window survive intact.
- **Ephemeral (peek):** a node outside the tree, rendered in the overlay sized like a magnified pane; dismissed on Escape/click.
- **Multi-window / tear-off:** layout is **per-tab** (shared across windows showing the tab); each window keeps its own browser-pane label↔blockId↔HWND map (`agentmux-cef/src/reducer/panes.rs`). Tear-off moves the subtree via layout commands + reassigns the block's tab.

---

## 5. File map
| Area | Files |
|---|---|
| Backend tree + ops | `agentmux-common/src/layout_types.rs`, `agentmux-srv/src/backend/layout/mod.rs` |
| Backend reducer + persistence | `agentmux-srv/src/reducer/layout.rs`, `reducer.rs`, `state.rs`, `backend/obj.rs` |
| Host browser-pane lifecycle | `agentmux-cef/src/reducer/panes.rs`, `browser_panes.rs`, `browser_pane/*` |
| Frontend model/geometry | `frontend/layout/lib/{layoutModel,layoutTree,layoutGeometry,layoutNodeModels,layoutModelHooks,layoutResize,layoutPersistence,utils}.ts` |
| Frontend render + styles | `frontend/layout/lib/TileLayout.win32.tsx` (+ darwin/linux), `tilelayout.scss`; `frontend/app/block/block.tsx`, `block.scss` |
| Reveal gate | `frontend/app/store/tab-reveal.ts` |
| Native pane positioning (FE) | `frontend/app/view/browser/browser-view.tsx`, `frontend/app/platform/{pane-rect-registry,pane-overlay,pane-anim,ipc}.ts` |
| Reflow animation design/analysis | `docs/specs/SPEC_PANE_REFLOW_ANIMATION_2026_05_29.md`, `docs/analysis/ANALYSIS_PANE_OPEN_CLOSE_ANIMATION_2026_05_29.md` |
