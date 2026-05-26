# SPEC: Pane resize dimension overlay (WxH badge)

**Date:** 2026-05-26
**Status:** Draft — ready to implement
**Author:** AgentA
**Tracking discussion:** TBD

---

## 1. Purpose

When any pane is being resized (the user is dragging a tile-layout splitter
or a magnified pane edge), show a small `WxH` badge in the bottom-left corner
of **every** pane in the layout. The badge gives live numeric feedback so the
user can dial a specific size — e.g. "make this terminal exactly 800×400" —
without external tooling, and so multiple panes' proportional changes are
visible at a glance during a drag.

The badge is read-only telemetry: it never accepts input, and never affects
hit-testing, focus, or pane content.

---

## 2. Behavior

### 2.1 Visibility

The badge mounts on **every visible pane** while `layoutModel.isSplitterDragging()`
is `true`. It unmounts when the signal flips back to `false` (drag end /
splitter release / ESC cancel).

**Why not `isResizing()`?** `LayoutModel.isResizing` is true for both
splitter drags AND container resizes (window resize, initial layout
observation). Gating on `isResizing` causes the badge to briefly flash
on every pane during a window resize, which the spec explicitly excludes.
`isSplitterDragging` is a separate memo that returns true *only* for
splitter-drag `pendingTreeAction`s. Codex P2 on PR #1057.

