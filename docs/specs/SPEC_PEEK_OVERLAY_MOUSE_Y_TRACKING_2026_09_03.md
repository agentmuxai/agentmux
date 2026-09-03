# SPEC — Peek overlay: track mouse Y while pinned to the right

**Status:** proposed → implementing
**Date:** 2026-09-03
**Author:** agent3
**Related:**
- `frontend/app/view/agent/components/PeekOverlay.tsx` (positioning logic)
- `frontend/app/view/agent/components/hover-anchor.ts` (`findScrollContainerRect`, `PEEK_ENTER_DELAY_MS`)
- `frontend/app/view/agent/hooks/useNodePeek.ts` (shared hover timer/state)
- `frontend/app/view/agent/components/MarkdownBlock.tsx`, `ToolBlock.tsx`, `UserMessageBlock.tsx` (the three `align="end"` consumers)

---

## 1. Why

The user wants the hover-to-peek metadata panel (timestamp + token
estimate, shown on hover over a thinking/message/tool-call row) to track
the mouse vertically while staying pinned to the right — i.e. it should
still appear on hover and stay horizontally fixed the way it does today,
but its vertical position should follow the cursor's Y as the mouse moves
over the row, instead of being frozen at the row's top edge.

## 2. Already satisfied: time panel on tool calls

The second part of the request — extending the time panel to tool calls —
**is already implemented on `main`**, no work needed. Verified directly:
`ToolBlock.tsx`'s `peekTimeText` memo (constructed identically to
`MarkdownBlock.tsx`'s) renders `${formatExactTime(ts)} · ${formatTimeAgo(ts)}`
inside a `<PeekOverlay>` exactly like the thinking/message-block case,
landed under `SPEC_TRANSCRIPT_NODE_HOVER_PEEK_ALL_KINDS_2026_08_25`. All
three row kinds — thinking/message blocks (`MarkdownBlock.tsx`), tool
calls (`ToolBlock.tsx`), and user messages (`UserMessageBlock.tsx`) —
already share the same `useNodePeek()` + `<PeekOverlay>` infrastructure
and already show time. This spec's actual scope is the mouse-Y-tracking
behavior below, which benefits all three consumers uniformly since the
change is centralized in `PeekOverlay.tsx` itself.

## 3. Current positioning behavior

`PeekOverlay.tsx`'s `update()` (lines 107-134), for the default
`align="end"` mode (every consumer except `UserMessageBlock`'s full-body
"Session context" preview, which uses `align="stretch"`):

```ts
setFloatingStyle({
    position: "fixed",
    left: `${rect.right}px`,
    top: `${rect.top}px`,
    transform: "translateX(-100%)",
    "max-width": `${rect.width}px`,
    "max-height": `${cap}px`,
});
```

`rect` is the hovered row's own `getBoundingClientRect()`. `top` is always
the row's top edge — never the mouse position. Repositioning is driven by
`floating-ui`'s `autoUpdate(row, floatingEl, update)` (line 158), which
only fires on scroll/resize, not on `mousemove`. There is no mouse
coordinate anywhere in this file today.

## 4. Design

### 4.1 Scope: `align="end"` only

