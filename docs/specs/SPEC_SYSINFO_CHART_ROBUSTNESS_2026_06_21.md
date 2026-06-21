# Spec: Sysinfo CPU Chart Robustness

**Date:** 2026-06-21  
**Status:** Ready for implementation  
**Files in scope:**
- `frontend/app/view/sysinfo/sysinfo-model.ts`
- `frontend/app/view/sysinfo/sysinfo-view.tsx`
- `frontend/app/view/sysinfo/sysinfo-plot.tsx`

---

## Bug inventory

### Bug 1 — Empty chart during heavy load

**Symptom:** When the remote/local machine is under 100% CPU load, sysinfo samples arrive
late or skip a tick. The chart goes blank instead of holding its last state.

**Root cause:** `sysinfo-view.tsx:53-54`:

```ts
if (dataItem.ts - prevLastTs > gapThreshold) {
    model.loadInitialData();   // ← sets loadingAtom = true
```

`gapThreshold` is `max(3000, interval * 2.5)`. A single dropped sample at 1 Hz
(1000 ms) exceeds this threshold (3000 ms) only when the backend is very stalled,
but at 2 s interval it takes only one missed tick. When `loadInitialData()` fires,
`loadingAtom` becomes `true` and the view hides everything:

```tsx
<Show when={connStatus()?.status == "connected" && !loading()}>
```

The chart goes blank for the entire async load cycle (~300 ms to a few seconds).
During the load, incoming samples are **silently dropped** (`if (model.loadingAtom()) return`),
creating another gap that may trigger a second reload — a feedback loop during sustained load.

---

### Bug 2 — Dock/undock chart distortion

**Symptom:** When the sysinfo pane is docked, undocked, or resized, a horizontal artifact
(a line that slides across the full chart width) appears momentarily before the chart
settles.

**Root cause:** `sysinfo-plot.tsx:32-39`:

```ts
const rszObs = new ResizeObserver((entries) => {
    for (const entry of entries) {
        setPlotWidth(entry.contentRect.width);   // fires every animation frame
        setPlotHeight(entry.contentRect.height);
    }
});
```

ResizeObserver fires on every intermediate size during a dock/undock transition
(multiple times per frame in CEF). Each callback updates the `plotWidth` / `plotHeight`
signals, which triggers the `createEffect` that destroys and recreates the entire SVG.

Two consequences:
1. **Multiple concurrent renders.** The effect is synchronous but the resize fires faster
   than SolidJS batches. Multiple in-flight renders momentarily stack the old and new SVG
   siblings in the DOM before the cleanup runs.
2. **Gradient ID collision.** The gradient `id="gradient-${blockId}-${yval}"` lives inside
   the SVG's `<defs>`. When two SVGs with the same gradient ID coexist for a frame, the
   area fill (`url(#gradient-...)`) may resolve to the wrong (already-removed) gradient,
   producing a black fill or a sliding artifact.

---

### Bug 3 — Small green dots / no fill after heavy CPU run

**Symptom:** After a sustained 100% CPU run ends, the chart shows only tiny green dots
at each sample position with no line or fill between them.

**Root cause:** Multiple compounding issues:

1. During the 100% run, Bug 1's reload loop fires repeatedly. Each `loadInitialData()`
   sets `loadingAtom = true`, drops incoming samples, and fetches history. The history
   from a remote machine under 100% load may itself contain many missed ticks.

2. `addInitialData()` gap-fills missed ticks with NaN items:
   ```ts
   const blankItemStart = { ...blankItemTemplate, ts: prevIdxItem.ts + 1, blank: 1 };
   const blankItemEnd   = { ...blankItemTemplate, ts: curIdxItem.ts - 1, blank: 1 };
   ```
   When a 120-sample window contains many NaN entries, Observable Plot's `areaY` and
   `lineY` break the continuous path at each NaN, producing dozens of disconnected
   micro-segments.