| State                                | Badge mounted? |
|--------------------------------------|----------------|
| Idle (no drag)                       | No             |
| Splitter drag in progress            | **Yes (all panes)** |
| Magnified-pane edge drag in progress | **Yes (all panes)** — when wired through `isSplitterDragging` |
| Window / container resize            | No (intentional — see above) |
| Pane move drag (pragmatic-dnd reorder) | No (sizes don't change) |
| Tab tear-off drag                    | No             |
| Pane being resized via keyboard      | Optional follow-up — out of scope for v1 |

The "every pane" rule is intentional: when you drag a splitter between A and
B, every other pane in the same row/column also shifts proportionally, and
users often want to see the cascade.

### 2.2 Content

Format: `<width>×<height>` using the multiplication sign (U+00D7), e.g.
`812×437`. Values are integer CSS pixels read from the pane's host element
(`clientWidth × clientHeight`).

No padding/border breakdown, no font-cell counts (terminals already render
their own status on resize via xterm's cursor row), no DPI-scaled values —
CSS pixels match what every other surface in the app reports.

### 2.3 Position

Bottom-left corner of the pane content area, **inside** the pane's clip
rect, with a small inset matching `--space-2` (8px):

```
┌─────────────────────────┐
│                         │
│   pane content          │
│                         │
│                         │
│ ┌─────┐                 │
│ │812×437│ ←── badge      │
│ └─────┘                 │
└─────────────────────────┘
```

Bottom-left was picked over bottom-right or center because:

- Terminal panes have their cursor + prompt at bottom-left; the badge sits
  flush above the same baseline and reads as a status indicator.
- Browser panes have a scrollbar on the right; bottom-left avoids the
  reserved-edge collision.
- Agent panes' "Send / Stop" buttons live bottom-right (per
  `SPEC_AGENT_PANE_BOTTOM_BUTTONS_2026_04_22.md`). Bottom-left is the
  only consistently-empty corner.

### 2.4 Style

Single SCSS rule on `.pane-size-badge`:

```scss
.pane-size-badge {
    position: absolute;
    left: var(--space-2);
    bottom: var(--space-2);
    z-index: 30;                        // above pane content, below modals (50+)
    padding: 2px var(--space-1-5);
    font-family: var(--termfontfamily, monospace);
    font-size: 11px;
    font-weight: 500;
    line-height: 1.4;
    color: var(--main-text-color);
    background: color-mix(in srgb, var(--main-bg-color) 88%, transparent);
    border: 1px solid var(--border-color);
    border-radius: 0;                   // hard-corners design (SPEC_HARD_CORNERS_2026_05_26)
    pointer-events: none;               // never blocks pane interactions
    user-select: none;
    backdrop-filter: blur(2px);         // legibility over busy content
    white-space: nowrap;
    // Match the design system's monospace status pills.
    letter-spacing: 0.02em;
}
```

No fade-in animation. The badge appears the instant `isResizing()` flips
true and disappears the instant it flips false — any easing would lag the
drag.

---

## 3. Implementation outline

### 3.1 New file

`frontend/app/block/pane-size-badge.tsx` — SolidJS component:

```tsx
import { Show, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { useLayoutModel } from "@/layout";

interface PaneSizeBadgeProps {
    /** Element to measure. Usually the pane's outer frame. */
    target: () => HTMLElement | undefined;
}

export const PaneSizeBadge = (props: PaneSizeBadgeProps): JSX.Element => {
    const layoutModel = useLayoutModel();
    const [size, setSize] = createSignal<{ w: number; h: number } | null>(null);

    let ro: ResizeObserver | null = null;

    onMount(() => {
        const el = props.target();
        if (!el) return;
        // Seed with the current rect so the first paint shows real numbers
        // (otherwise the badge briefly shows nothing on resize start).
        const r = el.getBoundingClientRect();
        setSize({ w: Math.round(r.width), h: Math.round(r.height) });
        ro = new ResizeObserver((entries) => {
            for (const entry of entries) {
                const cr = entry.contentRect;
                setSize({ w: Math.round(cr.width), h: Math.round(cr.height) });
            }
        });
        ro.observe(el);
    });

    onCleanup(() => { ro?.disconnect(); ro = null; });

    return (
        <Show when={layoutModel.isResizing() && size()}>
            <div class="pane-size-badge" aria-hidden="true">
                {size()!.w}×{size()!.h}
            </div>
        </Show>
    );
};
```

### 3.2 Mount point

Add to `frontend/app/block/blockframe.tsx` (every pane renders through
`<BlockFrame>` regardless of view type). Conditional on
`layoutModel.isResizing()` so the ResizeObserver isn't paying the cost
when idle.

The badge is **outside** the pane's scroll container but **inside** the
pane's `position: relative` outer frame, so `position: absolute` anchors
to the pane edge (not the viewport) and the badge clips with the pane on
overlap.

### 3.3 SCSS

Single file: `frontend/app/block/pane-size-badge.scss`, imported once
from `frontend/app/app.scss` (next to the other block-level rules).

### 3.4 No new state

`layoutModel.isResizing()` already exists (see
`frontend/layout/lib/types.ts:252` and `TileLayout.{win32,linux,darwin}.tsx`)
and is flipped by `onResizeStart` / `onResizeEnd` in
`frontend/layout/lib/layoutResize.ts`. No new reducer, no new event, no
new IPC.

For magnified-pane edge resize (RFC #404), that surface also has its own
"is dragging" signal — wire it into the same `<Show when={...}>` predicate
with an `||` if it's separate from `isResizing()`. Verify before merging
whether magnified-pane drag already toggles `isResizing()`; if not, plumb
through.

---

## 4. Edge cases

| Case                                | Behavior                                           |
|-------------------------------------|----------------------------------------------------|
| Pane height < ~50px                 | Badge still renders; the 11px text + 4px padding fits at ≥30px tall. Below that, the badge will overflow the pane edge — acceptable transient at extreme sizes. |
| Single-pane layout (no resize possible) | `isResizing()` never flips true → badge never mounts. No-op. |
| Pane that just unmounted mid-drag (e.g. close while dragging) | Solid's `<Show>` unmounts the badge with the parent. `onCleanup` disconnects the observer. |
| Modal open during resize            | Modals are `z-index ≥ 50`; badge is `z-index: 30`. Modal occludes — correct, since the user can't resize past the modal anyway. |
| Terminal pane with xterm at bottom-left | Badge sits over the terminal's cursor row for the duration of the drag. xterm doesn't lose focus and resumes input after release. Acceptable — the drag itself already obscures the cursor. |
| Reduced-motion preference           | No animations exist, so nothing to disable. |
| Pinned magnified pane               | Magnification doesn't change `isResizing()` — badge only mounts during the magnify-edge drag itself. |
| RTL layout                          | `left: var(--space-2)` flips to right under `[dir="rtl"]` via a logical-property pass — out of scope for v1; revisit when AgentMux adds an RTL theme. |

---

## 5. Out of scope (follow-ups)

- **Persistent display.** Some IDE-style users want WxH always visible. Add
  a settings toggle `layout:always-show-pane-size: bool` later if requested
  — not v1.
- **Aspect-ratio readout.** Bottom-left could also show "16:9" or "4:3"
  during the drag for video / mockup-aligned panes. Defer.
- **Snap markers.** "WxH" near a sibling pane's size could highlight when
  the user lands on a clean snap. Defer.
- **Keyboard resize.** If/when arrow-key splitter resize lands, reuse the
  same badge by also flipping `isResizing()` for the keystroke duration
  (debounced 300ms).

---

## 6. Acceptance criteria

1. Drag any splitter in a multi-pane tab → every pane in that tab shows a
   `WxH` badge at its bottom-left, updating live (≥30 fps tracking the
   mouse) until release.
2. Release the splitter → all badges unmount within one frame.
3. Open a modal mid-drag (impossible by user input, but verify via test) →
   modal overlays badges, badges still unmount on release.
4. Single-pane tab → no badge ever appears.
5. Numbers match `getBoundingClientRect()` of the pane frame (within ±1px
   for sub-pixel rounding).
6. Badge does not affect pane content scroll position, focus, or
   pointer-event hit-testing (test by clicking through the badge's bounding
   box while idle — should hit the underlying content).

---

## 7. References

- `frontend/layout/lib/layoutResize.ts` — `onResizeStart`,
  `onResizeMove`, `onResizeEnd`
- `frontend/layout/lib/types.ts:252` — `isResizing: Accessor<boolean>`
- `frontend/layout/lib/TileLayout.{win32,linux,darwin}.tsx` — consumers of
  `isResizing()` for the `animate` class gating
- `frontend/app/block/blockframe.tsx` — mount point for the badge
- `docs/specs/SPEC_HARD_CORNERS_2026_05_26.md` — radius token guidance
- `docs/specs/SPEC_AGENT_PANE_BOTTOM_BUTTONS_2026_04_22.md` — bottom-right
  reservation, justifying bottom-left choice
