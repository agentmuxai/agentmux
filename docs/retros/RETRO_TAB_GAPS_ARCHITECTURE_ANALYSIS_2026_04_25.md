# Tab-Bar Gap Drift — Architecture Analysis

**Date:** 2026-04-25
**Status:** Analysis report (research + ground truth)
**Owner:** AgentA
**Trigger:** A naïve one-line CSS fix to
            `frontend/app/tab/tab.scss` (removed
            active-tab `::after` bar-hiding) did NOT
            resolve the reported gap drift. The user
            asked for a full architecture analysis
            before further attempts.

---

## 1. Problem statement (from the user)

> "Tabs need to have constant width gaps between them, no
> matter what the size. But when I change the text and
> resize them, they develop varying gaps between each
> tab."

Specifically:
- Same gap distance between every adjacent pair, always.
- Stable across tab rename (text width changes).
- Stable across window resize (container width changes).
- Stable across active-tab transitions (which tab is
  highlighted).

## 2. Why the first fix didn't work

The hypothesis behind the failed fix: the active tab's
`::after` separator bar is hidden, AND its right
neighbour's bar is also hidden, so gaps adjacent to the
active tab look 1 px tighter than gaps elsewhere
(`tab.scss` lines 71-74, before the fix).

Removing those rules makes every gap show its bar — but
the user still sees drift, which means **the bar
visibility was a contributing factor at most, not the
root cause.** There is at least one more mechanism in
play.

## 3. Full rendering pipeline (ground truth)

```
WaveObj (workspace.tabids: string[])
  ↓ tabbar.tsx:159 — <For each={tabIds()}> emits
  ↓ DroppableTab per tabId
  ↓ droppable-tab.tsx:99-126 — wraps Tab in a flex item
  │   that injects inline `padding-left/right` from
  │   gapBefore/gapAfter (always 0 unless dragging)
  ↓ Tab → tab.tsx:235-273 — flex item with content-sized
  │   width, max 200 px / min 60 px
  ↓ Browser flex layout (CSS only)
```

Layout container chain:

```scss
.tab-bar {
    display: flex; flex-direction: row;
    flex: 1 1 auto; min-width: 0; overflow: hidden;
}

.tab-bar-scroll {
    display: flex; flex-direction: row;
    flex: 0 1 auto; min-width: 0;
    overflow-x: auto; overflow-y: hidden;
    gap: 1px;                              // <-- single source of "gap"
    position: relative;
}

.tab-drop-wrapper {
    position: relative; flex-shrink: 0;
    display: flex; height: 100%;
    transition: padding-left 100ms ease-out,
                padding-right 100ms ease-out,
                opacity 0.15s ease;
}

.tab {
    position: absolute;       // base — overridden below
    width: auto;
    min-width: 60px;
    max-width: 200px;
}

.tab-bar-scroll .tab {
    position: relative;       // override
    flex-shrink: 0;
    left: unset; transform: none;
}

.tab::after {
    position: absolute;
    left: 0; bottom: 4px;
    width: 1px; height: 18px;
    background: var(--border-color);
}

.tab:first-child::after { content: none; }
```

There are no JS-driven width writes (the `tabWidth` prop
is dead code — always `0`, formula `(0/3)*2 = 0`). The
gap-padding inline styles are zero unless the user is
dragging.

## 4. Hypotheses, ranked by likelihood

### H1: Sub-pixel rounding under `--zoomfactor` (HIGH likelihood)

The window is wrapped in `zoom: var(--zoomfactor)`. At
non-integer zoom (e.g. 1.1, 1.25), `gap: 1px` becomes
`1.1px` / `1.25px`. Different browsers round
fractional pixels with different algorithms — Math.ceil,
Math.floor, banker's rounding, or alternating per-element
rounding ("sub-pixel snapping"). The result: between two
adjacent tabs the gap may render as 1 px, the next pair
as 2 px, alternately or randomly across the row. This is
exactly the symptom the user describes, and exactly what
the cited research warns about
([John Resig — Sub-Pixel Problems in CSS], [BrowserStack —
Resolve sub-pixel rendering issues]).

The `::after` 1 px separator bars compound this: an
extra `1px` element rendered with sub-pixel anti-aliasing
right next to the gap. Either of the two 1 px values
may shift independently when text width changes, because
**text changes can move the tab's content-box origin by
fractional pixels**, changing the rounding bucket of both
the gap and the bar.

Why my fix didn't work: removing the bar-hiding made all
bars visible, but the bars themselves are still 1 px and
still subject to sub-pixel rounding. The drift is now
spread evenly across all gaps instead of being
concentrated near the active tab — but it's still drift.

### H2: Tab content sub-pixel layout drift on rename (MEDIUM)

