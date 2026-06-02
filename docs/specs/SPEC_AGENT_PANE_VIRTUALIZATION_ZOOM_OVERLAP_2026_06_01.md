# Agent-Pane Virtualization Overlap Under Zoom

**Status:** Phase 1 (zoom) MERGED #1231; **Phase 1.5 (data-index measure race) is the dominant residual cause — §3.4 / §4.4**, fix verified live
**Date:** 2026-06-01
**Author:** AgentA
**Tracking:** open

> **Update 2026-06-01 (post-#1231):** Phase 1 shipped and reduced — but did **not
> eliminate** — the overlap ("still overlaps when zoomed"). Live CDP inspection of
> the *running* virtualizer (not a synthetic probe) found the **dominant** cause
> was orthogonal to zoom: a **`data-index` / `measureElement` ref race** that
> leaves a subset of rows permanently stuck at their `estimateSize`, overlapping
> at **any** zoom. See **§3.4** (root cause, empirically isolated) and **§4.4**
> (fix). Phase 1's zoom normalization is still correct and necessary — it makes
> the rows that *do* measure land in the right unit; §3.4 ensures all rows
> actually measure.

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

### 3.2 SECONDARY — re-measure on collapse / expand / pin — **NOT a bug (corrected)**

> **Correction (2026-06-01, verified against installed `@tanstack/virtual-core`
> 3.14.0):** an earlier draft claimed rows are "measured once at first paint,
> never re-measured on height change." **That is wrong for this version.** The
> `ref={virtualizer.measureElement}` on each row (line 418) wires TanStack's
> built-in **`ResizeObserver`** (`this.observer.observe(node)` in
> `measureElement`, virtual-core L633; the observer callback re-measures via
> `this.options.measureElement(...)`, L220 → `resizeItem` updates the cache and
> corrects scroll position). So **any** row whose height changes — collapse,
> expand, the canceled-thinking local `expanded()` toggle, async markdown /
> syntax-highlight settle — is re-measured automatically, and the measured size
> is cached per item key so it survives scroll-recycle. After **Phase 1** those
> re-measures route through the zoom-normalized `measureElement`, so they are
> correct at every zoom.
>
> Net: there is **no separate "collapse/expand overlap" to fix** at the
> virtualizer layer — the library already handles it and Phase 1 made it
> zoom-correct. Building a manual per-row `ResizeObserver` + a
> `virtualizer.measure()` `createEffect` (the original Phase 2) would
> **double-observe** rows and force full-list re-measures where the library does
> surgical per-row ones — net negative. **Phase 2 is therefore dropped** unless
> empirical testing surfaces a *specific* residual gap (see §7), in which case
> the fix targets that gap, not this speculative design.

The only residual TanStack does NOT auto-correct is a transient: a row that
resizes *during* an active fast scroll is gated by `shouldMeasureDuringScroll`
and settles a frame later — self-correcting, not a persistent overlap.

### 3.3 TERTIARY — thinking-block height under-estimate

`estimateMarkdown` (`renderers.ts`) is a char-count heuristic; thinking content
with markdown/code/lists renders much taller than `chars/80 × 24px` predicts.
On its own this causes a one-time settle jump (acceptable once 3.1/3.2 are
fixed), not persistent overlap — but it makes the transient worse and is worth a
modest correction.

### 3.4 DOMINANT (post-#1231) — `data-index` is set AFTER the `measureElement` ref fires

After Phase 1 the user still saw overlap. Live CDP inspection of the running
virtualizer at zoom 0.63 (not a synthetic probe) found, repeatedly and
**persistently**, that a *subset* of on-screen rows held their `estimateSize`
fallback (**32 px**) as their virtualizer size while rendering at a completely
different real height — e.g.:

| row | virtualizer size (`translateY` Δ) | real `offsetHeight` | result |
|---|---|---|---|
| measured rows (0,1,2,4,5,7…) | = `offsetHeight` ✓ | — | flush (Phase 1 working) |
| 3 | **32** (estimate) | 23 | +5.6px gap |
| 6 | **32** (estimate) | 46 | **−9.2px OVERLAP** |
| 8, 9, 10 | **32** (estimate) | 23/24 | gaps/overlap |

Two diagnostics isolated it:
1. A **scroll nudge** (±1px) did **not** fix the stuck rows → not a transient
   mid-scroll skip.
2. A forced **container width change** — which fires the `ResizeObserver` on
   every *observed* row — **also** did not re-measure them. That is the tell:
   **the stuck rows were never being observed at all.**

**Mechanism.** TanStack's `measureElement(node)` reads
`node.getAttribute("data-index")` **first**, and if it is `null` it
**returns early — before `this.observer.observe(node)`**. So a row whose
`data-index` is absent at the instant the ref fires is *never* registered with
the `ResizeObserver` and is *never* re-measured for the rest of its life; it is
frozen at `estimateSize`.

In this codebase `data-index` is bound **reactively** — `data-index={props.dataIndex}`
on the row wrapper (`DocumentRow.tsx`), fed by `dataIndex={virtualItem.index}`.
SolidJS sets reactive attributes in a **render-effect that races the `ref`
callback**. When the ref (`virtualizer.measureElement`) wins, `data-index` is
still `null` → early return → row stranded at the estimate. It's a **race**,
which is exactly why only *some* rows are affected, and why it's **stable**
(a stranded row never gets a second chance until scroll-recycle recreates it).

This is **orthogonal to zoom**: the estimate (32) vs real height mismatch
produces overlap/gaps at *every* zoom including 100%. Phase 1's symptom framing
("largely absent at 100%") was the *zoom double-count* component; the data-index
race is a *second, independent* contributor that Phase 1 didn't touch. With both
fixed, a 50-row virtualized region at zoom 0.63 shows **0 stuck rows** (verified
live).

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

### 4.4 Set `data-index` synchronously in the ref (fixes 3.4) — **Phase 1.5**

Eliminate the race by writing `data-index` **inside the ref callback, on the line
before** `measureElement` reads it — so it is never `null` at measure time and
the row is always observed:

```ts
// AgentDocumentVirtualList.tsx — the <For> over getVirtualItems()
ref={(el) => {
    el.setAttribute("data-index", String(virtualItem.index));
    virtualizer.measureElement(el);
}}
dataIndex={virtualItem.index}   // reactive binding retained for index shifts
```

The reactive `dataIndex` binding is kept (it keeps the attribute fresh if a row's
index shifts), but the synchronous set in the ref wins the initial race
deterministically. `virtualItem.index` is fixed for a row's lifetime (TanStack
recreates the row object on window change, so the `<For>` remounts rather than
mutating index), so the value written is always correct.

