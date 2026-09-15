# Can window-level tab switching be as fast as Chrome's? — why it isn't today, and the one untried fix

**Date:** 2026-09-15
**Status:** implemented — §4's `content-visibility:hidden` experiment shipped
(with two structural fixes it required: absolute-positioned, stacked tab
containers instead of flex siblings, and explicit `pointer-events` gating —
both consequences of no longer using `display:none`), plus a follow-up
`document.startViewTransition()` layer for the actual reveal cross-fade and a
forced-synchronous-layout fix for a reveal-gate/content-visibility timing gap
found during live testing. Not yet benchmarked against the recorded 500-600ms
baseline (§5) — deliberately deferred per the repo owner's own call to let
this settle before re-approaching with numbers.
**Scope:** window-level tabs (`frontend/app/workspace/workspace.tsx`, the
top bar's own tab strip — analogous to Chrome's own tab strip, NOT the
in-pane terminal/agent tab strip inside a single pane, which is a related
but separately-tracked surface — see §6).

---

## 0. The ask, restated precisely

> Tab switching should be as fast as Google Chrome's. We're loading the
> same sort of DOM content, yet AgentMux has a much larger delay. Chrome
> is near-instant. Research how Chrome does it, outline the technical
> blockers, and say whether benchmarking is needed.

Short answer up front: **this has already been investigated, with real
measurements, twice** —
`docs/specs/SPEC_AGENT_PANE_TAB_SWITCH_PERF_2026_05_27.md` (the perf
investigation) and `docs/specs/SPEC_TAB_SWITCH_DECOUPLE_SELECT_FROM_PAINT_2026_09_04.md`
(a UX-perception fix built on top of it, shipped). The May investigation
measured a **500-600ms browser-side layout+paint cost per tab switch**,
methodically eliminated every JS-level hypothesis, and landed on a single
root cause: **AgentMux's tabs are not the same kind of thing Chrome's
tabs are.** Chrome's near-instant switch isn't a trick this codebase
forgot — it's a consequence of an architecture AgentMux's tabs
structurally don't have. §1-§3 explain why; §4 is the one concrete,
narrowly-targeted experiment that fits *within* AgentMux's existing
architecture and hasn't been tried; §5 says what benchmarking that needs.

---

## 1. How Chrome actually does it

Chrome's own tab switch is fast for a specific, well-documented reason
that has nothing to do with the DOM content being simple:

- **Each Chrome tab is backed by its own renderer process**, with its own
  already-computed layout tree. Switching tabs is a **browser-process**
  operation, not a renderer operation.
- The compositor keeps a **cached, already-rasterized GPU texture** for
  each tab's last-painted frame — including backgrounded ones (until
  memory pressure evicts it). Switching tabs is "tell the GPU compositor
  to present *this* texture instead of *that* one" — an operation whose
  cost does not scale with the tab's DOM complexity, because the DOM was
  never touched at switch time. No layout, no paint, no script — it's a
  presentation-layer swap of work that was already finished, possibly
  seconds or minutes earlier.
- A backgrounded tab's renderer keeps running (rAF/timers throttled, not
  frozen), so its content stays live and its cached texture stays
  reasonably fresh, but nothing about *switching to it* re-does that
  work — the switch and the content staying fresh are decoupled.

This is a genuinely different mechanism from "efficient CSS" or "a fast
CPU." It's multi-process isolation + persistent GPU-level frame caching,
purpose-built for exactly this UX.

---

## 2. What AgentMux actually does today (verified against current code)

One CEF-hosted renderer, one DOM, one SolidJS reactive tree, for the
**entire window** — every tab's content lives in the same render tree at
once. `frontend/app/workspace/workspace.tsx:50-55`:

```tsx
<For each={allTabIds()}>
    {(tid) => (
        <div class="flex flex-row h-full w-full" style={{
            display: tid === tabId() ? "flex" : "none",
            ...
```