Only the `"end"` (right-pinned, shrink-wrapped metadata) mode gets mouse-Y
tracking. `align="stretch"` (`UserMessageBlock`'s large body preview) is
unaffected — it's a top-anchored, potentially tall/scrollable body
render, not a small metadata tooltip, and the request is specifically
about the metadata peek panel.

### 4.2 Tracking the mouse

Centralized in `PeekOverlay.tsx` (not in each of the three callers) so
`MarkdownBlock`, `ToolBlock`, and `UserMessageBlock` all get the behavior
for free, matching this component's existing role as the single shared
positioning implementation.

- A `mousemove` listener is attached directly to `props.rowEl()` — set up
  in an effect that (re)attaches whenever the row element changes,
  independent of `show`. Tracking starts as soon as the row exists, not
  gated on the peek panel actually being visible yet, so that when
  `show` flips true (after `useNodePeek`'s existing `PEEK_ENTER_DELAY_MS`
  enter delay), the very first render already has a real, current Y
  instead of momentarily flashing at `rect.top` before the first
  `mousemove` fires. (In practice a mouse hovering the row will almost
  always have already generated at least one `mousemove` well before the
  50ms delay elapses, but this avoids relying on that.)
- The handler stores the latest `event.clientY` in a plain `let`
  (component-local, not a signal — no need to trigger SolidJS reactivity
  for this; positioning is applied via direct style writes exactly like
  the existing `update()`/`floatingStyle` signal already does).
- Each `mousemove` calls `update()` when `align !== "stretch"`, coalesced
  via `requestAnimationFrame` (one pending rAF at a time — same pattern
  `registerFloating`'s existing rAF-gated setup already uses in this
  file) so fast mouse movement doesn't trigger a style write per raw
  event.
- Listener is removed in `onCleanup`, mirroring the existing
  `cleanupAutoUpdate` teardown right next to it.

### 4.3 Computing `top`

In `update()`, for `align === "end"`:

```ts
const container = findScrollContainerRect(row);
const minTop = container.top;
const maxTop = container.bottom - BOTTOM_MARGIN_PX;
const top = lastMouseY != null ? Math.min(Math.max(lastMouseY, minTop), maxTop) : rect.top;
const cap = Math.max(0, container.bottom - top - BOTTOM_MARGIN_PX);
setFloatingStyle({
    position: "fixed",
    left: `${rect.right}px`,
    top: `${top}px`,
    transform: "translateX(-100%)",
    "max-width": `${rect.width}px`,
    "max-height": `${cap}px`,
});
```

- `left`/`transform` (horizontal pinning) is **unchanged** — "pinned to
  the right" stays exactly as it behaves today.
- `top` follows `lastMouseY` once available, clamped to the scroll
  container's own vertical bounds (reusing `findScrollContainerRect`,
  already imported) rather than the row's bounds — a tall row means the
  mouse can be anywhere within it, and the container clamp is what
  already existed for the `cap` calculation, so this reuses the same
  reference frame rather than introducing a second one.
- Falls back to `rect.top` (today's behavior) if no mouse position is
  known yet (shouldn't normally happen per §4.2, but keeps the function
  total/safe).
- `max-height` (`cap`) is recomputed relative to the new `top`, same
  formula as before just parameterized — otherwise a panel positioned
  further down the row would keep the old, too-generous height budget
  and could overflow past the container's bottom edge.

### 4.4 Why not thread mouse position through props instead

An alternative would be having each caller track its own mouse position
and pass it as a new `PeekOverlay` prop (e.g. `mouseY: Accessor<number |
null>`). Rejected: three call sites would each need an `onMouseMove`
handler added alongside their existing `onMouseEnter`/`onMouseLeave`, and
that's exactly the kind of "shared logic re-implemented per caller" drift
`useNodePeek.ts`'s own doc comment already says this codebase moved away
from once (it explicitly cites three near-identical hand-rolled copies as
the original problem). Attaching directly to `props.rowEl()` inside
`PeekOverlay` keeps every caller's JSX unchanged.

## 5. Non-goals

- No change to `align="stretch"` positioning.
- No change to the enter/leave delay logic in `useNodePeek.ts`.
- No horizontal mouse tracking — "pinned to the right" is explicit in the
  request; only vertical position moves.
- No change to `autoUpdate`'s scroll/resize handling — still needed for
  the horizontal/width recompute and as a fallback when the mouse hasn't
  moved since a scroll.

## 6. Testing

- Manual: hover a thinking block, a tool-call row, and a user message
  (each with the metadata peek, not the "Session context" body preview)
  in a tall enough row (long thinking text / long tool output) — confirm
  the panel's vertical position tracks the cursor as it moves up/down
  within the row, while staying pinned to the same horizontal position
  throughout.
- Confirm the panel never escapes the scroll container's top/bottom
  bounds even when the mouse is near the row's own top/bottom edge.
- Confirm `UserMessageBlock`'s "Session context" (`align="stretch"`)
  preview is visually unchanged (still top-anchored, no mouse-follow).