**Verification of record (CDP, live):** before — rows 3/6/8/9/10 stuck at the
32px estimate at zoom 0.63; a forced width-change did not rescue them (never
observed). After — a 50-row virtualized region at the same zoom reports **0
stuck rows**; the `−9.2px` overlap at row 6 is gone. Re-run the live probe (not
jsdom — Solid ref/attribute ordering and the `ResizeObserver` early-return can't
be exercised there) after a TanStack or Solid bump.

### 4.3 Estimator correction (mitigates 3.3) — optional

Add a modest buffer for thinking/markdown blocks (e.g. account for the
`.thinking-block` chrome and a code-block likelihood), or — once 4.2's
ResizeObserver lands — accept the heuristic and let measurement settle it. Low
priority; do only if jank-on-scroll-in remains visible after 4.1/4.2.

---

## 5. Edge cases

- **Zoom mid-scroll:** no special handling needed (§4.1). A value cached at one
  zoom (`GBCR/Z₁ = h` CSS px) stays correct at any other zoom, and the browser
  re-applies the new `zoom` to the whole subtree (offsets *and* heights), so a
  zoom change while scrolled deep does not strand stale offsets.
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

- **Phase 1 (zoom double-count) — MERGED #1231:** §4.1 zoom-normalized
  `measureElement` + plumb `zoomFactor` down. Removes the *zoom-correlated*
  component of the overlap. Necessary but **not sufficient** — see Phase 1.5.
- **Phase 1.5 (data-index measure race) — THE residual fix, §4.4:** set
  `data-index` synchronously in the row ref before `measureElement`. Without it,
  a subset of rows are never observed and stay stuck at `estimateSize`,
  overlapping at any zoom (§3.4). This is what was still visible after #1231.
  Verified live (0 stuck rows post-fix).
- **Phase 2 — DROPPED (see §3.2 correction).** TanStack 3.14's built-in
  per-row `ResizeObserver` already re-measures on any height change, and Phase 1
  made those re-measures zoom-correct; a manual observer/`measure()` would be
  redundant and net-negative. Reinstate ONLY if testing surfaces a *specific*
  residual overlap that the built-in observer demonstrably misses — and then fix
  that gap, not the original speculative design.
- **Phase 3 (polish):** §4.3 estimator correction, only if scroll-in jank
  persists. Independent of Phase 2.

---

## 8. Files

| File | Change |
|---|---|
| `frontend/app/view/agent/virtualization/AgentDocumentVirtualList.tsx` | **(Phase 1)** `measureElement` ÷ `zoomFactor`; new `zoomFactor` prop. **(Phase 1.5)** set `data-index` synchronously in the row `ref` before `measureElement` (fixes the never-observed-stuck-at-estimate race, §3.4/§4.4) |
| `frontend/app/view/agent/components/AgentDocumentView.tsx` | pass `zoomFactor` through; (Phase 2) optional `expandedNodes` in `documentState` |
| `frontend/app/view/agent/agent-view.tsx` | pass `zoomFactor` (line 640) into `AgentDocumentView` |
| `frontend/app/view/agent/components/MarkdownBlock.tsx` | (Phase 2) route canceled-thinking `expanded` into `documentState`, or rely on the row ResizeObserver |
| `frontend/app/view/agent/virtualization/renderers.ts` | (Phase 3, optional) thinking/markdown estimate buffer; perf-probe records CSS-px measured |

---

## 9. Risks

- **Re-measure storms (Phase 2):** the Phase 2 `createEffect` calling
  `virtualizer.measure()` on every `documentState` write could thrash during
  streaming. Scope it to the specific size-affecting sets (`collapsedNodes`,
  `pinnedNodes`) — NOT the whole document, and NOT `zoomFactor` (zoom needs no
  re-measure per §4.1) — so it fires only on genuine toggles, not on every
  token. (Cross-ref the input-responsiveness rules: no layout reads in the
  keystroke path; this effect is off the input path.)
- **ResizeObserver cost** (Phase 2): bounded to on-screen rows; disconnect on row
  unmount. Don't observe the whole document.
- **zoomFactor source drift:** the virtual list MUST use the exact same value
  that drives the CSS `zoom` (the `agent-view.tsx` memo), or the division won't
  cancel. Single source of truth via the prop; no independent re-derivation.
