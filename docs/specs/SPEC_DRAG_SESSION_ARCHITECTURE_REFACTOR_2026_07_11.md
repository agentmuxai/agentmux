# Spec: Drag-Session Architecture Refactor (Cross-Tab/Cross-Window Drag, Layout Persistence, Block Registry)

**Date:** 2026-07-11
**Status:** Draft — analysis complete, refactor proposed
**Repo state:** branch `fix/strobe-invert-reduced-motion-source-delete` @ `fdfb4282` (working tree; `main` @ `8b3bc3b4`)
**Scope:** The pane/tab drag system end-to-end (gesture state, overlay input capture, cross-tab commit, layout persistence, block-component registry). Supersedes the incremental-fix approach of `SPEC_PANE_DRAG_TO_TAB_2026_07_10.md` addenda A2 rounds 1–4.

---

## 1. Why a refactor: the field-incident record

Four rounds of point fixes over one day, each fixing the previous round's symptom and
exposing (or creating) the next:

| Round | Symptom | Fix applied | What it exposed |
|---|---|---|---|
| 1 | Slash-circle cursor, no hover UI (v1 popover design) | v2 spring-loaded tabs, real drop targets | worked, but… |
| 2 | Dead target tab after drag | 3-layer overlay cleanup nets; ghost/landing clamp; geometry refresh | source pane lingered in source tab |
| 3 | Moved pane remained in source tab | local source-delete at RPC completion + reactive `pruneDanglingLeaves` | dead **source** tab (mid-drag unmount suppressing dragend) |
| 4 | Dead source tab again | `dragInFlight` gate, deferred prune, all-tab overlay reset, probes | drag stopped committing at all; source pane freezes |

Probes from round 4 proved the previous theory insufficient: dragend fired, cleanup ran,
overlays were reset — **the tab still died**. The final inventory (§2) found *two
independent* dead-tab mechanisms and one fresh regression, all rooted in the same
structural defects. Point fixes cannot converge because each new writer/net changes the
race lattice for the others.

---

## 2. Architectural defects (from the full code inventory, ranked)

Full state/writer tables live in the analysis that produced this spec; the defects, with
one-line evidence:

1. **Drop-teardown depends on a browser event documented not to fire.** All
   overlay/`activeDrag` reset hangs off the drop/dragend chain, while pragmatic-dnd's own
   source states `dragend` "does not fire if the draggable source has been removed during
   the drag" (`lifecycle-manager.js:264-267`) — and the feature's data flow removes the
   source leaf mid-gesture.
2. **`activeDrag` is one boolean with two meanings** ("my own drag" on the source model;
   "a foreign drag is hovering me" force-set on the spring target) — and the target owns
   no draggable that could ever reset it (`droppable-tab.tsx:234` sets; only
   `tabbar.tsx:466` resets).
3. **The full-surface `.overlay-container` (z-5, `pointer-events:auto` while
   `activeDrag`) sits ABOVE the pane headers that host the draggables** — a stuck overlay
   is therefore both "dead tab" *and* "can't start a new drag from this tab," unifying the
   two headline symptoms (`TileLayout.win32.tsx:640-645`, `theme.scss:116/121`).
4. **The block-component registry is `Map<blockId, model>` with last-writer-wins register
   and unconditional delete-on-cleanup**, while every tab stays mounted. Double-mount a
   block (any dangling leaf) → the LATER mount overwrites the entry → the EARLIER mount's
   unmount **deletes the survivor's registration and disposes its ViewModel**
   (`global.ts:522-535`, `block.tsx:290/296`). Focus routing then silently fails for that
   pane: the second, registry-level dead-tab mechanism.
5. **Persistence is a blind, un-versioned, whole-`LayoutState` overwrite** on a 100ms
   debounce — including writing `pendingbackendactions: undefined` after processing
   (`layoutPersistence.ts:342-360`). Any stale local tree durably clobbers backend prunes
   AND queued actions. This is the engine of the documented stale-tree resurrection.
6. **Drag lifecycle state is scattered across ~14 stores in 3 layers** (TileLayout module
   globals ×3 platform copies, per-model signals, `crossTabDrag`/`dragInFlight`
   singletons, `tabbar-dnd` signals, `CrossWindowDragMonitor` global) with no single owner.
