# Spec: Scroll Chaining for Nested Tool-Preview Regions

**Date:** 2026-07-03
**Status:** proposed
**Related:** `docs/specs/PLAN_TOOL_BLOCK_SCROLL_DRIVEN_COLLAPSE_2026_06_16.md` (owns
`AgentDocumentVirtualList.handleScroll` — this spec must not fight its stick-to-bottom
gating), `frontend/app/view/agent/components/ToolBlock.tsx` (existing Ctrl+wheel zoom
handler on the same element tree — must keep working unmodified).

---

## Problem

The agent pane (`.agent-document`, rendered by `AgentDocumentVirtualList.tsx`) is one big
scrollable region containing a stream of messages and tool blocks. Several tool blocks
render their own internal scrollable preview — diff viewers, read/search/fetch results,
bash output, compact JSON (`_document-nodes.scss`, `ToolOverlayLog.tsx`) — each with a
capped `max-height` and its own `overflow-y: auto`.

Today, hovering over any of these inner previews and scrolling **captures all scroll
input**: once the preview reaches its own top or bottom, further scroll ticks either do
nothing (feels broken/stuck) or the browser's default scroll-chaining kicks in and jerks
the *outer* pane in a way that doesn't track the user's intent smoothly. The desired
behavior — standard on the modern web (chat apps, code review tools, IDE panels) — is:

> The element under the cursor consumes scroll input as long as it has room left to
> scroll in that direction. The instant it's exhausted (hits its top/bottom boundary),
> the *next* scroll tick passes straight through to the next scrollable ancestor, with no
> dead zone and no visible jump.

## Current state (audit)

| Element | File | Has `overscroll-behavior`? | Has scroll-boundary JS? |
|---|---|---|---|
| Outer pane `.agent-document` | `frontend/app/view/agent/styles/_document.scss:8-32` | **No** | `onScroll` exists (`AgentDocumentVirtualList.tsx:696`) but it's for stick-to-bottom/pagination/tool-collapse, not chaining |
| Primary tool scroller `.agent-tool-overlay-log` | `frontend/app/view/agent/styles/_tool-overlay-portal.scss:34-39` | **Yes** — `overscroll-behavior: contain` already set | `onScroll` for stick-to-bottom only (`ToolOverlayLog.tsx:99-103`) |
| ~10 secondary nested boxes (diff/read/search/compact-json/fetch previews) | `frontend/app/view/agent/styles/_document-nodes.scss` (lines ~376, 635, 919, 1118, 1172, 1229, 1243, 1285, ~1475, 1544) | **No** | No |
| `.agent-recent-sessions` (unrelated picker) | `_recent-sessions.scss:65-68` | Yes, `contain` | — |

So the primary tool-preview scroller already opted into `contain`, but every *secondary*
nested box inside it (the ones rendering the actual diff/log/JSON content) did not, and
there is no JS-level boundary handling anywhere. `ToolBlock.tsx:89-113` has an existing
`onWheel` handler, but it's scoped to `e.ctrlKey` (font-size zoom) and explicitly falls
through for plain scroll — it's not in the way of this fix and shouldn't be touched.

## Research: what actually solves this

**CSS `overscroll-behavior` is the mechanism, and it is standard-track.** `auto` (the
default) explicitly permits scroll chaining to the parent once a container's own scroll
range is exhausted — that's the "jerks the outer pane" behavior. `contain` stops the
chain at that element's boundary (scroll does not propagate to ancestors) while still
allowing the element's own local overscroll affordance internally; `none` goes further
and suppresses that local affordance too. This is confirmed directly from MDN and the
Chrome DevRel `overscroll-behavior` writeup, and matches the semantics this fix needs —
`contain`, not `none`: we want the swallow to stop, but no reason to disable whatever
native "can't scroll further" feedback the platform shows.

**Cross-browser gap is moot here.** Historically (write-ups from 2019–2021) Safari lagged
on `overscroll-behavior` support, which is why some public-web implementations still carry
a JS wheel-delta fallback. That doesn't apply to this app: AgentMux embeds a single pinned
Chromium via CEF (`agentmux-cef/Cargo.toml:41` pins `cef = "148"`), which has supported
`overscroll-behavior: contain` since Chromium 63 (2017). There is exactly one rendering
engine in play, and it's new. **A pure-CSS fix is sufficient and should be the whole
solution** — no wheel-event JS needed to make the core ask work.