Inactive tabs are `display: none`, not unmounted (this is deliberate —
preserves xterm.js scrollback and avoids a cold-mount cycle, and is
itself the right call relative to *unmounting*; the problem is one layer
deeper, in *which* hiding mechanism was chosen). `display: none` removes
an element from the layout tree **entirely** — it isn't merely invisible,
it has no box, no cached layout, no painted texture, nothing. Flipping it
back to `flex` doesn't reveal cached work the way un-backgrounding a
Chrome tab does — it triggers the browser laying out and painting that
entire subtree **as if it had just been inserted into the page for the
first time**, because as far as the rendering engine is concerned, it
just was.

### 2.1 This was measured, precisely, not assumed

`SPEC_AGENT_PANE_TAB_SWITCH_PERF_2026_05_27.md`'s investigation log:

- Baseline: **500-600ms long-task per tab switch**, firing *after* the
  existing reveal gate lifts (the gate's own `tab-switch` timing mark
  covers only its own window and is "misleadingly small" — the real cost
  is the long-task right after).
- Instrumented every plausible JS-side cause and falsified all five:
  markdown parsing (already memoized, 0 invocations during switches),
  OverlayScrollbars init (cold-boot only), partition recompute (cached),
  `measureElement` (50 calls totaling 0.3ms — trivial), `DocumentRow`
  mount count (3-15, not hundreds — not a `<For>` reconcile storm).
- Conclusion, verbatim: *"All five JS-side hypotheses fell. The 500ms
  long-task is browser-side layout + paint on `display:none → block`,
  opaque to JS-level perf hooks."*

This is the single most load-bearing fact for this analysis: **the cost
is not in AgentMux's code.** It's the browser engine doing exactly what
`display: none → flex` asks of it — a full layout pass over a subtree
that was, a moment ago, not part of the render tree at all. A faster
virtualizer, a better memo, a leaner component tree would not touch this
number, because none of those were ever the bottleneck — confirmed by
elimination, not left unverified.

### 2.2 A second-order effect the same report documents

`docs/reports/REPORT_TAB_FLASH_SYSTEMIC_ANALYSIS_2026_08_31.md` §3.4
adds: because a hidden tab's container measures 0×0 while `display:none`,
becoming visible fires a burst of `ResizeObserver` callbacks (0×0 → real
size) that re-lay-out the **entire pane tree**, on top of the base
layout cost above. A `content-visibility:auto` experiment at the
*per-row* level made this worse (§Phase 1 B+C below), not better — an
important, already-paid-for lesson for anyone tempted to retry it at the
wrong granularity.

---

## 3. The technical blockers, ranked

1. **`display:none` destroys the one thing that would make a fast switch
   possible: a valid, already-computed layout + paint.** This is the
   root cause, confirmed by elimination in §2.1. It is also, unlike
   blocker 2, something fixable *within* AgentMux's current
   single-process architecture — see §4.
2. **No per-tab persistent compositor surface — the architectural gap
   under blocker 1.** Chrome's actual instant-ness comes from a
   multi-process, GPU-texture-cached model AgentMux's tabs don't have and
   structurally can't get without either (a) real multi-process
   isolation per tab (AgentMux already does this for the embedded
   *Browser* pane — real websites open in real CEF sub-browsers — but
   not for its own SPA-rendered tabs/panes, which are plain DOM in the
   one main renderer), or (b) a browser primitive that approximates the
   same "keep the last paint around cheaply" property without a second
   process. §4 is (b); real per-tab CEF sub-browsers would be (a) — see
   §4.3 for why that's disproportionate here.
3. **The reveal-gate/settle-detector pattern hides the cost, it doesn't
   remove it.** `SPEC_TAB_SWITCH_DECOUPLE_SELECT_FROM_PAINT_2026_09_04.md`
   (shipped) made the tab-bar *pill* update instantly regardless of the
   pane's own reveal cost — genuinely good UX, and a real, verified fix
   for "the highlight lags" — but it explicitly does not touch the
   pane's own 500-600ms cost, by its own design (§6 of that spec: "This
   fix only stops the cascade's cost from gating the [pill]... the
   destination pane is free to take however long it needs"). The pane
   itself still eats the full cost on every switch; the user just isn't
   staring at a frozen tab-bar while it happens anymore.
