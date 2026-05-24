# Progress — startup-injection hover anchor

**Date:** 2026-05-24
**Spec:** `docs/specs/SPEC_STARTUP_HOVER_EXPANSION_ANCHOR_2026_05_24.md`
**Branch:** `agenta/hover-anchor-direction`

---

## Research round (best practices)

Consulted:

- **[Floating UI flip middleware](https://floating-ui.com/docs/computeposition)** — when a popover collides with the viewport, flip to the opposite placement. Standard pattern. The library accepts 12 placement names (`top`, `bottom`, `top-start`, …) and re-computes on the fly. We don't need the library, but the flip *heuristic* is exactly our direction selector.
- **[Floating UI `useHover` + `safePolygon`](https://floating-ui.com/docs/usehover)** — when the cursor moves from trigger to popover, draw a dynamic polygon between them; popover stays open as long as cursor is inside the polygon. Handles diagonal traversals over gaps.
- **[Radix UI Popover hover discussion](https://github.com/radix-ui/primitives/issues/2051)** — Radix doesn't ship hover-popover by default; reference for the patterns the community has converged on (mostly: hover-intent timeout + safePolygon).
- **[MDN mouseleave](https://developer.mozilla.org/en-US/docs/Web/API/Element/mouseleave_event)** — `mouseleave` is fired when the pointer has exited the element AND ALL OF ITS DESCENDANTS. "Descendants" = DOM tree, not visual containment. Confirmed via [javascript.info](https://javascript.info/mousemove-mouseover-mouseout-mouseenter-mouseleave) and the [odetocode article](https://odetocode.com/blogs/scott/archive/2011/08/18/a-tale-of-backgrounds-absolutes-mouseleave-and-mouseenter.aspx).
- **[CSS Anchor Positioning](https://developer.mozilla.org/en-US/docs/Web/CSS/CSS_anchor_positioning)** — Chrome 125+ ships pure-CSS popover anchoring via `anchor()` and `position-area`. AgentMux's CEF build is recent enough to support it, but the cross-platform story is uncertain; the JS-based getBoundingClientRect path is more portable. Park anchor-positioning as a follow-up cleanup.
- **[TanStack Virtual layout strategy](https://tanstack.com/virtual/latest/docs/api/virtualizer)** — `shouldAdjustScrollPositionOnItemSizeChange` is the per-row hook that triggers when measured height changes. Overlays as `position: absolute` descendants of a row do NOT change the row's measured height, so the virtualizer doesn't see them — exactly what we want.

### Decision: simplest robust design from the research

The DOM-ancestry semantics of `mouseleave` collapse the problem from "we need a safePolygon" to "we need to ensure the body is a DOM child of the same wrapper the hover signal is on." That's free. We get the floating-UI flip pattern via a 12-line pure function. The pinned-vs-hovered distinction maps directly onto absolute-vs-flow positioning.

We do NOT need:

- `safePolygon` — there's no gap between summary and absolute body anchored at `top: 100%` / `bottom: 100%`.
- A library — the geometry is `getBoundingClientRect()` + `window.innerHeight`, two reads at expand-time.
- An animation library — the expansion is instant (no transitions, per `feedback_no_timers_or_delays`).
- A focus-trap or escape-key handler — pinned in-flow content is normal document content; hover-expanded gets closed by mouseleave naturally.

### Decision: Option B (pin → in-flow) confirmed

Picked Option B (pinned content drops into normal document flow). Rationale ties to research:

- TanStack handles per-row remeasure well; the cost of pin → flow is one remeasure event for the pinned row, then steady state.
- Mental model: hover = preview overlay (transient), pin = commit to keep this content in the conversation (persistent). Matches how Radix / Floating UI / VSCode peek-vs-pin all work.
- Overlay-when-pinned (Option A) leaves a floating panel that hovers over neighbors — confusing when scrolling and inconsistent with the rest of the agent pane.

---

## Implementation plan

Three commits on `agenta/hover-anchor-direction`:

1. ✅ Pure function `pickExpandDirection` in a new module with L1 unit tests.
2. ✅ Wire the function into `UserMessageBlock`. Add `displayMode` signal (`"hidden" | "overlay-above" | "overlay-below" | "flow"`). Add SCSS for the three positioning modes.
3. ✅ L2 component tests covering the new states.

---

## Progress log

(Updated as implementation proceeds.)

### Step 1 — pure function + L1 tests ✅

Created `frontend/app/view/agent/components/hover-anchor.ts` with `pickExpandDirection(rect, viewportH, bodyEstimate) → "above" | "below"`.

Decision tree:
1. Body fits below → "below".
2. Body doesn't fit below but fits above → "above" (the canonical "near bottom of viewport" case).
3. Body fits in neither → pick the larger side; below on tie.

L1 tests cover 10 cases (`hover-anchor.test.ts`): generic top, generic bottom, exact-middle tie, fits-below-but-above-bigger (must still pick below per step 1), fits-neither-pick-larger, fits-neither-tie, off-top, off-bottom, zero-viewport, zero-body.

All 10 pass.

### Step 2 — component wire-up + SCSS ✅

`UserMessageBlock` rewritten:

- New `expandDirection` signal, defaults to `"below"`. Re-evaluated inside the 150ms enter timer via `pickExpandDirection(rect, window.innerHeight, STARTUP_BODY_ESTIMATE_PX)` where `rect = summaryEl.getBoundingClientRect()`. Direction locked for the duration of the hover; mouseleave clears, next mouseenter re-evaluates.
- New `bodyMode()` derived: `"hidden" | "overlay" | "flow"`. Drives the body's class binding.
- Summary is now ALWAYS rendered for collapsible rows (was conditional on `!expanded()` before). Stable ARIA / keyboard surface. Visibility of the BODY is what toggles.
- Summary hint text reflects pin state: `"(hover to peek · click to pin)"` collapsed; `"(pinned · click ✕ to collapse)"` pinned.
- `aria-expanded` now mirrors `expanded()` (was just `pinned`).

SCSS additions in `_document-nodes.scss`:

- `.agent-user-message` gets `position: relative` so absolute children anchor here.
- Three new positioning classes on `.agent-user-message-content`:
  - `--flow` (default, no override) — normal flow.
  - `--overlay-below` — `position: absolute; top: 100%; left:0; right:0; z-index: 10; max-height: calc(100vh - 80px); overflow-y: auto;` + subtle background/border/shadow.
  - `--overlay-above` — same but `bottom: 100%`.

Why `mouseleave` doesn't fire when the cursor crosses summary→absolute body: per the MDN spec, `mouseleave` fires when the pointer has exited the element **and all of its descendants**. The absolute body is still a DOM child of `.agent-user-message`, so cursor traversal across the geometric boundary is a no-op for `mouseleave`. Confirmed via the research links in the section above.

### Step 3 — L2 tests + final sweep ✅

Added in `UserMessageBlock.test.tsx`:

- `describe("body positioning")` — 3 cases: hover-expanded → an overlay class; pinned → `--flow`; regular non-startup → `--flow`.
- `describe("aria-expanded")` — 3 cases: collapsed=false; pinned=true; hover-expanded=true.

Updated 2 prior tests that assumed the summary unmounts when expanded — it doesn't now (always-mounted for ARIA stability).

Final test counts:
- `hover-anchor.test.ts`: 10 / 10 pass.
- `UserMessageBlock.test.tsx`: 22 / 22 pass.
- 32 / 32 total in this PR's footprint.

Pre-existing failing tests (BlockErrorBoundary + AgentPicker mock-fixture) are unchanged from origin/main.

---

## Status

All three steps complete on `agenta/hover-anchor-direction`. Ready to commit + push + open PR.