7. **Cleanup is defense-in-depth by accretion** — five-plus redundant, differently-gated
   teardown layers, several keyed on `dragActivatedTabIds`, because no single reliable
   teardown signal was identified. (One exists — see §3.1.)
8. **`pruneDanglingLeaves` (round 3-4) tracks the whole `Tab` object reactively and can
   delete legitimate leaves**: a freshly split/inserted pane whose block isn't in the
   frontend's `tab.blockids` *yet* gets pruned-and-persisted. **This is the round-4
   regression that made drops stop sticking.**
9. **The cross-tab commit fights the frontend's own mutate-then-persist contract** —
   the source delete had to be deferred (to protect dragend), leaving a double-mount
   window that must be swept asynchronously (defect 4 fires inside that window).
10. **Dead state drift**: `showOverlay` has no writer but is OR'd into the overlay
    transform — a symptom of an over-grown model.

### 2.1 ROOT CAUSE FOUND (post-spec, via the §7 hit-test probe): the placeholder-container click shield

The post-switch hit-test diagnostic identified the actual dead-tab layer in the
field: **`div.placeholder-container`** — the purely-visual drag-ghost layer — was
the top element at the tab center on every wedged tab, while `activeDrag` was
correctly `false` and the overlay-container was correctly `pointer-events: none`.