`.name` uses `flex: 1 1 auto; min-width: 0; overflow:
hidden; text-overflow: ellipsis;`. Renaming a tab from
"Untitled1" (8 chars) to "feature/auth-fix" (16 chars)
changes the `.name` content width. The flex auto-sizing
may produce a fractional total width, and when that
fractional value crosses a `1.5px` boundary the browser
re-rounds. Adjacent tabs absorb that one-pixel shift
asymmetrically because their own content widths are
already at different fractional offsets.

This compounds H1 — both can fire at the same time.

### H3: `transition: padding-left/right` on the wrapper (LOW–MEDIUM)

`.tab-drop-wrapper` animates padding changes over 100 ms.
If two adjacent tabs both hit a transition at slightly
different times (e.g. one started its drag before the
other acknowledged the insertion point), they could be
mid-transition with non-equal padding values for a
window the user perceives as "the gap drifted." But this
should only fire during drag-drop, and the user reports
drift during rename/resize — so likely not the cause
here.

### H4: `position: absolute` → `relative` override leaving stale offsets (LOW)

`.tab-bar-scroll .tab` explicitly unsets `left` and
`transform` after flipping `position` to `relative`. As
long as `transform: none` is in the cascade after every
animation that might write a transform, this is safe.
The bounce animation (`scaleX` on `.tab-bouncing .tab`)
runs and clears in 300 ms, then has the class removed at
400 ms — should leave no residue.

## 5. Industry best practices (research)

### "Use gap, drop margins" — modern consensus

Every authoritative source from 2024-2026 recommends
flex `gap` over per-item `margin-right`. AgentMux already
does this — the bar uses `gap: 1px` exclusively, with no
per-tab margin-right. **Already correct.**
Sources: [MDN: gap CSS property], [CSS-Tricks: A Complete
Guide to CSS Flexbox], [HTML All The Things: CSS gap].

### "Avoid `space-between` for fixed-gap scenarios"

`space-between` distributes remainder space — gaps WILL
vary. Only use it when you want the remainder distributed.
For constant gaps, use `gap` + `flex: 0 0 auto` on items.
AgentMux does this correctly. **Already correct.**

### "Keep values integer; sub-pixel = drift" — the actual hit

