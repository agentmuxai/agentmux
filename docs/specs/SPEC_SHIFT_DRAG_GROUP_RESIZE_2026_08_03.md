# SPEC: Shift+drag group resize — move all sibling panes together on one splitter drag

**Date:** 2026-08-03
**Status:** implemented — PR #2401; verified in code 2026-08-10.
**Author:** Loap (agent)
**Tracking discussion:** user request, this session — "resize a bunch of panes simultaneously… if ctrl is pressed when a pane is resized, then all the panes along that dimension resize, not just the pane border the user is dragging." User reviewed §3/§6's prior-art survey and confirmed Shift over the originally-requested Ctrl ("yes, shift sounds right … otherwise your recommendations look good").

---

## 1. Purpose

Today, dragging a splitter between two panes only ever transfers size between
those two immediate neighbors — every other pane in the same row/column stays
exactly the size it was. The user wants a modifier-held variant: hold a
modifier key while dragging any one splitter, and instead of a 2-pane
transfer, **every pane sharing that splitter's row/column resizes together**,
so a whole strip of panes can be grown or shrunk in one drag instead of N-1
sequential drags.

This is a pure interaction addition — no new panes, no layout-tree shape
changes, no new persisted state beyond what a normal resize already writes.

---

## 2. Current behavior (baseline, verified by reading the code)

Source: `frontend/layout/lib/TileLayout.{win32,darwin,linux}.tsx` (`ResizeHandle`,
~line 492-532), `frontend/layout/lib/layoutResize.ts` (`onResizeMove`/`onResizeEnd`,
line 56-142), `frontend/layout/lib/layoutGeometry.ts` (handle enumeration,
line 414-447), `frontend/layout/lib/layoutTree.ts` (`resizeNode` reducer,
line 431-453), `frontend/layout/lib/types.ts` (`LayoutNode`, `ResizeNodeOperation`,
`LayoutTreeResizeNodeAction`, line 72, 179-187, 203-272).

- The layout is a **recursive tree**, not a grid: `LayoutNode { id, children?,
  flexDirection: Row|Column, size, ... }`. `size` is a flex-unit weight among
  **one node's own children** — conserved within that `children` array only.
  Two panes that happen to render at the same pixel X/Y coordinate but live
  under different parents have no structural relationship in the data model.
- A resize handle exists **only between two consecutive entries of one
  `children` array** (`layoutGeometry.ts:423-447`). There is no "all borders
  in this row" list anywhere — handles are enumerated strictly per-parent.
- Dragging a handle (`onPointerDown` → `onPointerMove`, throttled 10ms →
  `onPointerUp`/`onLostPointerCapture`, debounced 30ms) computes a pixel
  delta from the drag's start position, converts it to a flex-weight delta,
  and does an **equal-and-opposite transfer between exactly the two flanking
  siblings**: `beforeNode.size -= diff; afterNode.size += diff`, each clamped
  to `MinNodeSizePx`. This is staged as a pending action per pointer-move
  tick (`SetPendingAction`) and committed once on release (`CommitPendingAction`
  → `onResizeEnd`), which is why a whole drag is one history entry instead of
  dozens.
- The commit is a `ResizeNode` action carrying `resizeOperations:
  ResizeNodeOperation[]` — **already a list**, not hardcoded to 2 entries.
  The reducer (`resizeNode`, `layoutTree.ts:431-453`) applies every op
  all-or-nothing after validating each (`0 ≤ size`, not minimize-locked) and
  just assigns `findNode(op.nodeId).size = op.size` per entry. Nothing about
  the action shape or reducer needs to change to carry more than 2 ops — the
  extension point already exists.