Mechanism: `overlayTransform` (`layoutModel.ts:441-455`) parks the placeholder
"offscreen" at `rect.top + 2 × rect.height` of the display container. When a
cross-tab drag ends, the SOURCE tab is `display:none` (spring-switched away), so
`getBoundingClientRect()` is all zeros and the parking spot computes to
**`top: 0` — exactly covering the tab** — and `.placeholder-container` carried
no `pointer-events: none` (only its children did). The memo's rect read is
non-reactive, so re-showing the tab never recomputes; the one event that would
(a new drag's `activeDrag` toggle) is unreachable because the layer covers the
drag handles. This single defect produced every field symptom across all four
rounds: dead source tab, drags that won't start from it, healing on reload,
re-breaking on the next move.

Fixed immediately (same branch): `pointer-events: none` on the container +
a large constant parking floor. The refactor's P2 (derived overlay state) and
the general lesson stand: **decorative layers must be input-transparent by
construction, and "offscreen" must never be computed from a possibly-zero
live rect.**

Two load-bearing facts from pragmatic-dnd's source that the refactor builds on:

- **Dispatch order at drop is fixed**: source draggable `onDrop` → drop targets
  (innermost→outermost) → **monitors last** (`make-adapter.js:16-28`).
- **Monitor `onDrop` fires for every gesture end** — commit, cancel (ESC/no-target), and
  even swallowed-dragend via the broken-drag `pointerdown` fallback
  (`lifecycle-manager.js:126-129`, `detect-broken-drag.js`). **Monitors are the single
  reliable teardown signal the current code never trusted.**

---

## 3. Design

### 3.1 P1 — One owner: `DragSession` (explicit state machine)

New module `frontend/layout/lib/dragSession.ts`, the sole owner of gesture state:

```
idle ──onDragStart──▶ dragging ──(monitor onDrop)──▶ settling ──commit/teardown──▶ idle
```

- **Inputs**: TileLayout draggable callbacks (start), overlay/tab drop-target callbacks
  (record hover + drop intent), ONE pragmatic monitor (gesture end — reliable per §2).
- **Owned state** (replaces all of): `globalDragNodeId/LayoutModel/Node`,
  `dragInFlight`, `crossTabDrop`, `hoveredDropTabId`, `dragActivatedTabIds`,
  spring timers, `currentDragPayload`'s tile variant.
- **Derived signals** (read-only for UI): `session.active()`, `session.sourceTabId()`,
  `session.springTabId()`, `session.hoverTabId()`, `session.dropIntent()`.
- **All teardown happens in exactly one place** — the monitor-driven `settle()` — making
  the window-dragend/pointerdown nets and TileLayout's `resetDragState` deletable. The
  pointerdown net survives only as an assertion+log (it should never fire).

### 3.2 P2 — Overlay input capture becomes derived state

Split `activeDrag`'s two meanings and derive both from the session:

```ts
// per TileLayout instance
const overlayInteractive = () =>
    dragSession.active() &&
    (dragSession.sourceTabId() === myTabId || dragSession.springTabId() === myTabId);
```

No imperative set/reset pairs anywhere; when the session hits `idle`, every overlay in
every tab derives to `pointer-events: none` in the same tick. Delete `showOverlay`.
`isDragging`/tile styling likewise derive from `session.sourceNodeId()`.

### 3.3 P3 — Commit-after-teardown (kills the dragend-suppression class)

Drop handlers **only record intent** on the session:
`session.recordDrop({ kind: "cross-tab-redock", blockId, sourceTabId, targetTabId, targetBlockId, direction })`.
The session executes the RPC in `settling`, strictly AFTER pragmatic teardown completed.
Consequences:

- Our code can never unmount the drag source mid-gesture — there is no mid-gesture
  mutation at all.
- The 250ms deferred prune, the RPC-completion delete, and the dragInFlight gate all
  become unnecessary; the source leaf is removed by the normal post-commit path while the
  session is already idle.
- In-tab moves keep their current synchronous commit (the source element survives an
  in-tab move; only cross-tab commits were hazardous).

### 3.4 P4 — Block-component registry hardening (independent, do first)

Owner-checked unregister + last-write tracking:

```ts
function unregisterBlockComponentModel(blockId: string, owner: BlockComponentModel) {
    if (blockComponentModelMap.get(blockId) === owner) {
        blockComponentModelMap.delete(blockId);
        cleanupBlockAtomCache(blockId);
    } // else: a newer mount owns the key — do not clobber it
}
```

…and `Block`'s `onCleanup` disposes only the ViewModel *it* created. This makes
double-mount survivable regardless of every other defect, and is a ~20-line change with
its own tests. Even after the drag refactor, transient double-mounts remain possible
(backend races), so this hardening is required, not optional.

### 3.5 P5 — Prune correctness (fix the round-4 regression)

`pruneDanglingLeaves` stays (the self-healing invariant is right) but:

- runs ONLY when `dragSession.idle()` **and** debounced ≥500ms after the last
  `Tab`/tree change (no more per-Tab-mutation reactive sweeps);
- never prunes a leaf younger than the debounce (a fresh split whose block-ownership
  update is still in flight);
- triggers: model init, post-`settling`, and the debounced watcher — not the raw
  `onBackendUpdate` else-branch.

### 3.6 P6 — Persistence discipline (kills the resurrection engine)

Phased, backend-coordinated:

- **Phase A (frontend-only, immediate):** `persistToBackend` stops writing
  `pendingbackendactions` entirely (queue is backend-owned; srv merges on write). A stale
  frontend push can then no longer erase queued actions — the clobber becomes a benign
  re-application.
- **Phase B (srv):** version/CAS on `Command::LayoutSetTree` + `Store::update`
  (`WHERE version = ?`); frontend retries on conflict by re-reading + re-applying local
  intent. This is the durable fix the resurrection investigation already proposed.
- **Phase C (aspirational, separate spec):** action-based persistence — the frontend
  sends tree *actions*, srv owns the tree. Removes the dual-writer model entirely.

---

## 4. Immediate stabilization (ship before/with the refactor's first PR)

1. **Registry hardening (§3.4)** — smallest change, removes the worst symptom
   (permanently dead tab) even when upstream races fire.
2. **Prune de-fanging (§3.5 triggers only)** — fixes the round-4 "drops don't stick /
   panes vanish" regression.
3. **Persistence Phase A (§3.6)** — one-file change, removes the queue-clobber.

With 1–3 in place the feature is usable-if-imperfect while P1–P3 land; alternatively, the
cross-tab drop commit can be feature-flagged (`tab:crosstabdrag` setting) if field noise
must stop immediately.

## Non-Goals

- Fixing the pre-existing in-strip **tab reorder** initiation bug (tracked in
  `SPEC_PANE_DRAG_TO_TAB_2026_07_10.md` addendum A3 — zero `tab-drag started` logs for
  2+ days; orthogonal, drag never *starts*).
- The cross-window remount feature's host hook (shipped, unaffected — it reads none of
  the refactored state).
