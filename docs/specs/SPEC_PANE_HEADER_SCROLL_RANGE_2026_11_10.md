# SPEC: The pane header's horizontal scroll covers tabs, +, grip and aux controls (minimize stays pinned)

**Date:** 2026-11-10
**Status:** implemented (PR #6915)
**Author:** Deke@narko
**Task:** #29324 (build #29325)
**Related:** #4808 (grips outside the scrolling strips, edge fades), #29020 (the minimize / "Merge into next pane" pair), `SPEC_PANE_TAB_STRIP_AGENT_TERMINAL_2026_07_20.md`

## 0. The ask

The operator, 2026-11-07, verbatim:

> "lets also refine the horizontal scroll over the pane tabs. for really thin panes, its still not enough room, lets include the +, grup, and all the aux controls (stop at the minimize) so that users can scroll through that entire range"

## 1. Today

`.block-frame-header` is one flex row. Left to right:

1. **The tab strip** (`PaneTabStrip.tsx`): the tabs and the trailing `+`. Only the pills inside `.pane-tab-strip-inner` scroll (`overflow-x: auto`), with edge fades from `createScrollEdges` while they overflow. This is the *only* horizontally scrollable part of the header.
2. **The connection chip** (remote terminals), the pane's inline header elements (`block-frame-textelems-wrapper`: title-ish text, inputs, icon buttons the view declares), and the error icon.
3. **The grip** (`PaneGrip`): the explicit drag handle, deliberately *outside* the scrolling strip since #4808.
4. **The end icons** (`.block-frame-end-icons` / `EndIcons`): the view's auxiliary controls and the frame's buttons.
5. **The window controls**: minimize, and the magnify/"Merge into next pane" sibling, at the right edge.

At ~300 px the tabs shrink (the #6868 fix) but the fixed parts 2–5 keep their room. On a thin pane the strip can be left with a pill and a sliver: the `+`, the grip and the aux controls are visible but the tabs are not, and the wheel over anything except the pills does nothing. There is no one range a person can scroll through.

## 2. The new range

**One horizontal scroll range** from the first tab to the last auxiliary control, holding, in order:

- every tab pill,
- the new-tab `+`,
- the connection chip and the pane's inline header elements,
- the grip,
- the end icons (the view's aux controls).

**The pinned cluster** — minimize and its magnify/"Merge into next pane" sibling (#29020) — is **not** in the range. It stays at the right edge at every width, always visible, never scrolled away, and the range's right boundary is that cluster's left edge. Scrolling stops there: the cluster cannot be hidden by a scroll, and the scroll cannot be dragged behind it.

When the pane is minimized to its header (or is a minimized chip), the same rule holds: the minimize/restore control and its pinned sibling are still outside the range; the range keeps the tabs, `+`, grip and aux controls, scrollable if they overflow.

## 3. Behaviour

- **Wheel / trackpad:** a horizontal gesture anywhere over the header row scrolls the range — over a tab, the `+`, the grip, an aux control, or the strip's empty space. A vertical wheel over the header does not scroll it (the header is not a vertical scroller) and keeps scrolling the page/pane body as it does today.
- **Overflow affordance:** the existing edge fades move from the pill strip to the range: a fade on the left while scrolled, on the right while there is more, each only over scrollable content and never over the pinned cluster. A tab selected by keyboard scrolls into view; the `+` and every aux control do the same when focused.
- **The active tab comes back (#46761):** the tab strip's reveal scrolls the range, as it scrolled the strip before (`SPEC_PANE_TAB_STRIP_REVEAL_SELECTED_2026_10_15.md` §3 to §5). The active tab is brought into view by the least it takes, clear of the range's fade, when it is selected, when the pane narrows, when the pane takes the focus, and on a click or a key in the pane's body. The range is left where it is after the person's own scroll of it (until they select a tab or work in the pane again), by a press on one of its own controls and the focus that press gives the pane, and while one of its controls holds the keyboard focus. The last tab comes in beside the fade; the range is not scrolled to its end for it.
- **No wrap, no squeeze to nothing:** the pills keep their current minimum, the controls keep their intrinsic sizes, and the range scrolls instead of compressing them further.
- **The range does not steal gestures:** a pointer press that begins a tab drag still drags the tab (its hit-testing accounts for `scrollLeft`); a drag on the grip still drags the whole pane; a drag on the header's empty space still drags the pane. Scrolling never starts a tab drag, a pane drag, or a drop.

## 4. Risks and how the design answers them

- **Grip inside a scroller.** Its pointerdown must remain the pane drag's handle; the scroll container must not treat a press on it as a scroll-start, and its `touch-action` must not let a horizontal pan cancel the drag by accident. The grip keeps its own hit area; only the wheel scrolls the range under it.
- **Tab drag coordinates.** The drag preview and drop-slot math currently assume a strip at `scrollLeft = 0` or read the pill's own box; they must use the scrolled position when the range is scrolled.
- **Aux controls become scrollable.** A control the person expects to always see (a mute, a connection chip) can sit off the right edge on a thin pane. Accepted by the ask ("scroll through that entire range"); the fades and keyboard scroll-into-view are the mitigation. The pinned cluster stays the one always-visible set.
- **#29020's merge pair** must stay adjacent to minimize; it moves with the pinned cluster, not into the range.

## 5. Implementation sketch

- Wrap parts 1–4 in one `.block-frame-header-scroll` (flex, `overflow-x: auto`, `min-width: 0`, `scrollbar-width: none`), and leave the window controls as the sibling after it.
- `PaneTabStrip` stops scrolling itself: `.pane-tab-strip-inner` becomes a plain flex row; `createScrollEdges` is driven by the new container instead of the strip.
- The wheel handler is attached to the header row and forwards `deltaX` (and `shift`+`deltaY`) to the range; no new listener per control.
- Tests: markup test that the range contains the `+`, grip and aux controls and excludes the minimize cluster; a wheel test that a horizontal delta over the grip or an aux control moves the range; a drag test that the grip and tabs still drag; the pinned cluster's position does not change while the range scrolls.
- Visual change: macOS (Clare/Masty) and Linux (Maricon) look checks at ~300 px, ~500 px and full width, dark and light, with screenshots.

## 6. Decided (the operator left these to the build)

- **The `+` scrolls with the tabs.** It belongs to them; it is the last item of the tab group inside the range, not pinned beside the window controls.
- **The fades stop short of the pinned cluster.** The right fade is drawn only over scrollable content and never touches the pinned icons, so a pinned control never looks dimmed or unavailable.