- **No modifier-key logic exists anywhere in the drag/resize/tear-off code
  today.** A repo-wide grep for `shiftKey|altKey|ctrlKey|metaKey` under
  `frontend/layout/` and the three `TileLayout.*.tsx` files returns zero
  matches in this path. (The one unrelated hit elsewhere in `frontend/` —
  `tab-reorder.ts:381`, `if (e.ctrlKey || e.metaKey) return;` — bails a
  *different* drag, tab reorder, out entirely when a modifier is held; it is
  not a resize precedent, but see §6 for why it's still worth a glance.)

One existing doc, `docs/specs/SPEC_PANE_RESIZE_DIMENSION_OVERLAY_2026_05_26.md`
§2.1, frames its "show the badge on every pane" rule around "every other pane
in the same row/column also shifts proportionally, and users often want to
see the cascade." Reading the actual resize math above, that cascade does
**not** currently happen at the data level — only the two flanking siblings'
`size` values change. If that overlay spec's framing reflects some other
reflow this document's author didn't find, it should be re-verified live
before this spec ships; it does not change anything proposed below either
way (see §9).

---

## 3. Research: how other tools handle "resize more than the two flanking panes"

No terminal multiplexer or tiling WM in this survey ships a drag-time
modifier that resizes an *arbitrary* group — the closest analogs are either
non-drag "equalize everything" commands, or full pro-app track/area
resizing, which *does* have direct precedent:

| Tool | Mechanism | Modifier | Scope |
|---|---|---|---|
| **Blender** (area borders) | drag | **Shift** | "move nearby aligned borders together while resizing" — the closest literal match to this request found anywhere. **Ctrl** on the same drag means something unrelated: snap-to-increment. |
| **Adobe Premiere Pro** (timeline tracks) | scroll/drag | **Shift** | all tracks of a type (video *or* audio) resize together. Ctrl/Alt on the same gesture instead scope to *one* track type at a time — again not "all". |
| **Kdenlive** (timeline tracks) | drag/double-click | **Shift** | all tracks of a type resize together; plain drag/double-click stays single-track. |
| **Ableton Live** (arrangement tracks) | drag | **Alt/Option** | resizes *all* tracks — the one tool in this survey where the group-resize modifier isn't Shift. |
| **Vegas Pro** (timeline tracks) | keyboard, not drag | Ctrl+Shift+arrow | all tracks; Ctrl alone is unrelated here too. |
| **Excel** (columns) | drag | *none* — pre-selection instead | select N columns first, then drag any one boundary inside the selection; **all selected columns snap to that exact new width** (an equalize-to-value outcome, not a proportional-redistribute one). |
| **tmux / i3 / sway / Windows Terminal / Zellij** | n/a | n/a | No drag-time "resize all" modifier exists in any of these. tmux/i3/sway instead treat a border shared by >2 containers (a "T-junction") as an **ambiguity to avoid** — resize commands there are documented as grabbing whichever single edge is under the cursor/focus, and users are advised to restructure the layout to avoid the shared border rather than rely on it resizing everyone. This is the cautionary case for §5.2/§8: an under-specified "resize everyone that happens to touch this point" model produces surprising, hard-to-predict results in real usage, not just here. |
| **JetBrains IDEs** | none (drag); a static "equalize" toggle exists (Advanced Settings → Editor Tabs → *Equalize proportions in nested splits*) | n/a | same story: no drag-time group modifier; equalization is a separate, non-drag action. |
| **react-resizable-panels / golden-layout** | n/a | n/a | No built-in modifier-driven group resize in either library; would be custom logic on top either way, same as this codebase. |

**Takeaway for this spec:**

1. The dominant convention across every tool that actually has this feature
   (Blender, Premiere, Kdenlive) is **Shift**, not Ctrl — see §6.
2. The dominant *algorithm* where it exists is "peers grow/shrink together,"
   not "everyone snaps to one exact value" (that's Excel's very different,
   pre-selection-driven model, not a drag modifier at all — not applicable
   here since there's no multi-select concept for splitters in this
   codebase).
3. The tmux/i3/sway experience is a warning, not a template: an
   ill-specified "resize everything that touches this border" is a known
   source of user confusion in exactly this problem space when the scope
   isn't crisply bounded to one parent/container. §5.2 exists because of
   this.

Sources: [Blender Areas manual](https://docs.blender.org/manual/en/latest/interface/window_system/areas.html), [Blender area border dragging discussion](https://devtalk.blender.org/t/different-dragging-results-for-regions-headers/25225), [Adobe Premiere Pro community — track height shortcuts](https://community.adobe.com/announcements-727/changing-track-size-timeline-track-height-1629334), [Kdenlive timeline manual](https://docs.kdenlive.org/en/user_interface/timeline.html), [Ableton Live Arrangement View manual](https://www.ableton.com/en/manual/arrangement-view/), [tmux resize-pane T-junction/all-splits issue](https://github.com/tmux/tmux/issues/1774), [i3 FAQ — window layout / nested containers](https://faq.i3wm.org/question/5762/window-layout-vertical-plus-horizontal-containers.1.html), [JetBrains YouTrack — Even split of the editor](https://youtrack.jetbrains.com/issue/IDEA-231376/Even-split-of-the-editor), [react-resizable-panels README/CHANGELOG](https://github.com/bvaughn/react-resizable-panels).

---

## 4. Scoping "the dimension" (this is the load-bearing decision)

Because the layout is a tree with per-parent flex-weight pools, not a grid
with global coordinates, "all the panes along that dimension" has two
structurally different possible meanings:

### 4.1 Scope A — all siblings under the dragged handle's own parent (recommended)

The dragged handle already belongs to exactly one parent (`parentNodeId` in
`layoutGeometry.ts`'s handle record). "That dimension" = that parent's
`flexDirection` (Row or Column); "all the panes along it" = every entry in
that parent's `children` array — already a flat, fully-enumerable list with
zero cross-branch ambiguity.

- Cheap: no new geometry pass, no cross-parent coordinate translation.
- Well-defined: no T-junction ambiguity — there is exactly one parent, so
  exactly one unambiguous sibling set.
- Fits the existing extension point exactly: `resizeOperations` already
  accepts N entries; the reducer already applies them all-or-nothing.

### 4.2 Scope B — every pane anywhere in the tree whose edge visually lines up with the dragged border's pixel coordinate, even across unrelated parents

This is the literal, maximal reading of "all the panes along that
dimension," and it is a materially different, much larger feature:

- Requires computing full render-time geometry (`layoutGeometry.ts`'s
  output) first, to find which *other* nodes — in unrelated `children`
  arrays, at different tree depths, possibly under a different
  `flexDirection` — happen to have an edge at the same pixel coordinate.
- Each affected node's `size` lives in a *different, independently-conserved
  flex-weight pool* (its own parent's). A single pixel delta has to be
  re-derived into N different pools' units, each with its own total and its
  own `MinNodeSizePx` clamp, and the pools can clamp independently, so
  "the same drag" can produce different relative effects in different
  branches.
- This is exactly the tmux/i3/sway T-junction case from §3 — the prior art
  survey found no tool that ships this as a *feature*; the ones with the
  structural precondition (tiling WMs) treat it as a bug/gotcha to design
  around, not something to lean into.

**Recommendation: ship Scope A only.** It's the well-defined, cheap,
precedented reading, it reuses existing infrastructure with no reducer or
type changes, and it matches how a user thinks about "this row of panes"
even though the underlying tree doesn't have a native "row" concept beyond
one parent's children. Scope B is called out explicitly as **out of scope**
in §9, not silently dropped, since the user's literal wording ("all the
panes along that dimension") could be read either way and a future reader
of this spec should know the ambiguity was noticed and a call was made.

---

## 5. Proposed algorithm (Scope A)

### 5.1 Baseline case unchanged

Without the modifier held, `onResizeMove` behaves exactly as it does today —
2-node equal-and-opposite transfer between the flanking siblings. Nothing
about the no-modifier path changes.

### 5.2 Modifier-held case

When the modifier is held during the drag (checked live off the `PointerEvent`
on every throttled move tick — see §5.3 for why "live" matters), instead of
transferring the whole delta to the one adjacent neighbor, distribute it
across **every other sibling under the same parent**, proportional to each
sibling's *current* size:

1. Compute the same pixel→flex-weight delta `Δ` the baseline case already
   computes for the dragged pair (unchanged math, `layoutResize.ts`'s
   existing pixel-to-size conversion).
2. The "near" side of the drag (the sibling whose edge the pointer is
   actually on top of) grows or shrinks by `Δ`, exactly as today.
3. The complementary `-Δ` is **not** applied entirely to the one immediate
   neighbor. Instead, spread it across **all other siblings in the parent's
   `children` array** (which now includes the previously-untouched ones,
   not just the immediate neighbor), each getting a share proportional to
   `sibling.size / sum(other siblings' sizes)` — a pane that currently holds
   more of the row gives up more; a pane that currently holds less gives up
   less. This is the same "grow one, everyone else absorbs proportionally"
   shape used by proportional/elastic-resize implementations in other flex
   splitter libraries, and it's the algorithm that best matches the survey's
   "peers move together" framing (§3) without requiring an equalize-to-one-
   value outcome (which is Excel's different, selection-driven model and
   doesn't fit a live drag).
4. Clamp every sibling to `MinNodeSizePx` as today. If the clamps leave less
   total room than `Δ` calls for, cap the dragged side's growth to whatever
   is actually redistributable — same conservation-safety principle the
   existing 2-node clamp already uses, just generalized to N-1 siblings
   instead of 1.
5. Emit one `ResizeNode` action with `resizeOperations` covering **every**
   affected sibling (the dragged pair's near side + every redistributed
   sibling) in a single staged/committed action — no reducer or action-shape
   change needed; this is a resize-math change in `layoutResize.ts` only.

### 5.3 Minimize-locked siblings within the group

Today's 2-node case is all-or-nothing: if either flanking node is
minimize-locked, the whole resize is rejected (`layoutResize.ts:76`,
reducer-side re-check `layoutTree.ts:436-443`). For the N-sibling group case,
blanket-rejecting the whole Ctrl-drag because *any one* of possibly several
siblings happens to be minimized would defeat the point of the feature (a
5-pane row with one minimized pane should still let the other 4 group-resize
together). **Recommended: exclude minimize-locked siblings from the
redistribution pool** (their size doesn't change, matching what "locked"
already means elsewhere) **and redistribute only across the unlocked
remainder**, rather than rejecting the whole action. This is a deliberate
behavior change from the existing all-or-nothing rule; approved as proposed
(§8, item 2).

### 5.4 Live modifier toggling mid-drag

Because the existing implementation recomputes the pixel delta fresh from
the drag's start position on every tick (not incrementally from the
previous tick — see §2), toggling the modifier key mid-drag should fall out
correctly for free: each tick just picks which formula (§5.1 vs §5.2) to
apply to the same drag-start baseline sizes + drag-start-to-now pixel delta.
No extra state should be needed to support "hold Ctrl partway through the
drag" or "release it partway through" — but this needs a live check (build +
manual drag test), not just a code read, since it's exactly the kind of
assumption that's cheap to get subtly wrong (e.g. if any intermediate state
*is* cached per-tick somewhere the code read didn't surface).

---

## 6. Modifier key: Shift (decided)

The user's original ask specified Ctrl. Two things surfaced during research
that argued for Shift instead, and the user reviewed and confirmed the
switch:

1. **§3's survey found Shift, not Ctrl, is the dominant convention** for
   this exact "peers move together" behavior in every tool that has it
   (Blender, Premiere, Kdenlive). Where Ctrl *does* appear on a resize-type
   drag in prior art (Blender), it means something else entirely
   (snap-to-increment) — so a user coming from any of those tools would
   likely reach for Shift first and be surprised Ctrl did this instead.
2. **Within this codebase specifically**, the only existing use of
   `ctrlKey`/`metaKey` on any drag interaction (`tab-reorder.ts:381`) uses it
   to mean "abort/ignore this drag" — a different gesture, but it means
   there's a very loose existing local association of Ctrl-during-drag with
   "opt out," not "opt into a bigger effect." Not a real conflict (different
   code path, different drag type, nothing breaks), just another point in
   Shift's favor: a first-time user's nearest in-app precedent for Ctrl+drag
   points the opposite direction from what group-resize needs.

**Decision: Shift.** `onResizeMove` reads `e.shiftKey` (not `e.ctrlKey`) to
branch into the §5.2 group-resize path. Whichever key is chosen still needs
some form of discoverability (handle tooltip text, or a first-use hint,
now also the help-pane entry from §7.1) since modifier-triggered drag
behaviors are inherently invisible until a user is told about them —
confirmed as a general weakness across the whole prior-art survey, not
something any of the surveyed tools solved particularly well either.

---

## 7. UI feedback while the modifier is held

- Cursor/handle affordance: the handle should visually indicate "group
  resize" mode is active while the modifier is held and the pointer is over
  a handle (before or during drag) — e.g. a highlight across all the
  siblings that will participate, not just the one handle. Exact visual
  treatment is a follow-up design decision, not specified here.
- `docs/specs/SPEC_PANE_RESIZE_DIMENSION_OVERLAY_2026_05_26.md`'s `WxH`
  badge already mounts on every pane during any splitter drag
  (`isSplitterDragging()`) — this should compose for free with a group
  resize (every participating pane already gets a live badge), no changes
  needed there.

### 7.1 Help pane entry (in scope for this pass)

Add an entry to the app's help/shortcuts pane documenting this behavior —
folded into this same implementation pass per the user's request, not
deferred as a follow-up. Exact wording/category/file location to be
confirmed against that pane's actual data structure (separate investigation,
this session); the entry should describe the gesture (Shift + drag a
splitter) and its effect (resizes every sibling pane sharing that splitter's
row/column, proportionally, instead of just the two panes flanking the
dragged border) using whatever phrasing convention the existing shortcut
list already uses for other drag-modifier or pane-layout entries.

---

## 8. Open questions

1. ~~Ctrl vs Shift~~ — **resolved, §6: Shift.**
2. ~~Minimize-locked exclusion vs. all-or-nothing~~ — **resolved: exclude
   and redistribute across the unlocked remainder, per §5.3, as proposed.**
3. Does group resize apply **only to the immediate parent's siblings**
   (Scope A, recommended) or is there real demand for the Scope B
   cross-branch behavior despite its cost/ambiguity (§4.2)? Recommend
   shipping Scope A and revisiting only if users explicitly ask for the
   cross-branch case.
4. Visual affordance for "which panes will move" before/during the drag
   (§7) — needs a design pass, not specified here.
5. Should there be a non-drag equivalent (a menu/shortcut action to
   "equalize this row/column," matching JetBrains' static toggle or tmux's
   `select-layout -E`) as a complementary, simpler feature? Out of scope for
   this spec but noted since §3 found it's the more common pattern overall
   (a dedicated equalize action, separate from any drag modifier).

---

## 9. Explicitly out of scope

- **Scope B** (cross-branch, pixel-aligned "everyone whose edge lines up"
  resize) — see §4.2. A future spec if there's real demand.
- Re-verifying `SPEC_PANE_RESIZE_DIMENSION_OVERLAY_2026_05_26.md`'s "every
  other pane in the same row/column also shifts proportionally" claim
  against live behavior (§2) — doesn't block or change this design, but is
  a loose end worth someone confirming separately.
- Keyboard-driven resize (arrow-key resize with a group modifier) — the
  existing codebase has no keyboard resize path today per the pane-resize
  overlay spec's own table (`docs/specs/SPEC_PANE_RESIZE_DIMENSION_OVERLAY_2026_05_26.md`
  §2.1, "Pane being resized via keyboard — Optional follow-up"); out of
  scope here too for the same reason.
- Persisting "this row prefers group-resize" as a per-layout setting —
  this is a momentary modifier-held behavior, not a mode toggle.

---

## 10. Implementation surface (for whoever picks this up)

All changes confined to `frontend/layout/lib/layoutResize.ts` (`onResizeMove`):
read the live modifier flag off the `PointerEvent`, branch between the
existing 2-node math (§5.1, unchanged) and the new N-sibling proportional
redistribution (§5.2-§5.3), and build the resulting `resizeOperations` array
with as many entries as siblings are affected. **No changes needed** to
`layoutTree.ts`'s `resizeNode` reducer, `types.ts`'s action/node shapes, or
`layoutGeometry.ts`'s handle enumeration — the existing `ResizeNodeOperation[]`
action already accepts an arbitrary-length op list and the reducer already
applies it all-or-nothing per entry.

## 11. Testing plan (once implemented)

- Unit tests for the new distribution math (pure function, same style as
  the existing resize math — no Win32/DOM dependency needed to test the
  proportional-split arithmetic itself: given N sibling sizes + a Δ, assert
  the redistributed sizes sum correctly and respect `MinNodeSizePx`).
- Live verification: a row of 4+ panes, modifier-held drag on an inner
  handle, confirm all siblings move proportionally and the total stays
  conserved; a row with one minimized sibling, confirm it's excluded per
  §5.3; toggling the modifier mid-drag (§5.4).

## 12. Sources

- Code read for this spec: `frontend/layout/lib/TileLayout.win32.tsx:492-532`,
  `frontend/layout/lib/layoutResize.ts:17-142`, `frontend/layout/lib/layoutGeometry.ts:414-447`,
  `frontend/layout/lib/layoutTree.ts:431-453`, `frontend/layout/lib/types.ts:72,179-272`,
  `frontend/app/tab/tab-reorder.ts:381`.
- `docs/specs/SPEC_PANE_RESIZE_DIMENSION_OVERLAY_2026_05_26.md` (existing,
  related — cross-referenced in §2 and §7).
- External prior-art sources listed inline in §3.