- Action-based persistence (Phase C) — separate spec once A/B prove out.

---

## 5. Files Touched (refactor core)

| File | Change |
|---|---|
| **New:** `frontend/layout/lib/dragSession.ts` | The FSM: state, derived signals, monitor wiring, `recordDrop`, `settle()` commit executor |
| `frontend/layout/lib/TileLayout.{win32,darwin,linux}.tsx` | Delete module globals + `resetDragState` net; draggable callbacks feed the session; overlay style derives per §3.2; drop handlers → `recordDrop` |
| `frontend/layout/lib/crossTabDrag.ts` | Shrinks to the redock RPC helper called by `settle()`; record moves into the session |
| `frontend/layout/lib/dragInFlight.ts` | **Deleted** (subsumed by session state) |
| `frontend/app/tab/tabbar-dnd.ts` | `hoveredDropTabId`/`dragActivatedTabIds` replaced by session signals; reorder-only state stays |
| `frontend/app/tab/droppable-tab.tsx` | Spring timer moves into session (`session.armSpring(tabId)`); tile drop-target records intent |
| `frontend/app/tab/tabbar.tsx` | Cleanup nets deleted; assertion-only pointerdown probe; strip drop target stays (cursor + payload) |
| `frontend/app/store/global.ts` + `frontend/app/block/block.tsx` | §3.4 registry hardening |
| `frontend/layout/lib/layoutPersistence.ts` | §3.5 prune triggers; §3.6 Phase A |
| `agentmux-srv` (Phase B only) | CAS on layout writes |

---

## 6. Implementation Order

1. §4.1 registry hardening (independent PR-able, unit-tested).
2. §4.2 prune de-fanging + §4.3 persistence Phase A (restores a usable baseline).
3. `dragSession.ts` FSM + TileLayout/DroppableTab/tabbar migration (the core refactor;
   in-tab drag first, cross-tab commit-after-teardown second).
4. Delete the cleanup nets + dead state (`showOverlay`, `dragInFlight`, module globals).
5. Persistence Phase B (srv CAS) — separate PR.
6. Field-test matrix: repeated A↔B pane moves with tab switches between each; ESC
   cancels; drop on strip/content/desktop; swallowed-dragend simulation (remove source
   node in devtools mid-drag) must leave every tab interactive.

---

## 7. Testing Guidance

- Unit: DragSession transition table (start→drop→settle→idle; cancel path; broken-drag
  path via synthetic monitor events); recordDrop intent consumed exactly once.
- Unit: registry — double-register then first-mount unregister leaves the second
  registration intact; ViewModel disposal ownership.
- Unit: prune — never fires while session active; never prunes younger-than-debounce
  leaves; still removes genuinely dangling leaves.
- Unit: persist payload excludes `pendingbackendactions` (Phase A).
- Integration (vitest, jsdom): full cross-tab move commits only after monitor onDrop;
  source model overlay derives to none on idle without any imperative reset.
- Manual: the §6.6 matrix, plus diags — `muxlog` must show zero
  `pointerdown net fired` assertions across a session.

---

## 8. References

- `specs/SPEC_PANE_DRAG_TO_TAB_2026_07_10.md` (+ addenda A1–A3) — feature spec and field
  incident log this refactor supersedes procedurally.
- `docs/investigations/INVESTIGATION_LAYOUT_DEAD_SPACE_STALE_TREE_RESURRECTION_2026_07_08.md`
  — the persistence clobber mechanism and srv-side pruning already landed.
- `specs/SPEC_CROSS_WINDOW_TAB_REMOUNT_2026_07_11.md` — adjacent feature; unaffected
  reader of the refactored state.
- pragmatic-drag-and-drop source: `make-adapter.js` (dispatch order),
  `lifecycle-manager.js` + `detect-broken-drag.js` (dragend suppression + fallbacks) —
  the two facts §3 builds on.
