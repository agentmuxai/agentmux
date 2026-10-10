# Minimized pane content creeps into view under chrome zoom

**Date:** 2026-10-10
**Code examined:** `main` at `fd6eae06f`
**Status:** fixed alongside this report: fix A steps 1–3 and fix B (below). A step 4 (the three
sibling offsets) is deferred: under hoisted pane chrome the header isn't inside the block frame
at all, so those offsets need checking per case rather than a blanket change.

## Symptom

With at least one pane minimized, chrome zoom (Ctrl+Scroll over the title bar,
status bar or a pane header) makes the minimized pane's own content start to
show beneath its header chip, rising further as you keep zooming out. Zooming
in has the opposite failure: the header chip itself gets cut off at the
bottom.

## Root cause

Chrome zoom and pane minimization measure the pane header in two different
units, and nothing connects them.

### 1. Chrome zoom scales the pane header in CSS only

`frontend/app/store/zoom.ts:235` (`applyChromeZoomCSS`) sets one CSS variable,
`--zoomfactor`, on `<html>`. The pane header picks it up through the shared
mixin:

```scss
// frontend/app/mixins.scss:26-34  (block-frame-default-header-layout)
max-height: var(--header-height);   // 33px (theme.scss:147)
min-height: var(--header-height);
...
zoom: var(--zoomfactor, 1);
```

Because `zoom` applies after the 33px height, the header **renders**
`33 × zoomfactor` px tall: 16.5 px at the 50% minimum, 66 px at the 200%
maximum (`zoom-factor.ts`: `MIN_ZOOM = 0.5`, `MAX_ZOOM = 2.0`).

### 2. The minimize layout uses a fixed 33px constant

A minimized pane is a display mode. Its tile is still the full block frame
(header plus content), shrunk to a chip-sized rect by the layout engine and
clipped by `.tile-node` / `.tile-leaf` `overflow: clip`
(`frontend/layout/lib/tilelayout.scss:95,105`). The chip size comes from a
hard-coded constant:

```ts
// frontend/layout/lib/layoutMinimize.ts:9-10
/** Height of the block header in CSS pixels (matches --header-height in theme.scss). */
export const HeaderHeightPx = 33;
```

That constant is used for every minimized size:

| Where | What it sizes |
|---|---|
| `layoutGeometry.ts:65` `collapsedExtentPx` | `HeaderHeightPx + gap` per stacked chip (vertical), `MinimizedRowSlotWidthPx + gap` (horizontal) |
| `layoutGeometry.ts:90` `minimizedFixedPx` | the main-axis slot for a minimized child in a Column parent |
| `layoutGeometry.ts:112` `minimizedCrossAxisPx` → `:414` | the chip height for a minimized child in a Row parent |
| `layoutGeometry.ts:446,469` | the height of chips docked (slipped) onto a sibling |

The comment at `layoutGeometry.ts:84-88` states the assumption outright: *"the
header has a FIXED --header-height in block.scss — so the slot allocation must
be header + gap"*. Chrome zoom broke that assumption once the header mixin
gained `zoom: var(--zoomfactor)`.

### 3. Nothing re-runs layout on a chrome zoom change

`chromeZoomAtom` (`zoom.ts:52`) is read only by `chromeZoomIn`/`chromeZoomOut`
inside `zoom.ts`. The layout engine (`frontend/layout/`) never imports it.
Neither `updateTree()` nor `computeMainAxisAllocation` gets a zoom input,
so the chip slot stays at `33 + gap` whatever the header actually renders at.
This is the decoupling you suspected.

### Resulting geometry

The chip slot stays `33 + gap` px while the header renders `33 × z`:

| Chrome zoom | Rendered header | Effect inside the 33px chip |
|---|---|---|
| 50% | 16.5 px | **~16.5 px of the pane's content visible below the header** |
| 80% | 26.4 px | ~6.6 px of content visible |
| 100% | 33 px | correct |
| 150% | 49.5 px | bottom third of the header clipped |
| 200% | 66 px | header cut in half: its vertically centred icons, title and **Restore button** sit on the clip edge, half hidden |

