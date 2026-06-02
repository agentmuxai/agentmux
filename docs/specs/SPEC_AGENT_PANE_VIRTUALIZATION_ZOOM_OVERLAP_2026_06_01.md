# Agent-Pane Virtualization Overlap Under Zoom

**Status:** Root cause empirically verified (§3.0); Phase 1 implementing
**Date:** 2026-06-01
**Author:** AgentA
**Tracking:** open

---

## 1. Symptom

Scrolling through agent-pane conversation **history**, rows render on top of each
other — most visibly, "thinking" text overlays adjacent messages. The overlap
**worsens the further you scroll** (it accumulates), and it is **strongly
correlated with per-pane zoom**: at 100% zoom it is largely absent; zoomed
in/out it appears and grows.

---

## 2. Architecture recap (where the bug lives)

The agent document is rendered by a **hybrid virtualized + streaming-buffer**
list (`frontend/app/view/agent/virtualization/AgentDocumentVirtualList.tsx`):

- History (the bulk of the scrollback) is **virtualized** via TanStack Virtual:
  off-screen rows are not in the DOM; on-screen rows are absolutely positioned
  with `transform: translateY(start)`, where `start` is the cumulative sum of
  row sizes the virtualizer holds.
- Each row size is first **estimated** (`estimateSize`, line 164) from a
  per-kind heuristic, then **measured** to its real height after paint
  (`measureElement`, line 181; `ref={virtualizer.measureElement}`, line 418).

Per-pane zoom is applied as Chromium CSS **`zoom`** on the root of the whole
subtree: `agent-view.tsx:725` → `style={{ zoom: zoomFactor() }}` (clamped
0.5–2.0, from `term:zoom` meta, line 640). The scroll container and the entire
virtualizer live **inside** that zoomed element.

---

## 3. Root cause

### 3.0 Empirical verification (cef-146, live instance via CDP)

A probe simulated the virtualizer's exact pattern inside CSS `zoom` containers —
measure a 100-CSS-px row, then position the next row at `translateY(measured)`
and read the actual gap between them in device px:

| zoom | css height | `getBoundingClientRect().height` | `offsetHeight` | row2 `translateY` | actual gap (device px) |
|---|---|---|---|---|---|
| **0.5** | 100 | **50** (scaled) | **100** (neutral) | 50 | **−25 → OVERLAP** |
| 1.0 | 100 | 100 | 100 | 100 | 0 (flush) |
| 1.5 | 100 | 150 | 100 | 150 | +75 (gap) |
| 2.0 | 100 | 200 | 100 | 200 | +200 (gap) |

This is decisive:
- **`getBoundingClientRect().height` IS zoom-scaled** in cef-146 (`= css × zoom`).
  The double-count is real: positioning at `translateY(measured)` yields **−25px
  overlap at zoom 0.5** — the reported bug in miniature.
- **Direction:** overlap when zoomed **out** (<1), gaps when zoomed **in** (>1);
  exactly **0 at zoom 1.0** — why it's invisible at 100% and only appears with
  zoom.
- **`offsetHeight` is zoom-NEUTRAL** (100 at every zoom) — it returns the
  unzoomed CSS-px layout height, which matches the fixed-px estimates.

### 3.1 PRIMARY — estimate/measure unit mismatch under `zoom`

The two size sources the virtualizer combines are in **different units** whenever
zoom ≠ 1:

| Source | Code | Unit |
|---|---|---|
| `estimateSize` → `estimateNode` | `renderers.ts` (`estimateTextHeight`: `TEXT_LINE_HEIGHT_PX = 24`, `CHARS_PER_LINE = 80`, fixed-px caps) | **unzoomed CSS px** (zoom-independent constants) |
| `measureElement` | `AgentDocumentVirtualList.tsx:182` → `element.getBoundingClientRect().height` | **zoomed device px** (`getBoundingClientRect` includes the ancestor `zoom`) |

A row with intrinsic height `h` (CSS px) under `zoom: Z`:

- estimated size = `h` (≈, zoom-independent)
- measured size = `h × Z` (`getBoundingClientRect`)

The virtualizer sums measured sizes for `start` offsets, then applies
`translateY(start)` to a row **inside** the `zoom: Z` subtree — so the browser
scales that offset by `Z` a **second** time. Row *i+1* lands at `Z × Σ(h_j × Z)`
= `Z² · Σh_j` device px, when its natural flow position is `Z · Σh_j`. The extra
factor `Z` is the bug:

- **Z < 1 (zoomed out):** offsets shrink faster than heights → rows **overlap**.
- **Z > 1 (zoomed in):** offsets grow faster than heights → **gaps**.
- **Z = 1:** factor is 1 → no error (why it only shows up with zoom).