[John Resig's classic post on sub-pixel rendering] is
explicit: any layout dimension that lands on a fractional
pixel will round per-browser-per-element, producing
visible 1 px shifts between elements that should be
identical. Recommended fixes:

1. **Round at the source.** Use the CSS `round()`
   function (Values & Units L4) where supported, or
   precompute integer values in JS.
2. **Use `transform: scale()` for zoom** instead of the
   `zoom` property — `transform: scale` is GPU-rendered
   and (according to BrowserStack research) snaps to
   integer pixels more reliably than the `zoom` property.
3. **Use `border-width` keywords** for 1 px borders
   (`thin`/`medium`/`thick`) — they survive zoom
   rounding more reliably than literal `1px`.

Sources: [John Resig — Sub-Pixel Problems], [BrowserStack
— Resolve sub-pixel rendering issues], [Medium / Kajabi
UX — CSS and Sub-Pixel Rendering].

### Chrome's own tab-strip — different approach

Chrome browser tabs **overlap each other by ~20 px**,
positioned with explicit z-order. Layout is computed by
`-layoutTabsWithAnimation:regenerateSubviews:` in
Chromium's source — not flex; not gap-based. The reason:
overlap eliminates the gap question entirely. There is
no inter-tab space, just z-ordered claim regions.
Sources: [Chromium — Tab Strip Design (Mac)], [Chromium
googlesource — tab_strip.h].

This is a viable architecture for AgentMux but a much
bigger rewrite than the gap fix. Filed as a future
direction, not a v1 path.

## 6. Recommended fix paths

Three options, ranked by effort and how thoroughly each
addresses the H1+H2 root cause:

### Option A — Quick, mostly addresses H1 (smallest)

- Make every gap an integer pixel multiple in pure flex
  layout: bump `gap: 1px` → `gap: 2px`, drop the
  `::after` bars (replace with the gap as the visual
  divider only). Use `--space-0-5` token (= 2 px in the
  default scale) so it survives theme changes.
- Keep tabs as content-width with `min/max-width`. Don't
  touch `--zoomfactor`.
- Live with potential drift at fractional zoom (1.1×,
  1.25×) — accepted because the gap is now 2 px and a
  1 px sub-pixel error reads as 50 % deviation rather
  than 100 % deviation, less perceptible.

**Pros:** ~5 lines of SCSS. Already partially shipped
(my failed PR removed bar-hiding but kept the bar
itself).

**Cons:** Doesn't actually solve sub-pixel drift; only
masks its perceptibility.

### Option B — Real fix for H1, swap zoom mechanism (medium)

- Replace `zoom: var(--zoomfactor)` on the tab strip
  (and possibly the rest of the chrome) with
  `transform: scale(var(--zoomfactor))` + the
  appropriate width-compensation so layout doesn't
  collapse.
- Switch `gap: 1px` to `gap: 2px` and drop the `::after`
  bar (Option A's CSS) — combined with integer zoom,
  every gap renders at the same pixel rounding bucket.
- May require touching `frontend/app/window/` zoom code
  (search for `--zoomfactor` usage) and verifying that
  pane content (terminals especially) doesn't break
  under `transform: scale`.

**Pros:** Eliminates the root cause; fix applies to
every CSS dimension in the chrome, not just tab gaps.

**Cons:** Touches more code. Risk of regressions in
terminal text rendering, which is sensitive to
sub-pixel positioning. Needs careful verification.

### Option C — Adopt overlap layout (largest, future)

- Re-architect the tab strip to use overlapping tabs
  with z-order, à la Chrome.
- Tabs stop having gaps entirely; the design language
  becomes "tabs claim space, the active one rises
  above."
- Requires a custom layout engine (JS-driven), not flex.

**Pros:** Eliminates gap-related bugs by removing gaps.
Matches mature browser UI.

**Cons:** Multi-day refactor. Drag-drop, hover, keyboard
focus all need redesign. Probably overkill unless we're
also addressing other tab-strip pain points.

## 7. Recommendation

Start with **Option A** (cheap, ~5 lines). If the user
still reports drift after Option A under non-integer
zoom, escalate to Option B. Option C is filed as a
future direction.

Concretely, Option A is:

```scss
// tabbar.scss
.tab-bar-scroll {
    gap: var(--space-0-5);   // was 1px — token-based, 2px
}

// tab.scss — drop the ::after bar entirely
.tab {
    // remove the &::after { ... } block
    // remove the &:first-child::after { content: none; }
    // remove the .active &+.tab::after rule (already done)
}
```

Plus:
- Verify by inspection: at 100% zoom, gap reads 2 px
  uniformly. At 90 % / 110 % / 125 %, sub-pixel rounding
  may produce a 1 px / 2 px / 3 px range — but the
  perceived **inconsistency** between adjacent gaps
  should be less because all gaps are subject to the
  same rounding (no bar to add a second 1 px element
  with independent rounding).

## 8. Open questions

1. Does AgentMux actually run at non-integer zoom by
   default? What's the value of `--zoomfactor` for a
   "normal" launch? If it's always 1.0, sub-pixel drift
   shouldn't fire and Option A's bar-removal alone may
   close the report.
2. Does the user see drift on a fresh launch (zoom = 1)
   or only after they've zoomed in/out via Ctrl+/Ctrl-?
3. Are tabs ever rendered into a context with a fractional
   `transform: scale(...)` ancestor (modal, magnify, etc)?
   That would also trigger sub-pixel drift independent
   of `--zoomfactor`.

These three questions, asked of the user during the next
implementation cycle, will tell us whether Option A is
sufficient or Option B is needed.

## 9. Sources

- [MDN — gap CSS property](https://developer.mozilla.org/en-US/docs/Web/CSS/gap)
- [MDN — column-gap CSS property](https://developer.mozilla.org/en-US/docs/Web/CSS/Reference/Properties/column-gap)
- [CSS-Tricks — A Complete Guide to CSS Flexbox](https://css-tricks.com/snippets/css/a-guide-to-flexbox/)
- [HTML All The Things — CSS gap: The Ultimate Guide](https://www.htmlallthethings.com/blog-posts/css-gap-the-ultimate-guide-to-spacing-flexbox-and-grid-items)
- [John Resig — Sub-Pixel Problems in CSS](https://johnresig.com/blog/sub-pixel-problems-in-css/)
- [BrowserStack — Resolve sub-pixel rendering issues](https://www.browserstack.com/docs/percy/common-issue/sub-pixel-rendering)
- [Medium / Kajabi UX — CSS and Sub-Pixel Rendering: The Case of the Clipped Border](https://medium.com/kajabi-ux/css-and-sub-pixel-rendering-the-case-of-the-clipped-border-4652c5a1b5ab)
- [Chen Hui Jing — Sub-pixel rendering and borders](https://chenhuijing.com/blog/about-subpixel-rendering-in-browsers/)
- [Chromium — Tab Strip Design (Mac)](https://www.chromium.org/developers/design-documents/tab-strip-mac/)
- [Chromium googlesource — tab_strip.h](https://chromium.googlesource.com/chromium/src/+/46c2139851df34f5d6dc2fa0f2a88aeab4cdb4f6/chrome/browser/ui/views/tabs/tab_strip.h)
- [W3C CSSWG drafts — gap with flex-wrap edge case](https://github.com/w3c/csswg-drafts/issues/5399)
- [Acko.net — CSS Sub-pixel Background Misalignments](https://acko.net/blog/css-sub-pixel-background-misalignments/)
