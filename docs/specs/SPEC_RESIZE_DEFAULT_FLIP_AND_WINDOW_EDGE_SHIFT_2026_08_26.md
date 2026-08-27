# SPEC: Resize refinements — flip group/direct defaults, and Shift+window-resize feeding only the edge panes

**Date:** 2026-08-26
**Status:** analysis + design — not implemented
**Author:** Loap (agent)
**Tracking discussion:** repo owner, this session — "the default should be the
relative resize that currently shift-resize does. The new feature is that when
users use shift-resize, it directly resizes the pane, like how the default
resize is currently. we also want introduce this to the pane borders along the
window. when window resizing, the default already does relative. but we want
to add a shift+resize to the edge panes that make pane size change on it's own
(other panes remain same width even though you are making the window larger)."

**Builds on / supersedes in part:**
- `SPEC_SHIFT_DRAG_GROUP_RESIZE_2026_08_03.md` (PR #2401) — introduced
  Shift+drag group resize. **This spec flips its §5.1/§5.2 default/modifier
  assignment**; everything else there (Scope A, the modifier being Shift,
  minimize-locked exclusion) stands.
- `SPEC_SHIFT_DRAG_GROUP_RESIZE_DIRECTION_FIX_2026_08_17.md` — the two-block
  distribution algorithm. Unchanged; it simply becomes the *default* math.

---

## 1. What exists today (verified by reading the code)

### 1.1 Splitter drag — two modes, Shift selects the group one

- `frontend/layout/lib/TileLayout.{win32,darwin,linux}.tsx` each pass the live
  modifier into the resize hot path:
  `onResizeMove(props.resizeHandleProps, clientX, clientY, event.shiftKey)`
  (win32: ~line 516, darwin: 433, linux: 457). This is the ONLY place mode
  selection happens.
- `frontend/layout/lib/layoutResize.ts::onResizeMove` (line 238-341) branches
  on that boolean:
  - `groupResize === false` (**today's default**): plain 2-node
    equal-and-opposite transfer between the two flanking siblings
    (`beforeNodeSize`/`afterNodeSize`, line 313-326), both floored at
    `MinNodeSizePx = 128` — the drag simply stops at the floor.
  - `groupResize === true` (**today's Shift**): `computeGroupResizeSizes`
    (line 165-211) — the two-block model: every sibling before the handle is
    one uniformly-scaling block, the driven pane plus everyone after it is
    the other; the handle border tracks the cursor 1:1; per-member floor with
    iterative redistribution (`shrinkBlockBy`, line 54-96).
- The drag context (`ResizeContext`, line 18-36) always snapshots
  `groupSiblingStartSizes` regardless of the modifier, precisely so Shift can
  be toggled mid-drag with no context rebuild. **This means the flip is
  near-free** — both formulas already run off the same context.
- Discoverability surface: `frontend/app/element/quicktips.tsx:213-215` —
  "Resize All Panes Together — Shift + Drag".

### 1.2 Window resize — proportional by construction, no code involved

Pane sizes are flex weights (`LayoutNode.size`) relative to their parent's
children; pixels are derived per-parent as `containerPx × weight/Σweights`
(`pixelToSizeRatio` in `additionalProps`). An OS window resize reaches the
layout only as a `ResizeObserver` tick (`layoutModelHooks.ts:69` →
`onContainerResize`, `layoutResize.ts:216-220`), which recomputes geometry
and briefly sets `isContainerResizing` (to suppress animations). **No weight
changes, so every pane scales proportionally — the "relative" default the
repo owner described.** Notably, this reflow does NOT enforce the 128px
floor (only interactive drags do — documented in `shrinkBlockBy`'s comment,
line 60-71).

There is **no host-side resize-loop handling today**: no
`WM_SIZING`/`WM_ENTERSIZEMOVE`/`WM_EXITSIZEMOVE` handlers anywhere in
`agentmux-cef/`. Precedent for sampling physical key state inside a native
loop exists (`GetAsyncKeyState` in `agentmux-cef/src/commands/drag.rs:244`
and `ui_tasks/drag.rs:337-349`), and host→renderer event channels already
exist for exactly this kind of mid-gesture signaling (the tabdrag events,
`srv_event_bridge.rs` / `events.rs`).

---

## 2. Change 1 — flip the splitter-drag defaults

### 2.1 New behavior

| Gesture | Old | New |
|---|---|---|
| Plain drag | 2-node direct transfer | **group resize (two-block)** |
| Shift + drag | group resize (two-block) | **2-node direct transfer** |

Rationale (owner-stated): the group behavior is the better everyday default —
one drag adjusts the whole row/column coherently; the surgical "move only
this border, touch only these two panes" case is the rarer, deliberate
action, which is what a modifier is for.

### 2.2 Implementation surface (small, by design)

- `TileLayout.{win32,darwin,linux}.tsx` — pass `!event.shiftKey` where
  `event.shiftKey` is passed today. Recommended cleanup while there: rename
  `onResizeMove`'s 4th param from `groupResize` to something
  gesture-agnostic (e.g. keep `groupResize` but compute it at the call site:
  `groupResize: !event.shiftKey`) so the semantics live in one place.
- `layoutResize.ts` — no math changes. Update the §5.2 comment block
  (line 299-304) that says "Shift held:" to describe the flipped mapping.
- `quicktips.tsx:213-215` — flip the entry: default drag now resizes the
  row/column together; **"Resize Single Border — Shift + Drag"** (or similar
  wording) documents the modifier.
- Mid-drag toggling keeps working unchanged (both formulas already share the
  same snapshot context, §1.1) — releasing/pressing Shift mid-drag now flips
  in the opposite direction, which needs one manual re-verification pass.

### 2.3 Behavioral deltas worth calling out (so they're chosen, not accidental)

1. **The default drag's driven pane is no longer pixel-exact** when panes sit
   past it — only the dragged border tracks the cursor 1:1 (the two-block
   model's documented trade-off, DIRECTION_FIX §4.2). This becomes the
   everyday behavior. Judged acceptable: the border under the pointer is
   what the user is steering.
2. **The default drag can now push distant borders** — a user who has
   carefully sized pane A will see A shrink when they drag an unrelated
   border in the same row (proportionally). That's the point of the flip,
   but it's the one regression-shaped complaint to expect; the Shift escape
   hatch is the answer, and the quicktips entry must say so.
3. Minimize-locked siblings: unchanged — excluded from the group pool
   (original spec §5.3), and the 2-node path already refuses drags flanking
   a locked node (`layoutResize.ts:264`).

---

## 3. Change 2 — Shift + OS-window resize feeds only the edge panes

### 3.1 New behavior

- Plain window resize: unchanged — proportional scaling (already the
  default, zero code).
- **Shift held while resizing the window by an edge:** the entire pixel
  delta goes to the pane(s) abutting the dragged window edge; every other
  pane keeps its **pixel** size exactly (their flex weights are rewritten to
  preserve pixels under the new container size).

### 3.2 Which panes are "the edge panes" — recursive edge-chain rule

The layout is a tree, not a grid, so "the pane at the right edge" must be
defined structurally. For a **width** delta on the **right** edge, walk the
tree from the root:

- **Row container:** only the **last** (rightmost) child's width changes —
  recurse into it if it's itself a container; every other child keeps its
  pixel width (weights recomputed: `w_i' = W · p_i' / P'` with `p_i'` the
  preserved pixel size, keeping the parent's weight sum `W` normalized).
- **Column container:** every child spans the full width — recurse into
  **every** child.

Mirror for left edge (first child), and for height deltas on top/bottom with
Row/Column roles swapped. A **corner** drag decomposes into the two axis
rules applied independently. The affected set is exactly the panes visually
touching the dragged edge — which is what a user pointing at "the edge pane"
means.

### 3.3 Floor handling — inward spill, then proportional fallback

When shrinking the window with Shift held, the edge pane shrinks first. At
its 128px floor (`MinNodeSizePx`), the remaining delta **spills inward** to
the next sibling in from the edge (accordion-style), and so on. If every
sibling in a container is at the floor, fall back to plain proportional
scaling for further shrinkage — matching the existing non-Shift reality that
window reflow may push panes below the floor (§1.2), so the two modes
converge instead of the Shift mode wedging against an OS resize it cannot
refuse. Growing has no cap (no max-size concept exists — same as splitter
drags).

`shrinkBlockBy` is the right primitive to reuse for the spill (it already
implements floored, pool-dropping redistribution) — but note its
distribution is proportional-within-the-pool; the accordion order described
here wants **edge-first ordering** instead. That's a new small pure function
(`spillInwardBy`?) alongside it, unit-testable the same way.

### 3.4 Gesture plumbing — how Shift and the dragged edge reach the layout

Two facts the renderer cannot reliably know on its own:
1. **Which edge** is being dragged — a `ResizeObserver` tick only reports new
   size.
2. **Whether Shift is held** — during a native resize modal loop the renderer
   may not receive key events, and focus is on the window frame.

**Recommended: host-driven (Windows first).** The host window proc handles:
- `WM_ENTERSIZEMOVE` → begin a resize session: snapshot request to renderer
  (`windowresize:begin`).
- `WM_SIZING` → carries the edge verbatim in `wParam`
  (`WMSZ_LEFT/RIGHT/TOP/BOTTOM/corners`); sample
  `GetAsyncKeyState(VK_SHIFT)` (precedent: `drag.rs:244`) per tick; forward
  `windowresize:tick { edge, shiftHeld }` to the renderer. Shift can
  therefore be pressed/released mid-resize and the mode follows live, same
  as splitter drags.
- `WM_EXITSIZEMOVE` → `windowresize:end` — renderer commits (one history
  entry, mirroring the splitter drag's stage-then-commit pattern,
  `SetPendingAction`/`CommitPendingAction`).

**macOS/Linux (phase 2):** no `WM_SIZING` equivalent with an edge param.
The edge is inferable from frame geometry deltas alone (origin.x moved with
width → left edge; width changed with origin fixed → right edge; likewise
vertically), which works from `windowWillResize`/configure events — or even
renderer-side via `window.screenX/screenY` deltas as a fallback. Shift
state is the harder half there (renderer key tracking is best-effort during
a native loop); ship Windows first and reuse whatever pattern the tear-off
work already proved per-platform.

### 3.5 Renderer-side algorithm

On `windowresize:begin`: snapshot every container's children pixel sizes
(from `additionalProps` rects — same source the drag context uses,
`layoutResize.ts:266-267`). Per `tick` with `shiftHeld`:

1. Compute the cumulative per-axis delta from the session-start container
   size (CSS px — divide out `--zoomfactor` if the observer reports zoomed
   units; verify against how `pixelToSizeRatio` is derived).
2. Apply the §3.2 recursion: for each affected container, produce a
   `ResizeNodeOperation[]` from the snapshot + delta (§3.3 spill math),
   staged via `SetPendingAction` exactly like a splitter drag tick.
3. Ticks with `shiftHeld === false` restage pure-snapshot-proportional
   sizes (i.e. no weight change relative to session start) so toggling
   Shift mid-resize is live and reversible within the session.

On `end`: `CommitPendingAction` (single undo/persist entry). A session with
Shift never held stages nothing and commits nothing — identical to today.

Note: unlike plain window resize (which never writes the tree), a
Shift-resize **persists changed weights** — that's the intent ("other panes
remain same width"), but it means the layout file changes on a
window-resize gesture for the first time. Worth one deliberate sign-off.

### 3.6 Out of scope / non-goals

- Floating panes (`floating_pane.rs`) — separate windows, own resize spec
  (`SPEC_FLOATING_PANE_EDGE_RESIZE_2026_05_29.md`); untouched.
- Magnified-pane state during a window resize — the magnify overlay already
  has its own reflow; not special-cased here.
- Maximize/restore, snap layouts, DPI-change reflows — these are not
  edge-drag gestures; they keep proportional behavior unconditionally
  (no `WM_SIZING` stream → no session → no Shift path, by construction).
- A persistent per-workspace "mode" toggle — this stays a momentary
  modifier, consistent with the splitter-drag feature.

---

## 4. Consistency note — one mental model after both changes

After both changes, **Shift uniformly means "surgical/direct"** and plain
gestures mean "the whole neighborhood adjusts":

| Gesture | Plain | Shift |
|---|---|---|
| Splitter drag | row/column adjusts together (relative) | only the two flanking panes (direct) |
| Window edge drag | everything scales (relative) | only the edge pane changes (direct) |

This inversion is the strongest argument for the flip: before it, Shift
meant "bigger effect" on splitters but would have meant "smaller effect" on
window edges — incoherent. After it, one rule covers both surfaces.

---

## 5. Test plan

**Change 1 (flip):**
- Existing `layoutResize.test.ts` suites stay green untouched (they test the
  pure functions, which don't move).
- Update/extend any test that encodes the modifier mapping (none found at
  the pure-function layer — the mapping lives in the three `TileLayout`
  variants, currently untested; a small component-level test asserting
  which branch `onResizeMove` takes per `shiftKey` value would lock the flip).
- Manual: plain drag moves the whole row; Shift drag moves one border;
  toggling mid-drag flips live, in both directions.

**Change 2 (window edge):**
- Unit: the new spill function (edge-first floored redistribution) and the
  weight-rewrite math (`preserve pixels under new total`), pure-function
  tests in the `layoutResize.test.ts` style, including corner (two-axis)
  cases and the all-at-floor proportional fallback.
- Manual (Windows): 3-pane row — Shift-drag right window edge wider: only
  the rightmost pane grows, others' pixel widths hold (verify with the
  `WxH` dimension overlay, which mounts on `isContainerResizing` /
  splitter drags — check it also engages here or extend it); shrink until
  the edge pane floors, confirm inward spill; release Shift mid-gesture,
  confirm reversion to proportional staging; nested split (column inside
  the row) — confirm recursion touches the correct leaves.
- Persistence: after a Shift window-resize, reload — the changed pane
  weights survive; after a plain window-resize, the layout file is
  byte-unchanged (today's behavior preserved).

## 6. Sources (code read for this spec)

- `frontend/layout/lib/layoutResize.ts:18-36, 54-96, 165-211, 216-229, 238-341`
- `frontend/layout/lib/TileLayout.win32.tsx:516`, `TileLayout.darwin.tsx:433`,
  `TileLayout.linux.tsx:457`
- `frontend/layout/lib/layoutModelHooks.ts:69`
- `frontend/app/element/quicktips.tsx:53-58, 213-215`
- `agentmux-cef/src/commands/drag.rs:244-245`, `agentmux-cef/src/ui_tasks/drag.rs:337-349`
  (GetAsyncKeyState precedent); repo-wide grep confirming no
  `WM_SIZING`/`WM_ENTERSIZEMOVE` handling exists yet
- `docs/specs/SPEC_SHIFT_DRAG_GROUP_RESIZE_2026_08_03.md`,
  `docs/specs/SPEC_SHIFT_DRAG_GROUP_RESIZE_DIRECTION_FIX_2026_08_17.md`