4. **Phases 2-3 of the original remediation plan were never built.** The
   May 27 spec's own sequencing named two structural mitigations —
   deferring expensive per-row hydration until in-viewport (Phase 2) and
   trusting the size estimator for the first reveal frame instead of a
   synchronous `measureElement` cascade (Phase 3) — as the next steps
   once JS-side causes were exhausted. Neither shipped (confirmed: no
   `LazyOnVisible` component exists in the tree, `AgentDocumentVirtualList.tsx`
   hasn't been touched since before that spec). These would reduce the
   *content-proportional* part of the layout cost but do not address
   blocker 1 itself — a `display:none → flex` flip over a *smaller*
   revealed subtree is still a full layout pass, just of less content.

---

## 4. The one thing that hasn't been tried, and fits the actual failure mode

The May 27 investigation tried `content-visibility: auto` at the
**per-row** level inside the virtualized document list, and it made
things measurably worse (700-1090ms vs. the 500-600ms baseline) — wrong
granularity, wrong CSS value, for a documented reason: `auto`'s
near-viewport skip-detection overhead dominates when most rows are
already near the viewport, and the 80px placeholder was far smaller than
real row heights, causing layout-shift cascades.

That result says nothing about a different application: **`content-visibility:
hidden`** (not `auto`) applied to the **whole per-tab container** in
`workspace.tsx`, replacing `display: none` outright.

- **Why this is the mechanism, not a guess.** `content-visibility:hidden`
  is a real Chromium primitive whose entire documented purpose is this
  exact scenario — the CSS spec's own reference use case is "tab-like
  UI." Per Chromium's own explainer: skips rendering while hidden (same
  cost class as `display:none` while inactive — no ongoing paint tax for
  backgrounded tabs), but **keeps a cached rendering state** so that
  un-hiding it lets the engine reuse that cache instead of laying out
  from zero. This is architecturally the closest single-process
  approximation of what Chrome's own compositor-texture caching does for
  its tabs — not identical (still one process, still one thread doing
  the reuse-and-validate work instead of a GPU texture swap), but the
  same *shape* of optimization: don't redo work that was already done.
- **Why `auto`'s failure doesn't predict `hidden`'s outcome.** `auto` is
  designed for *unbounded scrolling content* (skip off-screen rows *while
  still scrolling*, keep re-checking proximity) — a fundamentally
  different cost model from `hidden`, which is a binary, explicitly-
  driven on/off switch with no per-frame proximity polling. Applying
  `auto` inside a virtualizer that's already doing its own
  visible-range calculation was fighting the virtualizer, not
  complementing it. `hidden` at the tab-container boundary has no such
  conflict — nothing else in the tree is already deciding tab visibility
  on a per-frame basis.
- **Also fixes §2.2's second-order cost.** Unlike `display:none`,
  `content-visibility:hidden` elements are documented to keep real,
  cached layout dimensions rather than collapsing to 0×0 — closing the
  `ResizeObserver` 0×0→real-size burst the flash report names as a
  contributor, likely for free, as a side effect rather than a separate
  fix.
- **Compatibility with the existing reveal gate.** Nothing about this
  requires removing `tab-reveal.ts`'s gate — it can stay exactly as a
  safety net for whatever residual cascade remains, and its own
  before/after `[perf]` numbers become the acceptance signal (§5).
- **Risk, stated plainly.** This is a real, environment-specific
  behavior of one rendering engine (Chromium/CEF, which AgentMux always
  runs on — no cross-browser fallback risk here, a genuine advantage of
  being a CEF app rather than a general web page). It has not been tried
  at this granularity in this codebase. It could still fail for a reason
  specific to this DOM shape (e.g. if something inside a hidden tab
  forces a style/layout read every frame regardless of paint-visibility —
  worth auditing for `getBoundingClientRect`/`offsetHeight` reads that
  don't already gate on visibility, since those still return real
  numbers off cached layout and won't invalidate anything by themselves,
  but a *write* that follows one might).