The error is **cumulative** (it's a running sum), so it grows as you scroll —
matching the symptom. As rows transition from *estimated* (unzoomed) to
*measured* (zoomed) during scroll, the coordinate space is internally
inconsistent, which is what tips tall **thinking** rows into their neighbors
first.

### 3.2 SECONDARY — no re-measure on collapse / expand / pin (any zoom)

`estimateSize` reads `props.documentState()` (line 166) and `estimateNode`
adjusts for `collapsedNodes` / `pinnedNodes`, but **nothing invalidates the
virtualizer's cached measurements when that state changes.** `measureElement`
runs once per row at first paint. So:

- Toggling a tool/agent-message collapse (`AgentDocumentView.tsx` `toggleCollapse`/`togglePin`) changes a row's real height, but the virtualizer keeps the stale measured size → neighbors overlap until a remount.
- Canceled-thinking blocks expand via a **local** signal inside `MarkdownBlock.tsx` (`expanded()`), changing height entirely outside the virtualizer's knowledge.

This is a real overlap source independent of zoom; it compounds 3.1.

### 3.3 TERTIARY — thinking-block height under-estimate

`estimateMarkdown` (`renderers.ts`) is a char-count heuristic; thinking content
with markdown/code/lists renders much taller than `chars/80 × 24px` predicts.
On its own this causes a one-time settle jump (acceptable once 3.1/3.2 are
fixed), not persistent overlap — but it makes the transient worse and is worth a
modest correction.

---

## 4. Fix

### 4.1 Normalize measurement into the estimator's unit (fixes 3.1) — **Phase 1**

Make `measureElement` return **unzoomed CSS px**, matching `estimateSize`, by
dividing out the live zoom factor:

```ts
// AgentDocumentVirtualList.tsx
measureElement: (element) => {
    const measured = element.getBoundingClientRect().height / props.zoomFactor();
    // …perf-probe records the normalized (CSS-px) value so the miss-rate HUD
    //   stays meaningful at non-100% zoom…
    return measured;
},
```

The virtualizer then works **entirely in zoom-independent CSS px** (estimates and
measurements agree), and the single `zoom: Z` on the ancestor scales the
`translateY` offsets correctly along with the content. No double-count.

**Plumbing:** `zoomFactor` lives in `agent-view.tsx` (line 640). Thread it down
as an accessor prop: `agent-view.tsx` → `AgentDocumentView` → `AgentDocumentVirtualList`
(new `zoomFactor?: Accessor<number>` prop, default `() => 1`). Prefer an explicit
prop over re-reading `--zoomfactor` / `term:zoom` so the virtual list has one
source of truth identical to the value driving the CSS `zoom`.

**No zoom-change re-measure needed.** Because the normalized measure is in CSS px
(zoom-invariant) and a row's *layout* height does not change with `zoom` (zoom is
purely visual), a value cached at one zoom (`GBCR/Z₁ = h`) is still correct at any
other zoom. The browser re-applies the new `zoom` to the whole subtree, so the
CSS-px offsets keep rendering correctly. This is verified by §3.0: the fix makes
positions correct at every zoom from a single measurement.

**Why not just `offsetHeight`?** §3.0 shows `offsetHeight` is zoom-neutral in
cef-146 and would also work as a one-liner with no plumbing. We chose
`GBCR / zoomFactor` anyway because (a) it keeps **fractional** precision (matching
TanStack's default `getBoundingClientRect` measurement; `offsetHeight` is integer
and would drift sub-px per row over a long history), and (b) it's **explicit** —
it doesn't silently depend on `offsetHeight` staying zoom-neutral across future
CEF upgrades. The relationship can't be exercised in jsdom (no real CSS `zoom`
or layout), so the **CDP probe of §3.0 is the verification of record** — re-run
it after a CEF/Chromium bump to confirm `GBCR` still scales with zoom.

### 4.2 Re-measure on collapse / expand / pin (fixes 3.2)

Invalidate measurements when the inputs `estimateNode` depends on change:

```ts
createEffect(() => {
    const s = props.documentState();
    s.collapsedNodes; s.pinnedNodes;   // subscribe to the sets the estimator reads
    virtualizer.measure();
});
```

For the **canceled-thinking** local-expand case (`MarkdownBlock.tsx`), the
height change is in-renderer and not in `documentState`. Two options:

- **(preferred)** route the expand state into `documentState` (e.g. a
  `expandedNodes` set) so the §4.2 effect already covers it and the state
  survives virtualization recycle; **or**
- attach a `ResizeObserver` per virtualized row that calls
  `virtualizer.measureElement(el)` on height change (a general guard that also
  covers async markdown/syntax-highlight settle — see 3.3).

The ResizeObserver approach is the most robust general fix (it makes *any*
post-mount height change self-correcting) and subsumes 3.3; the cost is one
observer per on-screen virtualized row (bounded by overscan + viewport).

