# Research — Pane Minimize/Collapse Best Practices in Tiling Layout Systems

**Date:** 2026-07-16
**Type:** External research report (deep-research harness: 5 search angles, 21 sources
fetched, 96 claims extracted, top 25 adversarially verified by 3 independent votes each —
23 confirmed 3-0, 2 refuted 0-3, 0 unverified)
**Trigger:** Four distinct bug classes in two weeks from AgentMux's in-tree pane-minimize
design, culminating in the layout doctor capturing a live cascade producing **negative
sizes** (issue #2179). This report surveys how mature systems solve the same problem and
maps the findings 1:1 against what we already attempted.
**Companion docs:** `SPEC_LAYOUT_MINIMIZE_LOCKED_STATE_REDESIGN_2026_07_16.md` (Option B/C),
`INVESTIGATION_LAYOUT_DEAD_SPACE_STALE_TREE_RESURRECTION_2026_07_08.md`.

---

## 1. Verdict (TL;DR)

**Across every surveyed mature system, minimize/collapse is never modeled as size
arithmetic inside the flex/ratio tree.** Verified 3-0 per system, panes either:

1. **leave the tree entirely** into a dedicated dock structure (FlexLayout top-level
   `borders`, AvalonDock edge-anchored `LayoutAnchorGroup`s, Eclipse Trim Stacks,
   IntelliJ tool-window bars, i3 scratchpad), or
2. **collapse via a discrete container display mode** whose geometry is derived fresh at
   render time and never stored (i3 stacked/tabbed, FlexLayout/Dockview maximize,
   IntelliJ view modes).

The single strongest precedent: **tmux's maintainer rejected — in writing — the exact
design we built.** Asked for in-layout hidden panes, nicm warned that a "halfway" design
where panes are flagged inside the live layout so "every loop skips it *is* adding a
special kind of pane… no better than just using a special session" (i.e. out-of-tree
relocation), and tmux enforces the hard invariant that *every pane in a window's layout
is visible* — its zoom is a whole-layout snapshot swap that is forcibly undone before any
structural operation (15+ call sites).

**Our Option C (out-of-tree per-column dock) is the established pattern.** The only
in-tree collapse mature systems make work is the i3 model: a `displayMode: "stack"`
container flag whose header-stack geometry the renderer computes every pass, leaving
stored flex weights untouched. Our "slip" mechanism has **no analog in any surveyed
system**.

## 2. What we attempted (the failure history this is measured against)