**Framework correction (important — don't blindly port React advice):** most public
write-ups on "wheel handler + scroll boundary" describe React, where `onWheel` is
passive-by-default since React 17 (so `preventDefault()` silently no-ops unless you drop
to a manually-attached, `{ passive: false }` native listener or `onWheelCapture`) and
`stopPropagation()` on a synthetic event doesn't stop the browser's native scroll response
— see `facebook/react#5845`. **AgentMux's frontend is SolidJS (`package.json:114`,
`solid-js": "^1.9.11"`), not React.** Solid does not apply React's passive-by-default
special-casing to `onWheel`, and `wheel`/`scroll` aren't in Solid's small
globally-delegated event set — Solid attaches these directly on the element. None of this
should matter for the CSS-only fix below, but it means *if* a JS fallback is ever added,
the React gotchas researched here (capture-phase workarounds, conditional
`preventDefault()` based on scroll position, the classic "check `scrollTop`/`scrollHeight`
before deciding whether to swallow the event" pattern from `facebook/react#5845`) should be
re-verified against Solid's actual event binding before reuse, not assumed to transfer.

**DOM-level facts that stay true regardless of CSS/JS approach** (from MDN, directly
relevant if any JS boundary-check is ever needed):
- `WheelEvent.deltaMode` is one of `DOM_DELTA_PIXEL` / `DOM_DELTA_LINE` / `DOM_DELTA_PAGE`;
  in line mode the pixel-equivalent of "one line" is browser-dependent, so raw `deltaY`
  magnitude is not a portable "how many pixels" number.
- MDN explicitly recommends against inferring scroll direction/amount from wheel `delta*`
  — a `wheel` event doesn't always cause a scroll, and the delta doesn't necessarily match
  the content's actual scroll direction. Prefer reading `scrollTop` before/after, or a
  `scroll` listener, over trusting `deltaY` sign alone.
- These are moot for the CSS-only approach (no `deltaY` math involved) but matter if this
  spec's Phase 2 (below) is ever implemented.

## Proposed change

### Phase 1 — CSS-only, closes the actual gap (do this now)

Add `overscroll-behavior: contain` (or `overscroll-behavior-y: contain` where the box is
vertical-only) to every currently-uncontained nested scroll box in
`frontend/app/view/agent/styles/_document-nodes.scss`:

- `.agent-tool-agent-result` (~line 376)
- `.agent-tool-read-content` (~line 635)
- the generic result/search-result boxes (~lines 919, 1118, 1172, 1229, 1544)
- `.agent-tool-compact-json` (~line 1285)
- `.agent-tool-record-table` (~line 1243, `overflow: auto` → add `overscroll-behavior: contain`)
- `.agent-fetch-content` (~line 1475)

Also add it to the **outer pane**, `.agent-document`
(`frontend/app/view/agent/styles/_document.scss:8-32`) and its wrapper
`.pane-region--stream` (`PaneRegions.scss:18-25`). This isn't strictly required for the
inner-preview-to-outer-pane handoff (the *inner* box's `contain` is what stops the chain
from leaving the inner box), but it stops the chain from leaving `.agent-document` itself
and reaching the OS/window level (e.g. macOS's swipe-navigation-via-scroll gesture, or a
parent scroll region outside the pane), which is the same class of jarring behavior one
level up.

This is a same-shaped, mechanical change repeated ~12 times — no new component, no new
state, no risk to `AgentDocumentVirtualList`'s existing stick-to-bottom/pagination/
tool-collapse scroll logic, because `overscroll-behavior: contain` on the *inner* box means
the outer pane's `onScroll` handler simply never fires while the user is scrolling inside a
still-scrollable inner box — which is exactly the desired decoupling, achieved for free.

### Phase 2 — only if Phase 1 proves insufficient in practice

If manual testing (see below) turns up a real gap — e.g. a preview nested two levels deep
(a scroll box inside `ToolOverlayLog`'s own scroll box) where chaining behaves oddly, or a
touch/trackpad edge case CEF's Chromium handles differently than expected — fall back to a
small Solid-native boundary-check wheel handler as a supplement (not replacement) for the
CSS:

```ts
// Sketch only — verify Solid's addEventListener/passive semantics before landing.
function handleWheelBoundary(el: HTMLElement, e: WheelEvent) {
  const atTop = el.scrollTop <= 0;
  const atBottom = el.scrollTop + el.clientHeight >= el.scrollHeight - 1; // fractional-px slack
  const scrollingUp = e.deltaY < 0;
  const scrollingDown = e.deltaY > 0;
  if ((atTop && scrollingUp) || (atBottom && scrollingDown)) {
    return; // let it bubble to the ancestor — do NOT preventDefault/stopPropagation
  }
  // else: let the browser's native scroll handle it (still no preventDefault needed —
  // overscroll-behavior: contain already does the containment; this handler would only
  // exist to cover a CSS gap, not to reimplement the whole mechanism in JS).
}
```

The `- 1` fractional-pixel slack guards against sub-pixel layout rounding leaving
`scrollTop + clientHeight` a hair short of `scrollHeight` at a true boundary — a
well-documented gotcha with fractional zoom/DPI. Do not attempt this phase speculatively;
it adds a maintenance surface (interacts with `ToolBlock.tsx`'s existing Ctrl+wheel
handler, and with Solid's event binding model) that Phase 1 is expected to make
unnecessary given CEF's fixed, modern Chromium.

## Edits (Phase 1)

1. `frontend/app/view/agent/styles/_document-nodes.scss` — add
   `overscroll-behavior: contain;` to the ~10 nested scroll-box selectors listed above.
2. `frontend/app/view/agent/styles/_document.scss` — add to `.agent-document`.
3. `frontend/app/view/agent/components/PaneRegions.scss` — **skipped**: `.pane-region--stream`
   is `overflow: hidden`, not an actual scroll container, so the property would be an inert
   no-op there (see Risks table below).
4. No `.tsx`/`.ts` changes — Phase 1 is pure SCSS.

## Risks & edge cases

| Risk | Mitigation |
|---|---|
| `overscroll-behavior` set on an element that isn't a real scroll container has no effect (silently) | Verified against the actual `overflow-y`/`overflow: auto` + `max-height` selectors before applying it to each target — this is exactly why `.pane-region--stream` (`overflow: hidden`, not a scroll container) was excluded rather than "fixed" alongside the others. |
| Interaction with `ToolBlock.tsx`'s Ctrl+wheel zoom handler | That handler only branches on `e.ctrlKey`; unrelated to normal scroll and untouched by this change — confirm with a manual Ctrl+scroll test post-change regardless. |
| Interaction with `AgentDocumentVirtualList`'s stick-to-bottom / near-top pagination / scroll-driven tool-collapse logic (`PLAN_TOOL_BLOCK_SCROLL_DRIVEN_COLLAPSE_2026_06_16.md`) | `contain` on inner boxes prevents the outer `handleScroll` from firing during inner-box scrolling — verify live that pagination/collapse still trigger correctly when scrolling the *outer* pane itself (unaffected path) and don't fire spuriously while scrolling *inside* a tool preview (should now be impossible, which is the fix). |
| Nested-within-nested scroll (a scroll box inside `.agent-tool-overlay-log`, which is itself inside `.agent-document`) | Each level's own `contain` stops the chain at that level independently — no compounding needed; three independently-contained levels behave correctly. |

## Verification

- `npx tsc -p tsconfig.json --noEmit` (no `.ts`/`.tsx` touched, should be a no-op check).
- Manual, in `task dev`: open a PR-review-style tool call with a long diff/read/search
  result, scroll the mouse wheel/trackpad while hovering inside the preview to its
  bottom boundary, keep scrolling in the same direction — outer pane should pick up
  smoothly with no dead zone and no visible jump. Repeat scrolling up from the top
  boundary. Repeat with a tool preview near the very top/bottom of the visible pane
  (where the outer pane itself is close to its own scroll limits) to confirm no
  double-bounce.
- Confirm Ctrl+scroll zoom on a tool preview still works unchanged.
- Confirm stick-to-bottom (new streaming output auto-scrolls) and scroll-driven
  tool-collapse (per the companion spec) still behave correctly when scrolling the outer
  pane directly.