### 4.3 Estimator correction (mitigates 3.3) — optional

Add a modest buffer for thinking/markdown blocks (e.g. account for the
`.thinking-block` chrome and a code-block likelihood), or — once 4.2's
ResizeObserver lands — accept the heuristic and let measurement settle it. Low
priority; do only if jank-on-scroll-in remains visible after 4.1/4.2.

---

## 5. Edge cases

- **Zoom mid-scroll:** §4.1's `createEffect` on `zoomFactor()` re-measures, so a
  zoom change while scrolled deep doesn't strand stale offsets.
- **`getBoundingClientRect` vs `offsetHeight`:** §3.0 measured `offsetHeight` as
  zoom-neutral in cef-146, so it *would* work — but we use `GBCR / zoomFactor`
  for fractional precision and to avoid silently depending on that neutrality
  across CEF upgrades (a regression test pins the relationship).
- **Streaming buffer:** unaffected — it's normal flow (no `translateY`), so the
  double-count doesn't apply there. Only the virtualized region needs the fix.
- **scrollMargin / anchor math:** `scrollMargin` reads `offsetTop` (line 175) and
  anchor capture reads `clientHeight`/`scrollTop`; verify these are consistent
  with the normalized unit after 4.1 (they're used for scroll position, not row
  sizing, so should be unaffected, but smoke at zoom 0.5 and 2.0).

---

## 6. Testing

- **Manual matrix:** a long history (>100 nodes incl. several multi-paragraph
  thinking blocks). For zoom ∈ {0.5, 0.75, 1.0, 1.5, 2.0}: scroll top→bottom,
  assert no overlap and no growing gaps; toggle collapse/expand on a tool and a
  thinking block, assert neighbors reflow without overlap; expand a canceled
  thought, assert no overlap.
- **Unit test** (`renderers.ts`): estimator returns zoom-independent values
  (already true; add a regression asserting the constants aren't accidentally
  scaled).
- **Perf probe:** the existing `agentPerfStore.recordEstimatorMeasurement`
  (line 189) compares estimate vs measure — after 4.1 the recorded `measured`
  must be in CSS px (divide before recording) so the miss-rate HUD stays
  meaningful at non-100% zoom.
- **Layout-shift observer** (`startAgentLayoutShiftObserver`): should show
  reduced CLS at non-100% zoom after the fix.

---

## 7. Phasing

- **Phase 1 (the overlap fix):** §4.1 zoom-normalized `measureElement` + plumb
  `zoomFactor` down (no zoom-change re-measure needed — normalization is
  zoom-invariant). This alone removes the zoom-correlated overlap (the reported
  bug). Empirically verified mechanism (§3.0).
- **Phase 2 (state-change correctness):** §4.2 re-measure on
  collapse/expand/pin + the ResizeObserver (or `expandedNodes` in
  `documentState`) for in-renderer height changes. Removes the
  any-zoom collapse/expand overlap.
- **Phase 3 (polish):** §4.3 estimator correction, only if scroll-in jank
  persists.

---

## 8. Files

| File | Change |
|---|---|
| `frontend/app/view/agent/virtualization/AgentDocumentVirtualList.tsx` | `measureElement` ÷ `zoomFactor`; `createEffect`s to re-measure on zoom + collapse/pin change; new `zoomFactor` prop; (Phase 2) per-row ResizeObserver |
| `frontend/app/view/agent/components/AgentDocumentView.tsx` | pass `zoomFactor` through; (Phase 2) optional `expandedNodes` in `documentState` |
| `frontend/app/view/agent/agent-view.tsx` | pass `zoomFactor` (line 640) into `AgentDocumentView` |
| `frontend/app/view/agent/components/MarkdownBlock.tsx` | (Phase 2) route canceled-thinking `expanded` into `documentState`, or rely on the row ResizeObserver |
| `frontend/app/view/agent/virtualization/renderers.ts` | (Phase 3, optional) thinking/markdown estimate buffer; perf-probe records CSS-px measured |

---

## 9. Risks

- **Re-measure storms:** a `createEffect` calling `virtualizer.measure()` on every
  `documentState` write could thrash during streaming. Scope the effect to the
  specific sets (`collapsedNodes`, `pinnedNodes`, `zoomFactor`) — NOT the whole
  document — so it fires only on the genuine size-affecting toggles, not on every
  token. (Cross-ref the input-responsiveness rules: no layout reads in the
  keystroke path; this effect is off the input path.)
- **ResizeObserver cost** (Phase 2): bounded to on-screen rows; disconnect on row
  unmount. Don't observe the whole document.
- **zoomFactor source drift:** the virtual list MUST use the exact same value
  that drives the CSS `zoom` (the `agent-view.tsx` memo), or the division won't
  cancel. Single source of truth via the prop; no independent re-derivation.