| Attempt | Mechanism | Outcome |
|---|---|---|
| In-tree minimize | Squeeze `size` to header height; store `minimizedSize` for restore; "slip" (header migrates into adjacent column); "column dissolve" (fully-minimized column nests into a neighbor, converting leaves into columns) | Four bug classes below |
| Bug class 1 | `balanceNode` direction-alternation flipped a dissolved column's orientation | Point-guard (#2176) |
| Bug class 2 | Resize handles freely resized minimized panes (min-size guard was even inverted for them) | Write-point locks: `minimizedLockedSize`, reducer rejections, handle suppression (#2180) |
| Bug class 3 | Leaf→group promotions stranded leaf-only marker fields on branches (TS/Rust port divergence) | Found via live 0.53.6 DB archaeology; doctor invariant I2 |
| Bug class 4 | Cascading dissolves computed **negative** sizes (steal-from-neighbor arithmetic clamps the neighbor at a floor → `actualStolen < 0`), and the #2180 locks then *faithfully preserved the garbage* | Caught live by the layout doctor (`NONPOSITIVE_SIZE`, tree dump in dev log) |
| Diagnostics | 9-invariant layout doctor at every write choke point, both TS and Rust (PR #2184, unmerged) | Works — attributed bug class 4 in real time |

## 3. Survey: how mature systems model minimize

| System | Model | Where minimized panes live | Verified |
|---|---|---|---|
| **FlexLayout** | Out-of-tree | Top-level `borders` element — border nodes *"can only be used within the `borders` top-level element"*, structurally impossible inside the main row/tabset tree | 3-0 |
| **AvalonDock** | Out-of-tree | `ToggleAutoHide()` moves the pane out of `RootPanel` into a `LayoutAnchorGroup` on `LayoutRoot.{Left,Right,Top,Bottom}Side` | 3-0 |
| **Eclipse** | Out-of-tree (presentation) | Minimized view stacks move to **Trim Stacks** at the workbench-window edges (E4 nuance: the `MPartStack` stays in the E4 model, tagged `Minimized` and un-rendered — the *presentation* is out-of-tree) | 3-0 |
| **IntelliJ** | Out-of-tree | Tool windows attach to edge **tool-window bars** outside the editor split tree, managed by a separate `ToolWindowManager` layer; collapse behavior is one of 5 discrete view modes | 3-0 |
| **i3/sway (scratchpad)** | Out-of-tree | Minimize-equivalent moves the window to an invisible scratchpad workspace, entirely off the tiling layout | 3-0 |
| **i3/sway (stacked/tabbed)** | In-tree **display mode** | Per-container `layout` mode; non-focused children stay in the tree at full size (`percent` untouched), hidden purely by render-time stacking order; IPC tree schema has **no minimized-state or saved-size field at all** | 3-0 |
| **tmux (zoom)** | Whole-layout snapshot swap | `window_zoom()` saves `saved_layout_root`, builds a one-pane layout; unzoom swaps the snapshot back; forced unzoom before every structural op | 3-0 |
| **Dockview / FlexLayout (maximize)** | In-tree **discrete flag** | `maximizeGroup`/`MAXIMIZE_TOGGLE` store a dedicated reference (`_maximizedNode` / `maximizedTabSet`) and toggle *other views' visibility* — **no weights mutated**; Dockview `serialize()` deliberately exits maximize first so persisted dimensions stay uncorrupted | 3-0 |
| **DockPanel Suite** | Out-of-tree | Auto-hide moves content to edge strips with a dedicated overlay for temporary display | (secondary source) |

Coverage gaps (see §8): no claims about VS Code internals, Golden Layout, rc-dock,
Lumino, Qt QDockWidget, or macOS Dock survived verification — conclusions rest on the
seven systems above, all confirmed against primary docs plus source code.

## 4. The i3 insight: our "column dissolve" as a display mode

i3's stacked container is **visually identical to what column dissolve tries to build**
(a vertical stack of title bars with one/zero panes showing) — but implemented the
opposite way:

- The header stack is **recomputed by the renderer on every pass**: focused child gets
  container size minus the decoration heights of all children. Computed geometry
  (`rect`, `window_rect`, `deco_rect`) is documented as *"temporary, meaning they will be
  overwritten by calling render_con"* — rendered sizes are never a source of truth any
  operation could corrupt.
- **Orientation is not stored** — i3 deliberately removed the stored orientation field in
  2012; `con_orientation()` is a pure switch on the layout mode (vertical for
  splitv/stacking, horizontal for splith/tabbed). **Our bug class 1 is structurally
  impossible in i3**: no normalization pass can flip what is derived, not stored.
- Because the state is derived, i3 needs **no locks, no rejection rules, no
  `minimizedSize` bookkeeping** — the invariant-bearing state simply is not writable.

## 5. State-model patterns that keep these trees consistent

1. **One tree as sole source of truth.** i3 abandoned an earlier multiple-lists design as
   *"complicated to use (snapping), understand and implement."*
2. **A single command/reducer choke point.** FlexLayout: after model load, *all* changes
   go through `model.doAction()` (~19 typed actions); direct mutators are `@internal`.
   (AgentMux already has this — SPEC_864.)
3. **Computed geometry is throwaway derived state**, never read back as authoritative.
   Persistent size lives only in relative `percent`/weight fields that transient modes
   never touch. Dockview and tmux both **exit maximize/zoom before serializing** so
   display state cannot leak into stored sizes — the exact inverse of our bug class 4,
   where dissolve arithmetic leaked garbage into stored sizes and the locks preserved it.

Notably: **none of these systems relies on write-point locks or reducer rejections to
protect a size invariant.** Our #2180 lock layer is a defensible hardening of a design no
mature system uses; it treats a symptom the surveyed architectures remove at the root.

## 6. Restore semantics

- **AvalonDock** (the direct model for Option C): auto-hide stores
  `PreviousContainer = parentPane` (serialized as `PreviousContainerId`,
  `ILayoutPreviousContainer` pattern); un-hide re-docks into that stored container and
  falls back to creating a fresh pane via the layout engine only when the reference is
  null or detached (`previousContainer.Root == null`) — the documented answer to the
  **anchor-disappeared** problem.
- **Eclipse** tracks provenance: un-maximize restores only trim stacks created *by that
  maximize* (`MINIMIZED_BY_ZOOM` tag); independently minimized stacks stay minimized.
- **tmux** restores by whole-snapshot swap; any structural change invalidates the
  snapshot wholesale (forced unzoom) rather than attempting partial reconciliation.
- **i3 scratchpad** sidesteps restore entirely: the window returns *floating, centered* —
  never to its prior tree position.

Common thread: **explicit provenance references with defined fallbacks — never
reconstruction from residual size state left in the tree.** Our `minimizedSize`/
`slipMinimize.targetColumnId`/`columnDissolve.originalRowIndex` bookkeeping is a partial
reinvention of this, but stored *inside* the mutable tree where every structural op must
preserve it (bug class 3: they don't).

## 7. Recommendation for AgentMux

**Adopt Option C — it is the industry pattern, not an experiment.** Composed entirely
from verified precedents:

1. **Minimized panes leave the flex tree** into a per-column dock list of
   `{paneId, previousContainerId, savedRelativeSize}` — the AvalonDock
   `ILayoutPreviousContainer` pattern — with an explicit fallback (re-insert via the
   layout engine at a default position) when the anchor container is gone. If
   maximize-by-minimizing-others is ever added, tag dock entries Eclipse-style
   (`MINIMIZED_BY_ZOOM`) so bulk restore is scoped.
2. **All mutations stay on the reducer choke point** (we already have this). Dock
   membership becomes reducer-owned state under SPEC_864, so backend enforcement is
   structural, not arithmetic.
3. **If any in-tree collapse is retained, make it a display mode, not surgery:** replace
   "column dissolve" with `displayMode: "stack"` on the column — the renderer derives the
   header-stack geometry each pass from header count; stored flex weights are never
   touched. This deletes dissolve/undissolve structural surgery, the negative-size
   arithmetic, and the orientation coupling **simultaneously** (i3 pattern).
4. **Drop "slip".** No surveyed system implements anything comparable; its job (a lone
   Row pane collapsing sideways) is subsumed by dock placement rules.
5. **Keep the layout doctor** as the regression net during migration; note that
   invariants I2/I4/I6/I7 become *unnecessary by construction* once minimize state is
   unwritable — which is the measure of success: the doctor should end up with nothing
   to say about minimize.

What gets deleted: `_slipMinimize`, `_dissolveColumn`, `_undissolveColumn`,
`slipMinimize`/`columnDissolve`/`_slipAnchor` fields, both `balanceNode` carve-outs, the
`minimizedLockedSize` lock layer and its reducer rejections (~all of #2180's mechanism,
kept only as generic hardening if desired), and the steal-from-neighbor arithmetic.

**Bug-class → pattern map (why this can't regress):**

| Our bug class | Preventing pattern (verified) |
|---|---|
| 1 — direction flip | Orientation/geometry derived, not stored (i3) |
| 2 — resize-through-lock | Minimized panes aren't in the tree; there is no edge to resize (all dock systems) |
| 3 — stranded markers | No leaf-only marker fields in the tree; dock entry is the record (AvalonDock) |
| 4 — cascade negatives | No steal arithmetic; header strip geometry derived per render (i3); display state never serialized into sizes (Dockview/tmux) |

## 8. Caveats and open questions (from verification)

- **Coverage gaps:** VS Code internals, Golden Layout, rc-dock, Lumino, Qt QDockWidget,
  macOS Dock — no claims survived verification; conclusions rest on i3, tmux, FlexLayout,
  Dockview, AvalonDock, Eclipse, IntelliJ. Two Dockview claims (that it *lacks*
  collapse-to-edge per issue #765) were refuted 0-3 — draw no conclusion there.
- **Precision nuances:** Eclipse's E4 model keeps the minimized `MPartStack` in the model
  (tagged, un-rendered) — "out-of-tree" is strictly true of the presentation. IntelliJ
  does persist a per-window size "weight" for restore, but as memory separate from the
  collapse mechanism. FlexLayout's choke point is TS `@internal` visibility, not runtime
  immutability.
- **Open questions for the Option C spec:**
  1. When the host column itself closes: promote its dock to a neighbor column, escalate
     to a window-level tray (Eclipse/IntelliJ style), or force-restore? (AvalonDock's
     null/detached fallback answers the per-pane case, not the dock-container-vanishes
     case.)
  2. VS Code's grid/serializable-view internals remain the highest-profile unexamined
     web-tech analog — worth a targeted follow-up before finalizing the spec.
  3. Whether any "slip"-like affordance is worth re-adding as a dock placement rule.

## 9. Primary sources

- i3: userguide, hacking-howto, ipc docs; src/render.c, src/con.c, include/data.h;
  commit de94f6da1a (orientation field removal) — https://i3wm.org/docs/
- tmux: issue #3047 (maintainer quotes verified verbatim via GitHub API); window.c
  zoom/unzoom on master — https://github.com/tmux/tmux/issues/3047
- FlexLayout: README/Model.ts — https://github.com/caplin/FlexLayout
- AvalonDock: LayoutAnchorGroup wiki; LayoutAnchorable.cs:413-506 —
  https://github.com/Dirkster99/AvalonDock/wiki/LayoutAnchorGroup
- Eclipse: Workbench User Guide (min/max); MinMaxAddon.java —
  https://help.eclipse.org/latest/topic/org.eclipse.platform.doc.user/gettingStarted/qs-39g.htm
- IntelliJ: viewing-modes, manipulating-tool-windows —
  https://www.jetbrains.com/help/idea/viewing-modes.html
- Dockview: maximized-groups docs; gridview.ts —
  https://dockview.dev/docs/core/groups/maxmizedGroups/
- Adjacent practitioner failure logs: react-resizable-panels CHANGELOG (multi-year
  in-tree `collapsedSize` bug log), react-mosaic, pixeleuphoria docking-splitter notes.