---

## 5. Benchmarking — required, and already cheap to do

**Yes, benchmarking is required before this ships, in this codebase's own
established terms — not a new discipline, an existing one.** The May 27
spec is explicit and this project has apparently already burned a
verification gap once: *"Don't skip Phase 0... If the measurement
doesn't move, the PR doesn't ship."* The instrumentation already exists
and does not need to be built:

| Tap | Where | Threshold |
|---|---|---|
| Long-task observer | `frontend/perf/observers.ts` | ≥50ms, logged `[perf] long-task` |
| Tab-reveal gate timing | `tab-reveal.ts` | per-switch mark |
| IPC roundtrip | `frontend/perf/observers.ts` | >16ms |

**Repro, verbatim from the existing Phase 0 method** (reuse it exactly,
so the result is directly comparable to the recorded 500-600ms baseline
rather than a new, incomparable number): two tabs, one ~50-node agent
conversation and one ~200-node one, both populated, switch between them
5× each direction, grep `muxlog host` for `[perf] long-task` and
`tab-reveal` between switch timestamps.

**Acceptance bar**, already defined and still unmet by anything shipped
so far: hot tab-switch (2nd+ visit) long-task total **< 100ms**. Anything
that doesn't move the measured number toward that bar, per this
codebase's own rule, doesn't ship regardless of how principled the CSS
change looks on paper.

---

## 6. What this analysis does not cover

- **In-pane tab switching** (terminal/agent tabs *within* one pane) is a
  related but separately-tracked, separately-architected problem —
  `SPEC_PANE_TAB_SWITCH_CHROME_STABILITY_2026_09_07.md` (note: "Chrome"
  in that title means the pane's own *chrome* — header/tab-strip — not
  the browser; unrelated terminology collision, not a duplicate
  investigation). That surface's fix (hoist the tab strip out of the
  per-block remount boundary) has, per git history, landed already
  (`#3082`, `#3091`, `#3124`, `#3132`, `#3134`, `#3136`, `#3151`, `#3157`,
  `#3187`, `#3226`) — worth its own benchmarking pass if "Chrome speed"
  is also expected there, but out of scope for this document, which is
  about the window-level tab strip specifically.
- **Multi-process tab isolation** (§3 blocker 2's option (a) — real CEF
  sub-browsers per AgentMux tab, the literal architectural match for
  Chrome) is named for completeness, not recommended: AgentMux already
  pays that cost deliberately for the embedded Browser pane (real
  third-party websites, which need process isolation for security
  independent of perf), but applying it to the app's own SolidJS-rendered
  tabs would mean a CEF sub-browser instance per open tab — a large
  memory/startup-latency cost per tab, for a UI that (unlike arbitrary
  web content) is fully trusted and already fast to construct once laid
  out. §4's `content-visibility` experiment should be tried and measured
  first; only escalate to (a) if it provably can't close the gap.

---

## 7. References

- `docs/specs/SPEC_AGENT_PANE_TAB_SWITCH_PERF_2026_05_27.md` — the
  profiling investigation this analysis leans on most heavily
- `docs/specs/SPEC_TAB_SWITCH_DECOUPLE_SELECT_FROM_PAINT_2026_09_04.md` —
  shipped fix for the tab-*bar* perception, doesn't touch the pane cost
- `docs/reports/REPORT_TAB_FLASH_SYSTEMIC_ANALYSIS_2026_08_31.md` §3.4 —
  the 0×0-measurement/ResizeObserver second-order cost
- `docs/specs/SPEC_TAB_CONTENT_REVEAL_GATE.md` — the original reveal-gate
  mechanism both of the above build on
- `docs/specs/SPEC_PANE_TAB_SWITCH_CHROME_STABILITY_2026_09_07.md` — the
  related-but-distinct in-pane surface, see §6
- `frontend/app/workspace/workspace.tsx:50-83` — current `display:none`
  mechanism, verified against `main` as of this pass
- MDN / Chromium `content-visibility` explainers — the "tab-like UI"
  use case this analysis proposes matching against