3. `addContinuousData()` does **not** insert NaN gap markers for gaps it detects —
   it just pushes the point directly. So the initial historical data has proper gap
   markers but subsequent streaming data does not. After a reload, the first
   `addContinuousData()` call after the load ends appends a new point far in the future
   relative to the historical tail, leaving a large unmarked time gap where Observable
   Plot draws a single line crossing the entire X domain. This is the "line sliding
   across the whole chart" variant of Bug 3.

4. After the load ends and the final `loadInitialData()` fetches a clean buffer, the
   120-sample window is correct — but if the loaded history itself was sparse (many
   NaN entries from the heavy period), the chart renders as disconnected micro-segments
   that look like dots.

---

## Redesign

### Principle: hold-last-sample instead of NaN gaps

The core architectural fix is to **never render NaN** in the data buffer. When a sample
is late or missing, extend the last known value to the current time. This is the
"zero-order hold" (ZOH) pattern used by every professional telemetry chart (Grafana,
Datadog, AWS CloudWatch all default to "fill: last").

ZOH properties:
- Chart never goes blank during intermittent data loss
- No disconnected micro-segments after a heavy period
- The visual accurately communicates "we last saw X at time T" rather than implying
  the metric is undefined

A NaN gap is still correct for a **true signal break** — when the remote machine is
disconnected (connection lost, agent restarted). For that case we reload. For a single
missed tick under load, we hold.

---

### Change 1 — Reducer for sample buffer (`sysinfo-model.ts`)

Replace the mutable `data.push()` / `data.filter()` pattern with a reducer.
The reducer is a pure function: `(state: DataItem[], action: SampleAction) => DataItem[]`.

```ts
type SampleAction =
  | { type: 'RESET';  items: DataItem[] }
  | { type: 'APPEND'; item: DataItem; intervalSecs: number; numPoints: number }
  | { type: 'TRIM';   cutoffTs: number };

function sampleReducer(state: DataItem[], action: SampleAction): DataItem[] {
  switch (action.type) {
    case 'RESET':
      return buildInitialBuffer(action.items);   // gap-fill with NaN only at boundaries

    case 'APPEND': {
      const last = state[state.length - 1];
      const gap  = last ? action.item.ts - last.ts : 0;
      const maxGap = action.intervalSecs * 1000 * 3;  // >3 missed ticks = true break

      let next = state;
      if (last && gap > action.intervalSecs * 1000 * 1.5 && gap <= maxGap) {
        // 1–3 missed ticks: fill with hold-last values at nominal interval
        const steps = Math.round(gap / (action.intervalSecs * 1000)) - 1;
        for (let i = 1; i <= steps; i++) {
          next = [...next, { ...last, ts: last.ts + i * action.intervalSecs * 1000 }];
        }
      } else if (last && gap > maxGap) {
        // True break: insert NaN sentinels (connection hiccup or long pause)
        next = [...next, { ...last, ts: last.ts + 1, blank: 1 } as DataItem];
      }
      return [...next, action.item].filter(d => d.ts >= action.item.ts - action.intervalSecs * 1000 * action.numPoints);
    }

    case 'TRIM':
      return state.filter(d => d.ts >= action.cutoffTs);
  }
}
```

Replace `dataAtom._set(...)` calls with `dispatch(action)` through a
`createReducer`-style wrapper (or a plain signal + setter with the reducer inline):

```ts
const [data, setData] = createSignal<DataItem[]>([]);
const dispatch = (action: SampleAction) => setData(prev => sampleReducer(prev, action));
```

Expose `dispatch` from `SysinfoViewModel`. Remove `addInitialData` and
`addContinuousData` from the public API — replace them with:

```ts
resetData(items: DataItem[])  { dispatch({ type: 'RESET', items }) }
appendData(item: DataItem)    { dispatch({ type: 'APPEND', item, intervalSecs: ..., numPoints: ... }) }
```

---

### Change 2 — Remove the gap-triggered reload (`sysinfo-view.tsx`)

The gap check in the streaming handler is the root cause of Bug 1. Remove it entirely.
Use `APPEND` — which now handles gaps via ZOH or NaN sentinels — for every incoming sample:

```ts
// BEFORE
if (dataItem.ts - prevLastTs > gapThreshold) {
    model.loadInitialData();
} else {
    model.addContinuousData(dataItem);
}

// AFTER
model.appendData(dataItem);   // reducer decides ZOH vs sentinel
```

`loadInitialData()` is now called only on:
1. Component mount (initial hydration)
2. Connection name change (already handled by the `connName` effect)
3. Connection goes from disconnected → connected (already handled)

**Never** on a late/missing sample.

---

### Change 3 — Keep chart visible during initial load

Remove `!loading()` from the `<Show>` guard. Instead, keep showing the previous data
while a reload is in flight:

```tsx
// BEFORE
<Show when={connStatus()?.status == "connected" && !loading()}>
    <SysinfoViewInner ... />
</Show>

// AFTER — always render the inner component when connected, with stale data during load
<Show when={connStatus()?.status == "connected"}>
    <SysinfoViewInner ... />
</Show>
```

The inner component renders from `plotData()` which reflects `model.dataAtom()`.
During a reload, the atom still holds the previous data until `resetData()` replaces
it with fresh history. Users see stale data (not blank) during the brief reload.

If the atom is empty (first load, no history), the plot's `if (plotData.length === 0) return`
guard already produces a clean empty state — no spinner needed, just nothing until first
data arrives.

---

### Change 4 — Debounce the ResizeObserver (`sysinfo-plot.tsx`)

Add a 150 ms debounce to the resize handler. The dock/undock animation completes
well within 150 ms; the debounce means the chart only re-renders once at the settled
size, eliminating the intermediate SVG overlap that causes the gradient artifact.

```ts
onMount(() => {
    let timer: ReturnType<typeof setTimeout> | null = null;
    const rszObs = new ResizeObserver((entries) => {
        if (timer) clearTimeout(timer);
        timer = setTimeout(() => {
            for (const entry of entries) {
                setPlotWidth(entry.contentRect.width);
                setPlotHeight(entry.contentRect.height);
            }
        }, 150);
    });
    rszObs.observe(containerRef);
    onCleanup(() => {
        if (timer) clearTimeout(timer);
        rszObs.disconnect();
    });
});
```

---

### Change 5 — Fix `addContinuousData` gap (minor, precondition for reducer)

Currently `addContinuousData` never inserts NaN markers for gaps it detects — only
`addInitialData` does. This asymmetry is eliminated by the reducer (Change 1), which
handles both paths uniformly through `APPEND`.

---

## What the reducer does NOT change

- **Throttle (`CHART_UPDATE_INTERVAL_MS = 2000`)** — retained as-is. The SVG rebuild
  cost comment is still accurate; the ZOH fill adds negligible CPU.
- **Observable Plot** — no library change. The fix is in the data passed to it.
- **`addInitialData` NaN boundary markers** — the RESET action can keep NaN sentinels
  for boundaries at the start of the historical window (before the first sample). These
  are real gaps (we have no data before the window started), not missed ticks.
- **`loadInitialData` itself** — still used on connection events. The async fetch is fine;
  we just don't blank the chart while it runs.

---

## Implementation order

1. **Change 4** (debounce resize) — 30 min, zero risk, fixes Bug 2 immediately.
2. **Changes 1 + 2 + 5** (reducer + remove gap-reload) — ~2 h, fixes Bug 1 and the
   streaming-gap half of Bug 3.
3. **Change 3** (no blank during load) — 10 min, fixes the remaining blank-during-reload
   scenario, completes Bug 3 fix.

Total estimated effort: **~3 hours**.

---

## Files changed summary

| File | Change |
|------|--------|
| `sysinfo-model.ts` | Replace `addInitialData`/`addContinuousData` with `sampleReducer` + `resetData`/`appendData` |
| `sysinfo-view.tsx` | Remove gap-triggered `loadInitialData()`; remove `!loading()` from `<Show>` guard |
| `sysinfo-plot.tsx` | Debounce `ResizeObserver` callback by 150 ms |

No backend changes. No schema changes. No new dependencies.