The "creep" is the top of the content (a terminal's first rows, an agent's
transcript) moving up into the gap the shrinking header leaves. Each 5%
Ctrl+Scroll step exposes about 1.65 px more.

The 200% case is the more serious half: the Restore button, the user's way out
of the minimized state, is partly clipped.

## Same disease elsewhere (not the reported bug)

Other rules position things with the unzoomed `var(--header-height)` while the
header they sit beside is zoomed. At any chrome zoom other than 100% they are
offset by `33 × (z − 1)` px:

- `block.scss:280-282`, `.block-mask-inner` (the block-number overlay):
  `margin-top: var(--header-height)` (already marked `// TODO fix this magic`).
- `block.scss:129`, `.connstatus-overlay`: `top: calc(var(--header-height) + 6px)`.
- `blockstats.scss:6`: `top: calc(var(--header-height) + 4px)`.

`MinimizedRowSlotWidthPx = 180` (`layoutMinimize.ts:16`) has the same problem
along the horizontal axis. A zoomed header's contents are wider, so a row chip
truncates its title sooner at high zoom and wastes width at low zoom. This is
cosmetic only, since nothing leaks through.

## Fix options

### A. Make the layout zoom-aware (root-cause fix, recommended)

1. Replace the constant with a getter that returns the header's rendered height:
   `headerHeightPx() = HeaderHeightPx * chromeZoom` (and the same for the row
   chip width). Thread it into `collapsedExtentPx` / `minimizedFixedPx` /
   `minimizedCrossAxisPx` as a parameter defaulting to 33, so the existing pure
   tests in `layoutMinimize.test.ts` / `layoutModel.test.ts` keep passing
   unchanged. Then add cases at z = 0.5 and z = 2.0.
2. Have `LayoutModel` subscribe to `chromeZoomAtom` and call `updateTree()` on
   change, so chips resize on each Ctrl+Scroll step.
3. Keep the layout and CSS on one source: either the layout reads
   `chromeZoomAtom` (JS), or both read one derived CSS variable. The layout
   needs a number, so the JS signal is the natural owner. Don't let the layout
   measure the DOM (`getBoundingClientRect` of a header): that couples the
   layout pass to paint timing.
4. For the three CSS rules above, add
   `--header-height-rendered: calc(var(--header-height) * var(--zoomfactor, 1))`
   in `theme.scss` and use it wherever something sits *beside* (not inside) the
   zoomed header.

### B. Hide a minimized pane's content (defence in depth)

Have the block frame put a `block-minimized` class on itself when
`nodeModel.isMinimized()` is true, and hide everything but the header
(`visibility: hidden`, or `content-visibility: hidden`, on the content area).
Then content can never leak into a chip, whatever any future header-size
change does.

Use `visibility`/`content-visibility` rather than `display: none`. Terminal and
virtualised-list views react badly to a zero-size or `display: none` container,
as the collapse handling in `AgentDocumentVirtualList.tsx:1207-1213` already
shows. B alone does **not** fix the clipped header at zoom > 100%, so it
complements A rather than replacing it.

**Recommendation:** do A (steps 1–3 fix the reported bug and the clipped
Restore button; step 4 fixes the sibling offsets), and add B in the same PR as
a cheap guard.

## Repro (for verifying a fix)

1. Split a tab into two panes stacked vertically (a Column), one of them a
   terminal with some output.
2. Minimize the terminal pane.
3. Hover any pane header and Ctrl+Scroll down (zoom out) several steps. The
   terminal's top rows appear under its chip and rise with each step.
4. Ctrl+Scroll up past 100%. The chip's header is clipped and at 200% the
   Restore button is cut in half.
5. Repeat with the panes side by side (a Row). The minimized pane docks as a
   chip on top of its neighbour (the slip path, `layoutGeometry.ts:433+`) and
   shows the same leak and clipping.

## Side note

Chrome zoom isn't persisted: `loadZoom` (`zoom.ts:287`) is an empty stub and
`initChromeZoom` resets to 100% at startup. That's out of scope here, but it
means the bug always starts from a correct state after a restart. That hides
it until someone zooms.
